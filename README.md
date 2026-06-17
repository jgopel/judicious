# Judicious

`judicious` is a Rust library that implements rate limiting based on event
completion time instead of event start time. Unlike standard rate limiters (like
token buckets or leaky buckets) which enforce limits based on when a permit is
_acquired_, `judicious` enforces a cooldown period starting from when a permit
is returned (dropped).

## Usage

The main feature currently is `trailing_edge::RateLimiter`. Aliases let you
select an implementation by property rather than by naming a concrete type:
`trailing_edge::RateLimiter` asks for any trailing-edge limiter, while
`trailing_edge::unfair::RateLimiter` narrows it to an unfair one. There are
future plans for adding a fair rate limiter.

```rust
use chrono::Duration;
use judicious::RateLimiter as _;

#[tokio::main]
async fn main() {
    // Create a limiter allowing 1 concurrent operation.
    // Once an operation finishes, that slot is unavailable for 1 second.
    const NUM_SLOTS: usize = 1;
    let limiter = judicious::trailing_edge::RateLimiter::<NUM_SLOTS>::new(Duration::seconds(1));

    {
        println!("Acquiring permit...");
        let _permit = limiter.try_acquire_permit().expect("Permit should be available");

        // Simulate long async work
        println!("Doing work...");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        println!("Work done. Dropping permit.");
    }
    // Permit is dropped here. The 1-second cooldown starts NOW.

    // This will fail immediately because the cooldown is active.
    let result = limiter.try_acquire_permit();
    assert!(result.is_err());
}
```
