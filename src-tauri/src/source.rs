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

    /// A failed response that isn't throttling; a `Retry-After` on it (a 503's, say) still sets the next try.
    pub fn http(message: String, retry_after: Option<DateTime<Utc>>) -> Self {
        match retry_after {
            Some(at) => Self { kind: FailureKind::RateLimited { until: Some(at) }, message },
            None => message.into(),
        }
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
/// second when no requests remain. Azure DevOps and GitHub both send these. Never more than a day away.
pub fn retry_at(headers: &reqwest::header::HeaderMap, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    retry_after(headers, now).or_else(|| {
        let reset = header_number(headers, "x-ratelimit-reset").filter(|_| header_number(headers, "x-ratelimit-remaining") == Some(0))?;
        DateTime::from_timestamp(reset, 0).map(|at| at.min(now + chrono::Duration::seconds(DAY_SECONDS)))
    })
}

/// When `Retry-After` (in seconds) says to send the next request, on any response. Azure DevOps's
/// `X-RateLimit-Reset` is when its usage would drain, not a time to come back, so only a throttled answer reads it.
pub fn retry_after(headers: &reqwest::header::HeaderMap, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    header_number(headers, "retry-after").map(|seconds| now + chrono::Duration::seconds(seconds.clamp(0, DAY_SECONDS)))
}

fn header_number(headers: &reqwest::header::HeaderMap, name: &str) -> Option<i64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

const DAY_SECONDS: i64 = 86_400;

/// The longest a request waits for a `Retry-After` inside a poll; a longer one ends the poll as a rate limit.
pub const RETRY_AFTER_SLEEP_SECONDS: i64 = 30;

/// What a request does before sending, given the latest `Retry-After` time its source named.
#[derive(Debug, PartialEq)]
pub enum Pace {
    Send,
    Sleep(std::time::Duration),
    /// Too long to wait inside a poll.
    Hold(DateTime<Utc>),
}

pub fn pace(retry_after: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Pace {
    let Some(at) = retry_after.filter(|at| *at > now) else { return Pace::Send };
    let wait = at - now;
    if wait > chrono::Duration::seconds(RETRY_AFTER_SLEEP_SECONDS) {
        return Pace::Hold(at);
    }
    Pace::Sleep(wait.to_std().unwrap_or_default())
}

/// Whether a response is throttling: a 429, or a 403 that carries the rate-limit headers (GitHub's way).
pub fn throttled(status: reqwest::StatusCode, retry: Option<DateTime<Utc>>) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || (status == reqwest::StatusCode::FORBIDDEN && retry.is_some())
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
    fn retryAt_hugeValues_areCappedAtADay() {
        let day = now() + chrono::Duration::days(1);
        assert_eq!(retry_at(&headers(&[("retry-after", "99999999999999")]), now()), Some(day));
        assert_eq!(retry_at(&headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "99999999999")]), now()), Some(day));
    }

    #[test]
    fn throttled_is429_or403WithRateLimitHeaders() {
        use reqwest::StatusCode;
        let later = Some(now());
        assert!(throttled(StatusCode::TOO_MANY_REQUESTS, None));
        assert!(throttled(StatusCode::FORBIDDEN, later));
        assert!(!throttled(StatusCode::FORBIDDEN, None), "a 403 without the headers is a permission error");
        assert!(!throttled(StatusCode::OK, later));
    }

    #[test]
    fn retryAfter_readsOnlyRetryAfter_notTheResetTime() {
        let reset = (now() + chrono::Duration::minutes(20)).timestamp().to_string();
        assert_eq!(retry_after(&headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", &reset)]), now()), None);
        assert_eq!(retry_after(&headers(&[("retry-after", "5")]), now()), Some(now() + chrono::Duration::seconds(5)));
        assert_eq!(retry_after(&headers(&[("retry-after", "99999999999999")]), now()), Some(now() + chrono::Duration::days(1)));
    }

    #[test]
    fn http_namedTime_waitsLikeARateLimit_keepingItsMessage() {
        let later = now() + chrono::Duration::minutes(10);
        let named = SourceError::http("HTTP 503 Service Unavailable".into(), Some(later));
        assert_eq!(named.kind, FailureKind::RateLimited { until: Some(later) });
        assert_eq!(named.message, "HTTP 503 Service Unavailable");
        assert_eq!(SourceError::http("HTTP 503 Service Unavailable".into(), None).kind, FailureKind::Other);
    }

    #[test]
    fn pace_sleepsThroughAShortRetryAfter_andHoldsForALongOne() {
        let seconds = |s| Some(now() + chrono::Duration::seconds(s));
        assert_eq!(pace(None, now()), Pace::Send);
        assert_eq!(pace(seconds(-1), now()), Pace::Send, "a past time asks for nothing");
        assert_eq!(pace(seconds(0), now()), Pace::Send);
        assert_eq!(pace(seconds(30), now()), Pace::Sleep(std::time::Duration::from_secs(30)));
        assert_eq!(pace(seconds(31), now()), Pace::Hold(now() + chrono::Duration::seconds(31)));
    }

    #[test]
    fn retryAt_requestsLeftOrNoHeaders_isNone() {
        assert_eq!(retry_at(&headers(&[("x-ratelimit-remaining", "12"), ("x-ratelimit-reset", "1")]), now()), None);
        assert_eq!(retry_at(&HeaderMap::new(), now()), None);
    }
}
