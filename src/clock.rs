//! Small time helpers.

use std::time::{Duration, Instant};

/// The instant `d` ago, or now if the clock hasn't run that long yet (`Instant` can't go
/// before the machine's boot on some platforms).
pub fn ago(d: Duration) -> Instant {
    let now = Instant::now();
    now.checked_sub(d).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    #[test]
    fn never_underflows() {
        let _ = super::ago(std::time::Duration::from_secs(u64::MAX / 4));
        assert!(super::ago(std::time::Duration::from_millis(1)) <= std::time::Instant::now());
    }
}
