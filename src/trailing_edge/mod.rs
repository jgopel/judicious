/// Trailing-edge rate limiters that do not guarantee fairness between callers.
///
/// "Unfair" means there is no ordering guarantee: when a slot frees up, any waiting caller may
/// win it, rather than the one that has been waiting longest. Giving up fairness keeps the
/// implementations simple and cheap. Reach into this module to narrow your choice to an unfair
/// limiter; reach further (e.g. [`unfair::mutex`]) only when you need a specific implementation.
pub mod unfair;

/// The default trailing-edge [`RateLimiter`](crate::RateLimiter).
///
/// This alias lets you ask for "a trailing-edge rate limiter" by *property* rather than by naming
/// a concrete implementation. Use it when you care that the cooldown starts when permits are
/// returned, but you do not want to commit to a particular fairness guarantee or internal
/// mechanism. The alias may be re-pointed at a different implementation in the future without
/// breaking call sites that depend only on the trailing-edge behaviour.
///
/// If you need stronger guarantees (such as a specific fairness policy), select a more specific
/// alias like [`unfair::RateLimiter`] or a concrete type like [`unfair::mutex::RateLimiter`].
pub type RateLimiter<const MAX_SIMULTANEOUS: usize> = unfair::RateLimiter<MAX_SIMULTANEOUS>;

/// The error type for the default trailing-edge [`RateLimiter`](crate::RateLimiter).
pub type Error = unfair::Error;
