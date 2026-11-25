//! Rado's e-ink API v1 as the device reads it: `GET /v1/list` over plain HTTP/1.1, one request per connection.
//! The contract is `ink/fixtures/list.json`, which Rado's tests write from its own types.

use chrono::{DateTime, FixedOffset};
use serde::Deserialize;

use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct List {
    pub workspace: String,
    pub counts: Counts,
    pub last_refreshed: Option<DateTime<FixedOffset>>,
    /// Rado's source error, such as a failing Azure DevOps source and its next try.
    pub error: Option<String>,
    /// In Rado's rank order.
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Counts {
    pub reviews: u32,
    pub mine: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub key: String,
    pub tab: Tab,
    pub tier: Tier,
    pub title: String,
    pub age_hours: i64,
    /// Counts toward the counter: waiting on me.
    pub waiting: bool,
    pub draft: Option<Draft>,
}

/// The tabs the device tells apart; every other tab, and any added later, is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tab {
    ReReview,
    Mine,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    Low,
    Urgent,
    /// Also any tier a later Rado adds, so the list still reads.
    #[serde(other)]
    Normal,
}

/// A review agent's draft at the PR's current commit.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Draft {}

#[derive(Debug, PartialEq)]
pub enum Fetched {
    Changed { list: Box<List>, etag: Option<String> },
    NotModified,
    Unauthorized,
}

pub fn list_request(config: &Config, etag: Option<&str>) -> String {
    let mut request = format!(
        "GET /v1/list HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n",
        config.host, config.port, config.token
    );
    if let Some(etag) = etag {
        request.push_str(&format!("If-None-Match: {etag}\r\n"));
    }
    request.push_str("\r\n");
    request
}

/// Reads a whole response to a list request. Rado's own error text comes back as the error.
pub fn parse_response(raw: &[u8]) -> Result<Fetched, String> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("Rado's answer was cut off")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| "Rado's answer was not text")?;
    let body = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or("Rado's answer had no status")?;
    let etag = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("etag").then(|| value.trim().to_string())
    });
    match status {
        200 => {
            let list = serde_json::from_slice(body).map_err(|e| format!("Rado's list didn't read: {e}"))?;
            Ok(Fetched::Changed { list: Box::new(list), etag })
        }
        304 => Ok(Fetched::NotModified),
        401 => Ok(Fetched::Unauthorized),
        _ => {
            #[derive(Deserialize)]
            struct Error {
                error: String,
            }
            Err(serde_json::from_slice::<Error>(body).map(|e| e.error).unwrap_or_else(|_| format!("Rado answered {status}")))
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../fixtures/list.json");

    fn response(head: &str, body: &str) -> Vec<u8> {
        format!("{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn fixture_tabsTheDeviceDoesntTellApart_areOther() {
        let list: List = serde_json::from_str(FIXTURE).unwrap();
        let tab = |key: &str| list.rows.iter().find(|r| r.key == key).unwrap().tab;
        assert_eq!((tab("pr:9"), tab("pr:1"), tab("pr:8"), tab("pr:4")), (Tab::Other, Tab::Other, Tab::Other, Tab::ReReview));
    }

    #[test]
    fn aTierALaterRadoAdds_readsAsNormal() {
        assert_eq!(serde_json::from_str::<Tier>(r#""critical""#).unwrap(), Tier::Normal);
    }

    #[test]
    fn listRequest_sendsTokenAndEtag() {
        let config = Config { host: "192.168.1.14".into(), port: 7878, token: "t0k".into() };
        assert_eq!(
            list_request(&config, Some("\"abc\"")),
            "GET /v1/list HTTP/1.1\r\nHost: 192.168.1.14:7878\r\nAuthorization: Bearer t0k\r\nConnection: close\r\nIf-None-Match: \"abc\"\r\n\r\n"
        );
        assert!(!list_request(&config, None).contains("If-None-Match"));
    }

    #[test]
    fn parseResponse_200_givesListAndEtag() {
        let raw = response("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\netag: \"e1\"", FIXTURE);
        let Ok(Fetched::Changed { list, etag }) = parse_response(&raw) else { panic!() };
        assert_eq!(list.workspace, "Acme");
        assert_eq!(etag.as_deref(), Some("\"e1\""));
    }

    #[test]
    fn parseResponse_304And401() {
        assert_eq!(parse_response(&response("HTTP/1.1 304 Not Modified\r\nETag: \"e1\"", "")), Ok(Fetched::NotModified));
        assert_eq!(
            parse_response(&response("HTTP/1.1 401 Unauthorized", r#"{"error":"unauthorized"}"#)),
            Ok(Fetched::Unauthorized)
        );
    }

    #[test]
    fn parseResponse_failures_giveRadosTextOrWhatWentWrong() {
        assert_eq!(
            parse_response(&response("HTTP/1.1 500 Internal Server Error", r#"{"error":"Session is busy"}"#)),
            Err("Session is busy".into())
        );
        assert_eq!(parse_response(&response("HTTP/1.1 404 Not Found", "")), Err("Rado answered 404".into()));
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\n"), Err("Rado's answer was cut off".into()));
        assert!(parse_response(&response("HTTP/1.1 200 OK", "{}")).unwrap_err().starts_with("Rado's list didn't read"));
    }
}
