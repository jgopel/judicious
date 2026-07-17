/// Errors that can occur when interacting with the [`RateLimiter`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A permit cannot currently be acquired because the limit has been reached.
    ///
    /// Contains the time when the next permit might become available, if known.
    #[error("A permit cannot currently be acquired")]
    NoPermitAvailable(Option<chrono::DateTime<chrono::Utc>>),
}

type State = super::common::State;

type Sender = std::sync::mpsc::Sender<chrono::NaiveDateTime>;

/// A single RAII permit.
///
/// When this value is dropped, the permit is released and the drop time is pushed to a queue so
/// that it can be made available again after the appropriate interval.
#[derive(Debug)]
#[must_use]
pub struct SinglePermit {
    sender: Sender,
}

impl SinglePermit {
    fn new(sender: Sender) -> Self {
        Self { sender }
    }

    fn drop_impl(&mut self, at_time: chrono::NaiveDateTime) {
        tracing::trace!("Dropping permit at {at_time}");
        self.sender
            .send(at_time)
            .expect("Sender should not be disconnected");
    }
}

#[expect(clippy::missing_trait_methods, reason = "Bug in clippy 1.97.0")]
impl Drop for SinglePermit {
    fn drop(&mut self) {
        self.drop_impl(chrono::Utc::now().naive_utc());
    }
}

/// A permit for multiple actions.
///
/// When this value is dropped, the permit is released and the drop time is pushed to a queue so
/// that it can be made available again after the appropriate interval.
#[derive(Debug)]
#[must_use]
pub struct MultiPermit {
    sender: Sender,
    num_permits: usize,
}

impl MultiPermit {
    fn new(sender: Sender, num_permits: usize) -> Self {
        Self {
            sender,
            num_permits,
        }
    }

    fn drop_impl(&mut self, at_time: chrono::NaiveDateTime) {
        tracing::trace!(
            "Dropping {num_permits} permits at {at_time}",
            num_permits = self.num_permits
        );

        for _ in 0..self.num_permits {
            self.sender
                .send(at_time)
                .expect("Sender should never be disconnected");
        }
    }
}

#[expect(clippy::missing_trait_methods, reason = "Bug in clippy 1.97.0")]
impl Drop for MultiPermit {
    fn drop(&mut self) {
        self.drop_impl(chrono::Utc::now().naive_utc());
    }
}

/// Rate limiter that enforces a cooldown period after the action finishes.
///
/// The number of available slots is `MAX_SIMULTANEOUS`. If a slot is occupied or was occupied within
/// `interval`, it is not available.
#[derive(Debug)]
pub struct RateLimiter<const MAX_SIMULTANEOUS: usize> {
    interval: chrono::Duration,
    state: State,
    sender: Sender,
    receiver: std::sync::mpsc::Receiver<chrono::NaiveDateTime>,
}

impl<const MAX_SIMULTANEOUS: usize> RateLimiter<MAX_SIMULTANEOUS> {
    #[must_use]
    fn new_impl(state: State, interval: chrono::Duration) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self {
            interval,
            state,
            sender,
            receiver,
        }
    }

    /// Create a new rate limiter with all slots available.
    ///
    /// If you want to create a new rate limiter with no slots available, use `Self::new_exhausted`.
    #[must_use]
    pub fn new(interval: chrono::Duration) -> Self {
        Self::new_impl(State::default(), interval)
    }

    #[must_use]
    fn new_exhausted_impl(interval: chrono::Duration, start_time: chrono::NaiveDateTime) -> Self {
        let state = State {
            expiry_times: std::collections::VecDeque::from([start_time; MAX_SIMULTANEOUS]),
            ..Default::default()
        };
        Self::new_impl(state, interval)
    }

    /// Create a new rate limiter with all slots exhausted.
    ///
    /// The rate limiter will become available again after the `interval` has passed
    /// from the current time.
    #[must_use]
    pub fn new_exhausted(interval: chrono::Duration) -> Self {
        let start_time = chrono::Utc::now().naive_utc();
        Self::new_exhausted_impl(interval, start_time)
    }

    fn drain_returned_permits(&mut self) {
        let initial_count = self.state.expiry_times.len();
        self.state.expiry_times.extend(self.receiver.try_iter());
        let final_count = self.state.expiry_times.len();
        let num_elements_added = final_count - initial_count;
        self.state.active_connection_count -= num_elements_added;
    }

    fn remove_old_expiries(&mut self, for_time: &chrono::NaiveDateTime) -> usize {
        let partition_point = self
            .state
            .expiry_times
            .partition_point(|time| *time < (*for_time - self.interval));
        for _ in 0..partition_point {
            let _: Option<chrono::NaiveDateTime> = self.state.expiry_times.pop_front();
        }
        partition_point
    }

    fn try_acquire_permit_impl(
        &mut self,
        for_time: &chrono::NaiveDateTime,
    ) -> Result<SinglePermit, Error> {
        tracing::debug!("Trying to acquire permit for {for_time}");

        self.drain_returned_permits();
        self.remove_old_expiries(for_time);

        debug_assert!(self.state.active_connection_count <= MAX_SIMULTANEOUS);
        let next_available_time = self
            .state
            .expiry_times
            .front()
            .map(|time| time.and_utc() + self.interval);
        if self.state.active_connection_count == MAX_SIMULTANEOUS {
            tracing::trace!("{for_time} - No permit available, all connections in use");
            return Err(Error::NoPermitAvailable(next_available_time));
        }

        debug_assert!(self.state.expiry_times.len() <= MAX_SIMULTANEOUS);
        if self.state.expiry_times.len() == MAX_SIMULTANEOUS {
            tracing::trace!("{for_time} - No permit available, at rate limit");
            return Err(Error::NoPermitAvailable(next_available_time));
        }

        debug_assert!(
            self.state.expiry_times.len() + self.state.active_connection_count <= MAX_SIMULTANEOUS
        );
        if self.state.active_connection_count + self.state.expiry_times.len() == MAX_SIMULTANEOUS {
            tracing::trace!("{for_time} - No permit available, rate limit reached");
            return Err(Error::NoPermitAvailable(next_available_time));
        }

        self.state.active_connection_count += 1;
        Ok(SinglePermit::new(self.sender.clone()))
    }

    fn try_acquire_permits_impl(
        &mut self,
        for_time: &chrono::NaiveDateTime,
        num_permits: usize,
    ) -> Result<MultiPermit, Error> {
        debug_assert!(num_permits > 0);
        debug_assert!(num_permits <= MAX_SIMULTANEOUS);

        tracing::debug!("Trying to acquire {num_permits} permits for {for_time}");

        self.drain_returned_permits();
        self.remove_old_expiries(for_time);

        debug_assert!(self.state.active_connection_count <= MAX_SIMULTANEOUS);

        if self.state.active_connection_count + num_permits > MAX_SIMULTANEOUS {
            tracing::trace!(
                concat!(
                    "{for_time} - Not enough permits available, {num_conn} ",
                    "connections in use ({num_permits} requested)",
                ),
                for_time = for_time,
                num_conn = self.state.active_connection_count,
                num_permits = num_permits,
            );
            return Err(Error::NoPermitAvailable(None));
        }

        let num_expired = self.state.expiry_times.len();
        if self.state.active_connection_count + num_expired + num_permits > MAX_SIMULTANEOUS {
            let available = MAX_SIMULTANEOUS - self.state.active_connection_count - num_expired;
            tracing::trace!(
                "{for_time} - Not enough permits available. {num_permits} requested, {available} available"
            );
            let next_time = self
                .state
                .expiry_times
                .get(num_permits - 1)
                .or(self.state.expiry_times.back())
                .map(|time| time.and_utc() + self.interval);

            return Err(Error::NoPermitAvailable(next_time));
        }

        self.state.active_connection_count += num_permits;
        Ok(MultiPermit::new(self.sender.clone(), num_permits))
    }
}

impl<const MAX_SIMULTANEOUS: usize> crate::RateLimiter for &mut RateLimiter<MAX_SIMULTANEOUS> {
    type SinglePermit = SinglePermit;
    type MultiPermit = MultiPermit;
    type Error = Error;

    fn try_acquire_permit(self) -> Result<Self::SinglePermit, Self::Error> {
        self.try_acquire_permit_impl(&chrono::Utc::now().naive_utc())
    }

    fn try_acquire_permits(self, num_permits: usize) -> Result<Self::MultiPermit, Self::Error> {
        self.try_acquire_permits_impl(&chrono::Utc::now().naive_utc(), num_permits)
    }
}

#[cfg(feature = "tokio")]
#[expect(
    clippy::multiple_inherent_impl,
    reason = "this one is just for when tokio is enabled"
)]
impl<const MAX_SIMULTANEOUS: usize> RateLimiter<MAX_SIMULTANEOUS> {
    async fn retry_until_acquired<TReturn>(
        &mut self,
        mut acquire_fn: impl FnMut(&mut Self) -> Result<TReturn, Error> + Send,
    ) -> TReturn {
        loop {
            let result = acquire_fn(self);
            let next_time = match result {
                Ok(permit) => return permit,
                Err(Error::NoPermitAvailable(next_time)) => next_time,
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
impl<const MAX_SIMULTANEOUS: usize> crate::AsyncRateLimiter for &mut RateLimiter<MAX_SIMULTANEOUS> {
    fn acquire_permit(self) -> impl Future<Output = Self::SinglePermit> + Send {
        self.retry_until_acquired(move |rate_limiter| {
            rate_limiter.try_acquire_permit_impl(&chrono::Utc::now().naive_utc())
        })
    }

    fn acquire_permits(self, num_permits: usize) -> impl Future<Output = Self::MultiPermit> + Send {
        self.retry_until_acquired(move |rate_limiter| {
            rate_limiter.try_acquire_permits_impl(&chrono::Utc::now().naive_utc(), num_permits)
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn dt_from_str(str: &str) -> chrono::NaiveDateTime {
        chrono::DateTime::parse_from_rfc3339(str)
            .unwrap()
            .naive_utc()
    }

    #[test]
    fn can_acquire_permit_immediately_after_normal_construction() {
        let mut rate_limiter = RateLimiter::<42>::new(chrono::Duration::seconds(43));

        let _permit = rate_limiter
            .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
            .unwrap();
    }

    #[test]
    fn cannot_acquire_permit_immediately_after_exhausted_construction() {
        let start_time = dt_from_str("2022-01-02 03:04:05Z");
        let interval = chrono::Duration::seconds(43);
        let mut rate_limiter = RateLimiter::<42>::new_exhausted_impl(interval, start_time);

        let result = rate_limiter.try_acquire_permit_impl(&start_time);

        let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
            panic!("Expected NoPermitAvailable error");
        };
        assert_eq!(
            next_permit_time,
            Some(dt_from_str("2022-01-02 03:04:48Z").and_utc())
        );
    }

    #[test]
    fn permit_can_be_held_while_owner_is_used_mutably() {
        use crate::RateLimiter as _;

        struct Writer {
            rate_limiter: RateLimiter<1>,
            messages_sent: usize,
        }

        impl Writer {
            fn send(&mut self, _permit: SinglePermit) {
                self.messages_sent += 1;
            }
        }

        let mut writer = Writer {
            rate_limiter: RateLimiter::new(chrono::Duration::milliseconds(100)),
            messages_sent: 0,
        };

        let permit = writer.rate_limiter.try_acquire_permit().unwrap();
        writer.send(permit);

        assert_eq!(writer.messages_sent, 1);
    }

    mod single_permit {
        use pretty_assertions::assert_eq;

        use super::*;

        #[test]
        fn can_acquire_permit_from_empty_rate_limiter() {
            let initial_state = State {
                active_connection_count: 0,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter
                .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 1,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn can_acquire_permit_when_exactly_1_connection_is_available() {
            const CONNECTION_COUNT: usize = 10;
            let initial_state = State {
                active_connection_count: CONNECTION_COUNT - 1,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CONNECTION_COUNT> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter
                .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: CONNECTION_COUNT,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn can_acquire_permit_when_expiry_times_is_nearly_full() {
            let current_time = dt_from_str("2022-01-02 03:04:05Z");
            let initial_expiry_times = std::collections::VecDeque::from([
                current_time,
                current_time,
                current_time,
                current_time,
            ]);
            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<5> {
                interval: chrono::Duration::seconds(5),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter.try_acquire_permit_impl(&current_time).unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 1,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn can_acquire_permit_from_full_expiries_after_interval_has_passed() {
            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:03:59Z"),
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:01Z"),
                dt_from_str("2022-01-02 03:04:02Z"),
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);
            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<6> {
                interval: chrono::Duration::seconds(5),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter
                .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 1,
                    expiry_times: initial_expiry_times.into_iter().skip(1).collect()
                }
            );
        }

        #[test]
        fn cannot_acquire_permit_when_all_connections_are_active() {
            const CONNECTION_COUNT: usize = 10;
            let initial_state = State {
                active_connection_count: CONNECTION_COUNT,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CONNECTION_COUNT> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let result = rate_limiter.try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"));

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(next_permit_time, None);

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: CONNECTION_COUNT,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn cannot_acquire_permit_when_previous_permits_are_not_expired() {
            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);
            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<2> {
                interval: chrono::Duration::seconds(5),
                state: initial_state,
                sender,
                receiver,
            };

            let result = rate_limiter.try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"));

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(
                next_permit_time,
                Some(dt_from_str("2022-01-02 03:04:08Z").and_utc())
            );

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 0,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn cannot_acquire_permit_when_all_expiry_times_are_exactly_at_current_time() {
            let current_time = dt_from_str("2022-01-02 03:04:05Z");
            let initial_expiry_times = std::collections::VecDeque::from([
                current_time,
                current_time,
                current_time,
                current_time,
                current_time,
            ]);
            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<5> {
                interval: chrono::Duration::seconds(5),
                state: initial_state,
                sender,
                receiver,
            };

            let result = rate_limiter.try_acquire_permit_impl(&current_time);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(
                next_permit_time,
                Some(dt_from_str("2022-01-02 03:04:10Z").and_utc())
            );

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 0,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn cannot_acquire_permit_when_all_expiry_times_are_about_to_be_retired() {
            let current_time = dt_from_str("2022-01-02 03:04:05Z");
            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:00Z"),
            ]);
            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<5> {
                interval: chrono::Duration::seconds(5),
                state: initial_state,
                sender,
                receiver,
            };

            let result = rate_limiter.try_acquire_permit_impl(&current_time);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(
                next_permit_time,
                Some(dt_from_str("2022-01-02 03:04:05Z").and_utc())
            );

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 0,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn cannot_acquire_permit_when_sum_of_active_connections_and_expired_connections_equals_max()
        {
            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);
            let initial_state = State {
                active_connection_count: 8,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<10> {
                interval: chrono::Duration::seconds(5),
                state: initial_state,
                sender,
                receiver,
            };

            let result = rate_limiter.try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"));

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(
                next_permit_time,
                Some(dt_from_str("2022-01-02 03:04:08Z").and_utc())
            );

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 8,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn dropping_permit_updates_the_state() {
            let initial_state = State {
                active_connection_count: 1,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let mut permit = rate_limiter
                .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 2,
                    expiry_times: std::collections::VecDeque::default()
                }
            );

            permit.drop_impl(dt_from_str("2022-01-02 03:04:06Z"));
            rate_limiter.drain_returned_permits();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 1,
                    expiry_times: std::collections::VecDeque::from([dt_from_str(
                        "2022-01-02 03:04:06Z"
                    )])
                }
            );
        }
    }

    mod multi_permit {
        use pretty_assertions::assert_eq;

        use super::*;

        #[test]
        fn can_acquire_5_permits_when_all_connections_are_available() {
            let initial_state = State {
                active_connection_count: 0,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5)
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 5,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn can_acquire_5_permits_when_exactly_5_connections_are_available() {
            const CAPACITY: usize = 42;
            const REQUESTED: usize = 5;

            let initial_state = State {
                active_connection_count: CAPACITY - REQUESTED,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), REQUESTED)
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: CAPACITY,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn can_acquire_5_permits_when_all_expiries_slots_are_empty() {
            let initial_state = State {
                active_connection_count: 0,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5)
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 5,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn can_acquire_5_permits_when_exactly_5_expiry_slots_are_available() {
            const CAPACITY: usize = 10;
            const REQUESTED: usize = 5;

            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:01Z"),
                dt_from_str("2022-01-02 03:04:02Z"),
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);

            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), REQUESTED)
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: REQUESTED,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn can_acquire_max_permits() {
            const CAPACITY: usize = 10;

            let initial_state = State {
                active_connection_count: 0,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), CAPACITY)
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: CAPACITY,
                    expiry_times: std::collections::VecDeque::default(),
                }
            );
        }

        #[test]
        fn cannot_acquire_5_permits_when_0_connections_are_available() {
            const CAPACITY: usize = 42;

            let initial_state = State {
                active_connection_count: CAPACITY,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(next_permit_time, None);

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: CAPACITY,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn cannot_acquire_5_permits_when_1_connection_is_available() {
            const CAPACITY: usize = 42;

            let initial_state = State {
                active_connection_count: CAPACITY - 1,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(next_permit_time, None);

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: CAPACITY - 1,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn cannot_acquire_5_permits_when_4_connections_are_available() {
            const CAPACITY: usize = 42;

            let initial_state = State {
                active_connection_count: CAPACITY - 4,
                expiry_times: std::collections::VecDeque::default(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(next_permit_time, None);

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: CAPACITY - 4,
                    expiry_times: std::collections::VecDeque::default()
                }
            );
        }

        #[test]
        fn cannot_acquire_5_permits_when_0_expiry_slots_are_available() {
            const CAPACITY: usize = 10;

            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:03:55Z"),
                dt_from_str("2022-01-02 03:03:56Z"),
                dt_from_str("2022-01-02 03:03:57Z"),
                dt_from_str("2022-01-02 03:03:58Z"),
                dt_from_str("2022-01-02 03:03:59Z"),
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:01Z"),
                dt_from_str("2022-01-02 03:04:02Z"),
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);

            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(15),
                state: initial_state,
                sender,
                receiver,
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(
                next_permit_time,
                Some(dt_from_str("2022-01-02 03:04:14Z").and_utc())
            );

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 0,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn cannot_acquire_5_permits_when_1_expiry_slot_is_available() {
            const CAPACITY: usize = 10;

            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:03:56Z"),
                dt_from_str("2022-01-02 03:03:57Z"),
                dt_from_str("2022-01-02 03:03:58Z"),
                dt_from_str("2022-01-02 03:03:59Z"),
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:01Z"),
                dt_from_str("2022-01-02 03:04:02Z"),
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);

            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(15),
                state: initial_state,
                sender,
                receiver,
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };

            assert_eq!(
                next_permit_time,
                Some(dt_from_str("2022-01-02 03:04:15Z").and_utc())
            );

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 0,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn cannot_acquire_5_permits_when_4_expiry_slots_are_available() {
            const CAPACITY: usize = 10;

            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:03:59Z"),
                dt_from_str("2022-01-02 03:04:00Z"),
                dt_from_str("2022-01-02 03:04:01Z"),
                dt_from_str("2022-01-02 03:04:02Z"),
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);

            let initial_state = State {
                active_connection_count: 0,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(15),
                state: initial_state,
                sender,
                receiver,
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };

            assert_eq!(
                next_permit_time,
                Some(dt_from_str("2022-01-02 03:04:18Z").and_utc())
            );

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 0,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn cannot_acquire_5_permits_when_sum_of_active_connections_and_expired_connections_is_too_close_to_max()
         {
            const CAPACITY: usize = 10;

            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:04:02Z"),
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);
            let initial_state = State {
                active_connection_count: 5,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(15),
                state: initial_state,
                sender,
                receiver,
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };

            assert_eq!(
                next_permit_time,
                Some(dt_from_str("2022-01-02 03:04:19Z").and_utc())
            );

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 5,
                    expiry_times: initial_expiry_times
                }
            );
        }

        #[test]
        fn dropping_permits_updates_the_state() {
            let initial_expiry_times = std::collections::VecDeque::from([
                dt_from_str("2022-01-02 03:04:02Z"),
                dt_from_str("2022-01-02 03:04:03Z"),
                dt_from_str("2022-01-02 03:04:04Z"),
            ]);
            let initial_state = State {
                active_connection_count: 5,
                expiry_times: initial_expiry_times.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: initial_state,
                sender,
                receiver,
            };
            let mut permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-01 12:00:00Z"), 3)
                .unwrap();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 8,
                    expiry_times: initial_expiry_times,
                }
            );

            permit.drop_impl(dt_from_str("2022-01-01 12:00:05Z"));
            rate_limiter.drain_returned_permits();

            assert_eq!(
                rate_limiter.state,
                State {
                    active_connection_count: 5,
                    expiry_times: std::collections::VecDeque::from([
                        dt_from_str("2022-01-02 03:04:02Z"),
                        dt_from_str("2022-01-02 03:04:03Z"),
                        dt_from_str("2022-01-02 03:04:04Z"),
                        dt_from_str("2022-01-01 12:00:05Z"),
                        dt_from_str("2022-01-01 12:00:05Z"),
                        dt_from_str("2022-01-01 12:00:05Z"),
                    ])
                }
            );
        }
    }

    #[cfg(feature = "tokio")]
    mod tokio_tests {
        use crate::AsyncRateLimiter as _;
        use crate::RateLimiter as _;

        use super::*;

        #[tokio::test]
        async fn permit_already_available() {
            let interval = chrono::Duration::milliseconds(100);
            let mut limiter = RateLimiter::<1>::new(interval);

            let start = chrono::Utc::now();
            let _permit = limiter.acquire_permit().await;
            let elapsed = chrono::Utc::now() - start;

            assert!(elapsed < interval);
        }

        #[tokio::test]
        async fn single_permit_cooldown() {
            let interval = chrono::Duration::milliseconds(100);
            let mut limiter = RateLimiter::<1>::new(interval);

            let previous_permit = limiter.try_acquire_permit().unwrap();
            drop(previous_permit);

            let start = chrono::Utc::now();
            let _permit = limiter.acquire_permit().await;
            let elapsed = chrono::Utc::now() - start;

            let cutoff = chrono::Duration::milliseconds(500);
            assert!(interval <= elapsed && elapsed < cutoff);
        }

        #[tokio::test]
        async fn multi_permit_cooldown() {
            let interval = chrono::Duration::milliseconds(100);
            let mut limiter = RateLimiter::<3>::new(interval);

            let previous_permit = limiter.try_acquire_permits(3).unwrap();
            drop(previous_permit);

            let start = chrono::Utc::now();
            let _permit = limiter.acquire_permits(3).await;
            let elapsed = chrono::Utc::now() - start;

            let cutoff = chrono::Duration::milliseconds(500);
            assert!(interval <= elapsed && elapsed < cutoff);
        }
    }
}
