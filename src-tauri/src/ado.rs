use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::score::CommentStats;

const API: &str = "api-version=7.1";
const PAGE: usize = 500;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub pull_request_id: i64,
    pub title: String,
    pub created_by: IdentityRef,
    pub creation_date: DateTime<Utc>,
    pub repository: Repository,
    pub source_ref_name: String,
    pub target_ref_name: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub reviewers: Vec<Reviewer>,
    pub last_merge_source_commit: Option<Commit>,
}

impl PullRequest {
    pub fn branch(&self) -> &str {
        self.source_ref_name.trim_start_matches("refs/heads/")
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRef {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub project: Project,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reviewer {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub vote: i32,
    #[serde(default)]
    pub is_required: bool,
    #[serde(default)]
    pub is_container: bool,
    #[serde(default)]
    pub has_declined: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub commit_id: String,
}

#[derive(Deserialize)]
struct List<T> {
    value: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionData {
    authenticated_user: AuthenticatedUser,
}

#[derive(Deserialize)]
struct AuthenticatedUser {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Thread {
    status: Option<String>,
    #[serde(default)]
    is_deleted: bool,
    /// Moves on every push, vote, status change and comment, so it is the PR's last activity.
    last_updated_date: Option<DateTime<Utc>>,
    #[serde(default)]
    comments: Vec<ThreadComment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadComment {
    comment_type: Option<String>,
    #[serde(default)]
    is_deleted: bool,
}

/// Counts what people wrote; vote, push and other system threads don't count.
fn comment_stats(threads: &[Thread]) -> CommentStats {
    let mut stats = CommentStats::default();
    for thread in threads.iter().filter(|t| !t.is_deleted) {
        let written = thread
            .comments
            .iter()
            .filter(|c| !c.is_deleted && c.comment_type.as_deref() == Some("text"))
            .count() as u32;
        if written == 0 {
            continue;
        }
        stats.total += written;
        if matches!(thread.status.as_deref(), Some("active" | "pending")) {
            stats.open_threads += 1;
        }
    }
    stats
}

/// The latest update to any thread, written or system (pushes and votes are system threads).
fn last_activity(threads: &[Thread]) -> Option<DateTime<Utc>> {
    threads.iter().filter(|t| !t.is_deleted).filter_map(|t| t.last_updated_date).max()
}

/// What one threads call tells us about a PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrThreads {
    pub comments: CommentStats,
    pub last_activity: Option<DateTime<Utc>>,
}

pub struct Client {
    http: reqwest::Client,
    org: String,
    auth: String,
}

impl Client {
    pub fn new(org: &str, pat: &str) -> Self {
        let token = base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"));
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("static reqwest config"),
            org: org.to_string(),
            auth: format!("Basic {token}"),
        }
    }

    pub fn pr_url(&self, pr: &PullRequest) -> String {
        format!(
            "https://dev.azure.com/{}/{}/_git/{}/pullrequest/{}",
            self.org,
            urlencode(&pr.repository.project.name),
            urlencode(&pr.repository.name),
            pr.pull_request_id
        )
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, String> {
        let response = self
            .http
            .get(url)
            .header("Authorization", &self.auth)
            .send()
            .await
            .map_err(|e| format!("network: {e}"))?;
        let status = response.status();
        // ADO answers an invalid PAT with a 203 sign-in HTML page instead of a 401.
        if status == reqwest::StatusCode::NON_AUTHORITATIVE_INFORMATION || status == reqwest::StatusCode::UNAUTHORIZED {
            return Err("PAT rejected (expired or missing Code: Read scope)".into());
        }
        let endpoint = url.split('?').next().unwrap_or(url).rsplit("_apis/").next().unwrap_or(url);
        if !status.is_success() {
            return Err(format!("HTTP {status} from {endpoint}"));
        }
        let body = response.bytes().await.map_err(|e| format!("network: {e}"))?;
        serde_json::from_slice::<T>(&body).map_err(|e| format!("unexpected {endpoint} response: {e}"))
    }

    pub async fn my_id(&self) -> Result<String, String> {
        let conn: ConnectionData = self.get(&format!("https://dev.azure.com/{}/_apis/connectionData", self.org)).await?;
        Ok(conn.authenticated_user.id)
    }

    pub async fn active_pull_requests(&self) -> Result<Vec<PullRequest>, String> {
        let mut all = Vec::new();
        loop {
            let url = format!(
                "https://dev.azure.com/{}/_apis/git/pullrequests?searchCriteria.status=active&$top={PAGE}&$skip={}&{API}",
                self.org,
                all.len()
            );
            let page: List<PullRequest> = self.get(&url).await?;
            let done = page.value.len() < PAGE;
            all.extend(page.value);
            if done {
                return Ok(all);
            }
        }
    }

    pub async fn threads(&self, project_id: &str, repo_id: &str, pr_id: i64) -> Result<PrThreads, String> {
        let url = format!(
            "https://dev.azure.com/{}/{project_id}/_apis/git/repositories/{repo_id}/pullRequests/{pr_id}/threads?{API}",
            self.org
        );
        let threads: List<Thread> = self.get(&url).await?;
        Ok(PrThreads { comments: comment_stats(&threads.value), last_activity: last_activity(&threads.value) })
    }
}

pub(crate) fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_connectionDataWithoutDisplayName_parses() {
        let json = r#"{"authenticatedUser":{"id":"84616ca2-2050-634b-b7ae-dc02d548c5d3",
            "descriptor":"Microsoft.IdentityModel.Claims.ClaimsIdentity;x","providerDisplayName":"Antonis",
            "isActive":true,"properties":{}},"instanceId":"i","deploymentType":"hosted"}"#;
        let conn: ConnectionData = serde_json::from_str(json).unwrap();
        assert_eq!(conn.authenticated_user.id, "84616ca2-2050-634b-b7ae-dc02d548c5d3");
    }

    #[test]
    fn deserialize_teamReviewer_keepsDisplayName() {
        let json = r#"{"id":"8294237d","displayName":"[contoso]\\TeamAlpha","vote":10,"isContainer":true}"#;
        let r: Reviewer = serde_json::from_str(json).unwrap();
        assert_eq!(r.display_name, r"[contoso]\TeamAlpha");
        assert!(r.is_container);
    }

    #[test]
    fn commentStats_countsWrittenCommentsAndOpenThreadsOnly() {
        let json = r#"{"value":[
            {"status":"active","comments":[{"commentType":"text"},{"commentType":"text"}]},
            {"status":"fixed","comments":[{"commentType":"text"}]},
            {"status":"pending","comments":[{"commentType":"text","isDeleted":true},{"commentType":"text"}]},
            {"comments":[{"commentType":"system"}]},
            {"status":"active","isDeleted":true,"comments":[{"commentType":"text"}]},
            {"status":"active","comments":[{"commentType":"text","isDeleted":true}]}
        ]}"#;
        let threads: List<Thread> = serde_json::from_str(json).unwrap();
        assert_eq!(comment_stats(&threads.value), CommentStats { total: 4, open_threads: 2 });
    }

    #[test]
    fn lastActivity_isTheLatestLiveThreadIncludingSystemThreads() {
        let json = r#"{"value":[
            {"status":"active","lastUpdatedDate":"2026-11-08T09:00:00Z","comments":[{"commentType":"text"}]},
            {"lastUpdatedDate":"2026-11-09T15:30:00Z","comments":[{"commentType":"system"}]},
            {"isDeleted":true,"lastUpdatedDate":"2026-11-10T08:00:00Z","comments":[]},
            {"comments":[{"commentType":"text"}]}
        ]}"#;
        let threads: List<Thread> = serde_json::from_str(json).unwrap();
        assert_eq!(last_activity(&threads.value), Some("2026-11-09T15:30:00Z".parse().unwrap()));
    }

    #[test]
    fn lastActivity_noThreads_isNone() {
        assert_eq!(last_activity(&[]), None);
    }
}
