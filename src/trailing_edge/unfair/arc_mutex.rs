impl super::mutex_common::PermitState for std::sync::Arc<std::sync::Mutex<State>> {
    fn lock_permit_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.lock().expect("This should never fail")
    }
}

impl super::mutex_common::StateStore for std::sync::Arc<std::sync::Mutex<State>> {
    type PermitState<'a> = std::sync::Arc<std::sync::Mutex<State>>;

    fn from_state(state: State) -> Self {
        Self::new(std::sync::Mutex::new(state))
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, State>, Error> {
        self.lock().map_err(|_error| Error::MutexPoisoned)
    }

    fn permit_state(&self) -> Self::PermitState<'_> {
        Self::clone(self)
    }
}

/// Errors that can occur when interacting with the [`RateLimiter`].
pub type Error = super::mutex_common::Error;

/// Internal state of the rate limiter.
pub type State = super::mutex_common::State;

/// A RAII permit for a single unit of concurrency.
///
/// When this value is dropped (returned), the permit is released, but the "slot" it occupied
/// remains unavailable for the configured `interval` of the rate limiter. This means the
/// cooldown period starts at the moment the permit is dropped, not when it was created.
///
/// The permit holds an [`Arc`](std::sync::Arc) to the shared state of the rate limiter, so it does
/// not borrow the [`RateLimiter`] it came from and can outlive any reference to it.
pub type SinglePermit = super::mutex_common::SinglePermit<std::sync::Arc<std::sync::Mutex<State>>>;

/// A RAII permit for multiple units of concurrency.
///
/// When this value is dropped (returned), the permits are released, but the "slots" they occupied
/// remain unavailable for the configured `interval` of the rate limiter. This means the
/// cooldown period starts at the moment the permits are dropped.
///
/// The permit holds an [`Arc`](std::sync::Arc) to the shared state of the rate limiter, so it does
/// not borrow the [`RateLimiter`] it came from and can outlive any reference to it.
pub type MultiPermit = super::mutex_common::MultiPermit<std::sync::Arc<std::sync::Mutex<State>>>;

/// A rate limiter that enforces a cooldown period after usage (return-time based).
///
/// `MAX_SIMULTANEOUS` defines the maximum number of "slots" available.
/// A slot is occupied if a permit is currently held, OR if a permit was recently
/// held and the cooldown `interval` has not yet passed since it was dropped.
///
/// This implies that long-running tasks holding a permit will delay the availability
/// of that slot for future tasks until `duration_held + interval` time has passed.
pub type RateLimiter<const MAX_SIMULTANEOUS: usize> =
    super::mutex_common::RateLimiter<MAX_SIMULTANEOUS, std::sync::Arc<std::sync::Mutex<State>>>;

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
        let rate_limiter = RateLimiter::<42>::new(chrono::Duration::seconds(43));

        let _permit = rate_limiter
            .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
            .unwrap();
    }

    #[test]
    fn cannot_acquire_permit_immediately_after_exhausted_construction() {
        let start_time = dt_from_str("2022-01-02 03:04:05Z");
        let interval = chrono::Duration::seconds(43);
        let rate_limiter = RateLimiter::<42>::new_exhausted_impl(interval, start_time);

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
            let rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter
                .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CONNECTION_COUNT> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter
                .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<5> {
                interval: chrono::Duration::seconds(5),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter.try_acquire_permit_impl(&current_time).unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<6> {
                interval: chrono::Duration::seconds(5),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter
                .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CONNECTION_COUNT> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let result = rate_limiter.try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"));

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(next_permit_time, None);

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<2> {
                interval: chrono::Duration::seconds(5),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
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
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<5> {
                interval: chrono::Duration::seconds(5),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
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
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<5> {
                interval: chrono::Duration::seconds(5),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
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
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<10> {
                interval: chrono::Duration::seconds(5),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
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
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let mut permit = rate_limiter
                .try_acquire_permit_impl(&dt_from_str("2022-01-02 03:04:05Z"))
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
                State {
                    active_connection_count: 2,
                    expiry_times: std::collections::VecDeque::default()
                }
            );

            permit.drop_impl(dt_from_str("2022-01-02 03:04:06Z"));

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5)
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), REQUESTED)
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5)
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), REQUESTED)
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let _permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), CAPACITY)
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(next_permit_time, None);

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(next_permit_time, None);

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };

            let result =
                rate_limiter.try_acquire_permits_impl(&dt_from_str("2022-01-02 03:04:05Z"), 5);

            let Err(Error::NoPermitAvailable(next_permit_time)) = result else {
                panic!("Expected NoPermitAvailable error");
            };
            assert_eq!(next_permit_time, None);

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(15),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
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
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(15),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
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
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(15),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
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
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<CAPACITY> {
                interval: chrono::Duration::seconds(15),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
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
                *rate_limiter.state.lock().unwrap(),
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
            let rate_limiter = RateLimiter::<42> {
                interval: chrono::Duration::seconds(43),
                state: std::sync::Arc::new(std::sync::Mutex::new(initial_state)),
            };
            let mut permit = rate_limiter
                .try_acquire_permits_impl(&dt_from_str("2022-01-01 12:00:00Z"), 3)
                .unwrap();

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
                State {
                    active_connection_count: 8,
                    expiry_times: initial_expiry_times,
                }
            );

            permit.drop_impl(dt_from_str("2022-01-01 12:00:05Z"));

            assert_eq!(
                *rate_limiter.state.lock().unwrap(),
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
            let limiter = RateLimiter::<1>::new(interval);

            let start = chrono::Utc::now();
            let _permit = limiter.acquire_permit().await;
            let elapsed = chrono::Utc::now() - start;

            assert!(elapsed < interval);
        }

        #[tokio::test]
        async fn single_permit_cooldown() {
            let interval = chrono::Duration::milliseconds(100);
            let limiter = RateLimiter::<1>::new(interval);

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
            let limiter = RateLimiter::<3>::new(interval);

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
