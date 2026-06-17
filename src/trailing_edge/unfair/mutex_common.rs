/// Errors that can occur when interacting with the [`RateLimiter`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The internal mutex was poisoned.
    #[error("Mutex poisoned")]
    MutexPoisoned,
    /// A permit cannot currently be acquired because the limit has been reached.
    ///
    /// Contains the time when the next permit might become available, if known.
    #[error("A permit cannot currently be acquired")]
    NoPermitAvailable(Option<chrono::DateTime<chrono::Utc>>),
}

type ExpiryTimes = super::common::ExpiryTimes;
type State = super::common::State;

/// Storage strategy for mutex-backed rate limiter state.
pub trait StateStore: Sized {
    /// The state handle retained by permits created from this store.
    type PermitState<'a>: PermitState
    where
        Self: 'a;

    /// Builds a store from an initial state value.
    fn from_state(state: State) -> Self;

    /// Locks the limiter state.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MutexPoisoned`] if the internal state mutex is poisoned.
    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, State>, Error>;

    /// Returns the handle a permit should retain.
    fn permit_state(&self) -> Self::PermitState<'_>;
}

/// A handle retained by permits so they can update shared state on drop.
pub trait PermitState {
    /// Locks the permit state.
    fn lock_permit_state(&self) -> std::sync::MutexGuard<'_, State>;
}

/// A RAII permit for a single unit of concurrency.
///
/// When this value is dropped (returned), the permit is released, but the "slot" it occupied
/// remains unavailable for the configured `interval` of the rate limiter. This means the
/// cooldown period starts at the moment the permit is dropped, not when it was created.
#[derive(Debug)]
#[must_use]
pub struct SinglePermit<TState: PermitState> {
    state: TState,
}

impl<TState: PermitState> SinglePermit<TState> {
    fn new(state: TState) -> Self {
        Self { state }
    }

    pub(in crate::trailing_edge::unfair) fn drop_impl(&mut self, at_time: chrono::NaiveDateTime) {
        tracing::trace!("Dropping permit at {at_time}");
        let mut state = self.state.lock_permit_state();
        state.active_connection_count -= 1;
        state.expiry_times.push_back(at_time);
    }
}

impl<TState: PermitState> Drop for SinglePermit<TState> {
    fn drop(&mut self) {
        self.drop_impl(chrono::Utc::now().naive_utc());
    }
}

/// A RAII permit for multiple units of concurrency.
///
/// When this value is dropped (returned), the permits are released, but the "slots" they occupied
/// remain unavailable for the configured `interval` of the rate limiter. This means the
/// cooldown period starts at the moment the permits are dropped.
#[derive(Debug)]
#[must_use]
pub struct MultiPermit<TState: PermitState> {
    state: TState,
    num_permits: usize,
}

impl<TState: PermitState> MultiPermit<TState> {
    fn new(state: TState, num_permits: usize) -> Self {
        Self { state, num_permits }
    }

    pub(in crate::trailing_edge::unfair) fn drop_impl(&mut self, at_time: chrono::NaiveDateTime) {
        tracing::trace!(
            "Dropping {num_permits} permits at {at_time}",
            num_permits = self.num_permits
        );
        let mut state = self.state.lock_permit_state();

        state.active_connection_count -= self.num_permits;
        for _ in 0..self.num_permits {
            state.expiry_times.push_back(at_time);
        }
    }
}

impl<TState: PermitState> Drop for MultiPermit<TState> {
    fn drop(&mut self) {
        self.drop_impl(chrono::Utc::now().naive_utc());
    }
}

/// A rate limiter that enforces a cooldown period after usage (return-time based).
///
/// `MAX_SIMULTANEOUS` defines the maximum number of "slots" available.
/// A slot is occupied if a permit is currently held, OR if a permit was recently
/// held and the cooldown `interval` has not yet passed since it was dropped.
///
/// This implies that long-running tasks holding a permit will delay the availability
/// of that slot for future tasks until `duration_held + interval` time has passed.
#[derive(Debug)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "sibling implementation modules test shared state without exposing fields publicly"
)]
pub struct RateLimiter<const MAX_SIMULTANEOUS: usize, TState> {
    pub(in crate::trailing_edge::unfair) interval: chrono::Duration,
    pub(in crate::trailing_edge::unfair) state: TState,
}

impl<const MAX_SIMULTANEOUS: usize, TState> RateLimiter<MAX_SIMULTANEOUS, TState>
where
    TState: StateStore,
{
    /// Creates a new rate limiter with the specified cooldown `interval`.
    ///
    /// The `interval` specifies how long a slot remains unavailable *after* a permit is dropped.
    /// Initially, all permits are available.
    #[must_use]
    pub fn new(interval: chrono::Duration) -> Self {
        Self {
            interval,
            state: TState::from_state(State::default()),
        }
    }

    pub(in crate::trailing_edge::unfair) fn new_exhausted_impl(
        interval: chrono::Duration,
        start_time: chrono::NaiveDateTime,
    ) -> Self {
        Self {
            interval,
            state: TState::from_state(State {
                expiry_times: std::collections::VecDeque::from([start_time; MAX_SIMULTANEOUS]),
                ..Default::default()
            }),
        }
    }

    /// Creates a new rate limiter that is initially exhausted.
    ///
    /// This simulates a state where all permits have just been used and dropped
    /// at the current time, so no new permits can be acquired until the `interval` cooldown has passed.
    #[must_use]
    pub fn new_exhausted(interval: chrono::Duration) -> Self {
        let start_time = chrono::Utc::now().naive_utc();
        Self::new_exhausted_impl(interval, start_time)
    }

    fn remove_old_expiries(
        expiry_times: &mut ExpiryTimes,
        for_time: &chrono::NaiveDateTime,
        interval: &chrono::Duration,
    ) -> usize {
        let partition_point = expiry_times.partition_point(|time| *time < (*for_time - *interval));
        for _ in 0..partition_point {
            let _: Option<chrono::NaiveDateTime> = expiry_times.pop_front();
        }
        partition_point
    }

    pub(in crate::trailing_edge::unfair) fn try_acquire_permit_impl(
        &self,
        for_time: &chrono::NaiveDateTime,
    ) -> Result<SinglePermit<TState::PermitState<'_>>, Error> {
        tracing::debug!("Trying to acquire permit for {for_time}");
        let mut state = self.state.lock_state()?;
        tracing::trace!("{for_time} - lock acquired");

        debug_assert!(state.active_connection_count <= MAX_SIMULTANEOUS);
        let next_available_time = state
            .expiry_times
            .front()
            .map(|time| time.and_utc() + self.interval);
        if state.active_connection_count == MAX_SIMULTANEOUS {
            tracing::trace!("{for_time} - No permit available, all connections in use");
            return Err(Error::NoPermitAvailable(next_available_time));
        }

        Self::remove_old_expiries(&mut state.expiry_times, for_time, &self.interval);

        debug_assert!(state.expiry_times.len() <= MAX_SIMULTANEOUS);
        if state.expiry_times.len() == MAX_SIMULTANEOUS {
            tracing::trace!("{for_time} - No permit available, at rate limit");
            return Err(Error::NoPermitAvailable(next_available_time));
        }

        debug_assert!(state.expiry_times.len() + state.active_connection_count <= MAX_SIMULTANEOUS);
        if state.active_connection_count + state.expiry_times.len() == MAX_SIMULTANEOUS {
            tracing::trace!("{for_time} - No permit available, rate limit reached");
            return Err(Error::NoPermitAvailable(next_available_time));
        }

        state.active_connection_count += 1;
        let permit_state = self.state.permit_state();
        drop(state);

        Ok(SinglePermit::new(permit_state))
    }

    pub(in crate::trailing_edge::unfair) fn try_acquire_permits_impl(
        &self,
        for_time: &chrono::NaiveDateTime,
        num_permits: usize,
    ) -> Result<MultiPermit<TState::PermitState<'_>>, Error> {
        debug_assert!(num_permits > 0);
        debug_assert!(num_permits <= MAX_SIMULTANEOUS);

        tracing::debug!("Trying to acquire {num_permits} permits for {for_time}");
        let mut state = self.state.lock_state()?;
        tracing::trace!("{for_time} - lock acquired");

        Self::remove_old_expiries(&mut state.expiry_times, for_time, &self.interval);

        debug_assert!(state.active_connection_count <= MAX_SIMULTANEOUS);

        if state.active_connection_count + num_permits > MAX_SIMULTANEOUS {
            tracing::trace!(
                concat!(
                    "{for_time} - Not enough permits available, {num_conn} ",
                    "connections in use ({num_permits} requested)",
                ),
                for_time = for_time,
                num_conn = state.active_connection_count,
                num_permits = num_permits,
            );
            return Err(Error::NoPermitAvailable(None));
        }

        let num_expired = state.expiry_times.len();
        if state.active_connection_count + num_expired + num_permits > MAX_SIMULTANEOUS {
            let available = MAX_SIMULTANEOUS - state.active_connection_count - num_expired;
            tracing::trace!(
                "{for_time} - Not enough permits available. {num_permits} requested, {available} available"
            );
            let next_time = state
                .expiry_times
                .get(num_permits - 1)
                .or(state.expiry_times.back())
                .map(|time| time.and_utc() + self.interval);

            return Err(Error::NoPermitAvailable(next_time));
        }

        state.active_connection_count += num_permits;
        let permit_state = self.state.permit_state();
        drop(state);

        Ok(MultiPermit::new(permit_state, num_permits))
    }
}

impl<'a, const MAX_SIMULTANEOUS: usize, TState> crate::RateLimiter
    for &'a RateLimiter<MAX_SIMULTANEOUS, TState>
where
    TState: StateStore,
{
    type SinglePermit = SinglePermit<TState::PermitState<'a>>;
    type MultiPermit = MultiPermit<TState::PermitState<'a>>;
    type Error = Error;

    /// Attempts to acquire a single permit.
    ///
    /// Returns a [`SinglePermit`] if a slot is available.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoPermitAvailable`] if all slots are occupied (either by active permits or by cooldowns from recently dropped permits).
    /// Returns [`Error::MutexPoisoned`] if the internal state mutex is poisoned.
    fn try_acquire_permit(self) -> Result<Self::SinglePermit, Self::Error> {
        self.try_acquire_permit_impl(&chrono::Utc::now().naive_utc())
    }

    /// Attempts to acquire multiple permits at once.
    ///
    /// Returns a [`MultiPermit`] if enough slots are available.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoPermitAvailable`] if there are insufficient slots.
    /// Returns [`Error::MutexPoisoned`] if the internal state mutex is poisoned.
    fn try_acquire_permits(self, num_permits: usize) -> Result<Self::MultiPermit, Self::Error> {
        self.try_acquire_permits_impl(&chrono::Utc::now().naive_utc(), num_permits)
    }
}

#[cfg(feature = "tokio")]
#[expect(
    clippy::multiple_inherent_impl,
    reason = "this one is just for when tokio is enabled"
)]
impl<const MAX_SIMULTANEOUS: usize, TState> RateLimiter<MAX_SIMULTANEOUS, TState>
where
    TState: StateStore,
{
    async fn retry_until_acquired<TReturn>(
        &self,
        mut acquire_fn: impl FnMut() -> Result<TReturn, Error> + Send,
    ) -> TReturn
    where
        TReturn: Send,
    {
        loop {
            let result = acquire_fn();
            let next_time = match result {
                Ok(permit) => return permit,
                Err(Error::NoPermitAvailable(next_time)) => next_time,
                #[expect(clippy::panic, reason = "mutex should never be poisoned")]
                Err(Error::MutexPoisoned) => panic!("Internal mutex is poisoned"),
            };
            let wait_time =
                next_time.map_or(self.interval, |wake_time| wake_time - chrono::Utc::now());
            tokio::time::sleep(
                wait_time
                    .to_std()
                    // The wake time was in the past when the now calculation
                    // was made, so just re-wake immediately
                    .unwrap_or(std::time::Duration::from_secs(0)),
            )
            .await;
        }
    }
}

#[cfg(feature = "tokio")]
impl<'a, const MAX_SIMULTANEOUS: usize, TState> crate::AsyncRateLimiter
    for &'a RateLimiter<MAX_SIMULTANEOUS, TState>
where
    TState: StateStore + Sync,
    TState::PermitState<'a>: Send,
{
    /// Asynchronously acquires a single permit.
    ///
    /// Waits until a slot is available if the rate limit has been reached.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    fn acquire_permit(self) -> impl Future<Output = Self::SinglePermit> + Send {
        use crate::RateLimiter as _;

        self.retry_until_acquired(move || self.try_acquire_permit())
    }

    /// Asynchronously acquires multiple permits.
    ///
    /// Waits until enough slots are available if the rate limit has been reached.
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    fn acquire_permits(self, num_permits: usize) -> impl Future<Output = Self::MultiPermit> + Send {
        use crate::RateLimiter as _;

        self.retry_until_acquired(move || self.try_acquire_permits(num_permits))
    }
}
