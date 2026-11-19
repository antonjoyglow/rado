//! What one source's poll gives the session, in the same shape for Azure DevOps and GitHub. Each source's adapter
//! (`ado::digest`, `github::digest`) builds it; the session files it without knowing which source it came from.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::group::Related;
use crate::score::{PrThreads, ReviewItem};

/// Why a source's poll failed, which sets how long it waits before the next try.
#[derive(Debug, Clone, PartialEq)]
pub enum FailureKind {
    /// Throttled; `until` is when the source said to come back, if it said.
    RateLimited { until: Option<DateTime<Utc>> },
    Rejected,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceError {
    pub kind: FailureKind,
    pub message: String,
}

impl SourceError {
    pub fn rate_limited(until: Option<DateTime<Utc>>) -> Self {
        Self { kind: FailureKind::RateLimited { until }, message: "rate limited".into() }
    }

    pub fn rejected(message: &str) -> Self {
        Self { kind: FailureKind::Rejected, message: message.into() }
    }

    /// The same error under a source's name, such as `Azure DevOps: `.
    pub fn named(self, source: &str) -> Self {
        Self { message: format!("{source}: {}", self.message), ..self }
    }
}

impl From<String> for SourceError {
    fn from(message: String) -> Self {
        Self { kind: FailureKind::Other, message }
    }
}

impl From<&str> for SourceError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

/// When a throttled response says to come back: `Retry-After` in seconds, or else the `X-RateLimit-Reset` epoch
/// second when no requests remain. Azure DevOps and GitHub both send these.
pub fn retry_at(headers: &reqwest::header::HeaderMap, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let number = |name: &str| headers.get(name)?.to_str().ok()?.trim().parse::<i64>().ok();
    if let Some(seconds) = number("retry-after") {
        return Some(now + chrono::Duration::seconds(seconds.max(0)));
    }
    if number("x-ratelimit-remaining") == Some(0) {
        return DateTime::from_timestamp(number("x-ratelimit-reset")?, 0);
    }
    None
}

/// What the MCP queue needs from a PR beyond its list row.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueRef {
    /// The Azure DevOps project, or the GitHub owner.
    pub project: String,
    /// The Azure DevOps repository id, or `owner/name` on GitHub.
    pub repo_id: String,
    pub agent: bool,
    /// The PR's head commit.
    pub source_commit: Option<String>,
    /// The commit of my latest approval or change request.
    pub last_voted: Option<String>,
}

/// One source's fresh answer, classified but not yet filed.
pub struct SourcePoll {
    /// The PRs that involve me, in no particular order, without comments or last activity yet.
    pub items: Vec<ReviewItem>,
    pub urls: HashMap<i64, String>,
    /// Comment counts and last activity, by PR id; a PR whose threads didn't load has none.
    pub threads: HashMap<i64, PrThreads>,
    /// Other PRs on the branches of mine, where I am not a reviewer.
    pub related: Vec<Related>,
    /// By PR id.
    pub queue_refs: HashMap<i64, QueueRef>,
    /// Errors that came back next to data, already named for the status line: some PRs may be missing, so the
    /// list can't prune hidden PRs or forget the baseline's unlisted ones.
    pub warning: Option<String>,
    /// Reviews waiting on me may be missing, so the queue names the source as missing.
    pub queue_partial: bool,
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        pairs.iter().map(|(k, v)| (reqwest::header::HeaderName::from_static(k), HeaderValue::from_str(v).unwrap())).collect()
    }

    fn now() -> DateTime<Utc> {
        "2026-11-19T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn retryAt_retryAfterSeconds_isThatLongFromNow() {
        let at = retry_at(&headers(&[("retry-after", "90"), ("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1")]), now());
        assert_eq!(at, Some(now() + chrono::Duration::seconds(90)));
    }

    #[test]
    fn retryAt_noRequestsLeft_isTheResetTime() {
        let reset = now() + chrono::Duration::minutes(20);
        let at = retry_at(&headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", &reset.timestamp().to_string())]), now());
        assert_eq!(at, Some(reset));
    }

    #[test]
    fn retryAt_requestsLeftOrNoHeaders_isNone() {
        assert_eq!(retry_at(&headers(&[("x-ratelimit-remaining", "12"), ("x-ratelimit-reset", "1")]), now()), None);
        assert_eq!(retry_at(&HeaderMap::new(), now()), None);
    }
}
