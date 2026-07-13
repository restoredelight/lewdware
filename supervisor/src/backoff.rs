use std::time::Duration;

pub const MAX_ATTEMPTS: u32 = 3;

/// `attempt` is 1-based (the attempt about to be made). 2s, 4s, 8s, capped at 30s.
pub fn delay_for(attempt: u32) -> Duration {
    let secs = 2u64.saturating_pow(attempt.clamp(1, 5)).min(30);
    Duration::from_secs(secs)
}

pub enum Outcome {
    Retry(Duration),
    GiveUp,
}

/// `attempts_so_far` is how many auto-restarts have already happened for the current episode
/// (0 right after the first crash).
pub fn next_action(attempts_so_far: u32) -> Outcome {
    if attempts_so_far >= MAX_ATTEMPTS {
        Outcome::GiveUp
    } else {
        Outcome::Retry(delay_for(attempts_so_far + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_up_to_max_attempts_with_growing_delay() {
        let delays: Vec<_> = (0..MAX_ATTEMPTS)
            .map(|attempts| match next_action(attempts) {
                Outcome::Retry(delay) => delay,
                Outcome::GiveUp => panic!("expected a retry at attempt {attempts}"),
            })
            .collect();

        assert_eq!(
            delays,
            vec![
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
            ]
        );
    }

    #[test]
    fn gives_up_once_max_attempts_reached() {
        assert!(matches!(next_action(MAX_ATTEMPTS), Outcome::GiveUp));
        assert!(matches!(next_action(MAX_ATTEMPTS + 5), Outcome::GiveUp));
    }

    #[test]
    fn delay_is_capped() {
        assert_eq!(delay_for(10), Duration::from_secs(30));
    }
}
