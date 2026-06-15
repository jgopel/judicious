/// A mutex-backed implementation of an unfair trailing-edge rate limiter.
///
/// This is the concrete implementation that backs the [`RateLimiter`] alias. Name this module
/// directly only when you specifically want the mutex-based variant; otherwise prefer selecting a
/// limiter by property through [`RateLimiter`] or [`trailing_edge::RateLimiter`](crate::trailing_edge::RateLimiter).
pub mod mutex;

/// The default unfair trailing-edge [`RateLimiter`](crate::RateLimiter).
///
/// This alias lets you ask for "an unfair trailing-edge rate limiter" by *property* rather than by
/// naming a concrete implementation. Use it when you have decided you do not need fairness between
/// callers but do not want to commit to a particular internal mechanism. The alias may be
/// re-pointed at a different unfair implementation in the future without breaking call sites that
/// depend only on the unfair, trailing-edge behaviour.
///
/// To pin a specific implementation, name the concrete type directly (e.g.
/// [`mutex::RateLimiter`]).
pub type RateLimiter<const MAX_SIMULTANEOUS: usize> = mutex::RateLimiter<MAX_SIMULTANEOUS>;

/// The error type for the default unfair trailing-edge [`RateLimiter`](crate::RateLimiter).
pub type Error = mutex::Error;
