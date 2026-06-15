/// An `Arc`-backed implementation of an unfair trailing-edge rate limiter.
///
/// This is the concrete implementation that backs the [`RateLimiter`] alias. Its permits hold an
/// `Arc` to the shared state rather than a borrow of the limiter, so a permit can be held while
/// the value that owns the limiter is used mutably. Name this module directly only when you
/// specifically want this variant; otherwise prefer selecting a limiter by property through
/// [`RateLimiter`] or [`trailing_edge::RateLimiter`](crate::trailing_edge::RateLimiter).
pub mod arc_mutex;

mod mutex_common;

/// A mutex-backed implementation of an unfair trailing-edge rate limiter whose permits borrow the
/// limiter.
///
/// Name this module directly only when you specifically want the borrow-based variant; otherwise
/// prefer selecting a limiter by property through [`RateLimiter`] or
/// [`trailing_edge::RateLimiter`](crate::trailing_edge::RateLimiter).
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
/// [`arc_mutex::RateLimiter`]).
pub type RateLimiter<const MAX_SIMULTANEOUS: usize> = arc_mutex::RateLimiter<MAX_SIMULTANEOUS>;

/// The error type for the default unfair trailing-edge [`RateLimiter`](crate::RateLimiter).
pub type Error = arc_mutex::Error;
