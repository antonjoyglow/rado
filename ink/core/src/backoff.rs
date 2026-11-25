use std::time::Duration;

/// How often the list is asked for while Rado answers.
pub const POLL: Duration = Duration::from_secs(30);
const FIRST_RETRY: Duration = Duration::from_secs(5);
const LAST_RETRY: Duration = Duration::from_secs(60);

/// The wait before the next list request: the poll interval while Rado answers, and 5 s doubling to 60 s while it doesn't.
#[derive(Debug, Default)]
pub struct Backoff {
    retry: Option<Duration>,
}

impl Backoff {
    pub fn answered(&mut self) -> Duration {
        self.retry = None;
        POLL
    }

    pub fn failed(&mut self) -> Duration {
        let next = self.retry.map_or(FIRST_RETRY, |last| (last * 2).min(LAST_RETRY));
        self.retry = Some(next);
        next
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn failures_backOffFrom5To60_andAnAnswerGoesBackTo30() {
        let mut backoff = Backoff::default();
        let waits: Vec<u64> = (0..6).map(|_| backoff.failed().as_secs()).collect();
        assert_eq!(waits, [5, 10, 20, 40, 60, 60]);
        assert_eq!(backoff.answered(), POLL);
        assert_eq!(backoff.failed().as_secs(), 5);
    }
}
