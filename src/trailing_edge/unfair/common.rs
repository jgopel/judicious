pub(in crate::trailing_edge::unfair) type ExpiryTimes =
    std::collections::VecDeque<chrono::NaiveDateTime>;

/// Internal state of the rate limiter.
#[derive(Debug, Clone, Default, PartialEq)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "sibling implementation modules test shared state without exposing fields publicly"
)]
pub struct State {
    pub(in crate::trailing_edge::unfair) active_connection_count: usize,
    pub(in crate::trailing_edge::unfair) expiry_times: ExpiryTimes,
}
