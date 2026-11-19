//! The review queue that Rado exports for its MCP server (`src/bin/rado-mcp.rs`). This file uses only serde,
//! serde_json, chrono and indexmap, so the server can include it without Tauri.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const QUEUE_FILE: &str = "queue.json";
pub const DRAFTS_FILE: &str = "agent-drafts.json";
/// Held while a draft is recorded, so two agents marking at once don't drop each other's mark.
pub const DRAFTS_LOCK_FILE: &str = "agent-drafts.lock";
/// The drafts file keeps the newest marks only, so it stays small.
pub const MAX_DRAFTS: usize = 500;
pub const SETTINGS_FILE: &str = "settings.json";
/// The queue file's schema. Bump it only for a change an older `rado-mcp` would misread: readers ignore fields they
/// don't know, so an added field needs no bump.
pub const QUEUE_SCHEMA: u32 = 1;

/// Letters, digits and dashes only, so an id read from a hand-edited file can't point outside the data folder.
pub fn is_workspace_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Where a workspace keeps its queue, drafts, votes and hidden PRs.
pub fn workspace_dir(root: &Path, id: &str) -> PathBuf {
    root.join("workspaces").join(id)
}

/// The PRs waiting on my review at the last successful poll: not hidden, and not my own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    /// `QUEUE_SCHEMA` when written; files from before 0.11.0 have none and are schema 1.
    #[serde(default = "first_schema")]
    pub schema_version: u32,
    pub refreshed: DateTime<Utc>,
    /// The Azure DevOps organization; empty for a workspace with GitHub only.
    pub organization: String,
    pub poll_minutes: u32,
    /// Sources whose last poll failed or came back partial (`azureDevOps`, `gitHub`): some of their PRs may be missing
    /// from `branches`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_sources: Vec<String>,
    /// Best-ranked first. One branch is one change, even when it spans several repos.
    pub branches: Vec<Branch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub branch: String,
    pub prs: Vec<QueuedPr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedPr {
    /// `azureDevOps` or `gitHub`; files written before GitHub was queued have Azure DevOps PRs only.
    #[serde(default = "azure_dev_ops")]
    pub provider: String,
    /// Unique across providers: the Azure DevOps PR id, or GitHub's database id. `mark_drafted` takes this one.
    pub pr_id: i64,
    /// The number in the PR's URL: the PR id on Azure DevOps, the PR number on GitHub.
    #[serde(default)]
    pub number: i64,
    /// The Azure DevOps project, or the GitHub owner.
    pub project: String,
    pub repo: String,
    /// The Azure DevOps repository id, or `owner/repo` on GitHub.
    pub repo_id: String,
    /// Opened by a bot account, or labelled as agent work (`agent`, `ai`, `ai-generated`, `ai-assisted`).
    #[serde(default)]
    pub agent_authored: bool,
    pub url: String,
    pub title: String,
    pub author: String,
    pub target: String,
    /// `needsReview`, or `reReview` when there are new pushes since my vote.
    pub state: String,
    pub tier: String,
    pub score: i32,
    pub age_hours: i64,
    pub draft: bool,
    pub required: bool,
    pub source_commit: Option<String>,
    /// The source commit when I last voted; set for a re-review, so only the changes since then need a look.
    pub last_voted_commit: Option<String>,
}

pub const AZURE_DEV_OPS: &str = "azureDevOps";
pub const GITHUB: &str = "gitHub";

fn azure_dev_ops() -> String {
    AZURE_DEV_OPS.into()
}

fn first_schema() -> u32 {
    1
}

/// The source commit at which the agent last drafted a review, keyed by PR id, oldest mark first. Still a plain JSON
/// object, so older `rado-mcp` copies read it.
pub type Drafts = IndexMap<i64, String>;

/// How old the queue may be before an agent must not trust it: three missed polls, and never less than 30 minutes.
pub fn stale_after_minutes(poll_minutes: u32) -> i64 {
    (3 * i64::from(poll_minutes)).max(30)
}

/// The queue without the PRs already drafted at their current source commit; a new push brings a PR back.
pub fn pending(queue: &Queue, drafts: &Drafts) -> Vec<Branch> {
    queue
        .branches
        .iter()
        .filter_map(|b| {
            let prs: Vec<QueuedPr> = b.prs.iter().filter(|p| !is_drafted(p, drafts)).cloned().collect();
            (!prs.is_empty()).then(|| Branch { branch: b.branch.clone(), prs })
        })
        .collect()
}

fn is_drafted(pr: &QueuedPr, drafts: &Drafts) -> bool {
    pr.source_commit.as_ref().is_some_and(|c| drafts.get(&pr.pr_id) == Some(c))
}

/// Records a draft as the newest mark and forgets the oldest past `MAX_DRAFTS`. A PR that leaves the queue keeps its
/// draft, so it isn't drafted again when it comes back at the same commit.
pub fn record_draft(drafts: &mut Drafts, pr_id: i64, commit: &str) {
    drafts.shift_remove(&pr_id);
    drafts.insert(pr_id, commit.to_string());
    let excess = drafts.len().saturating_sub(MAX_DRAFTS);
    drafts.drain(..excess);
}

/// The file's contents, or None when it is missing, unreadable or not valid.
pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    read_json_file(path).ok().flatten()
}

/// Why a file that exists couldn't be used; each message names the file.
#[derive(Debug)]
pub enum ReadError {
    /// Held by another program, most likely for a moment.
    Unreadable(String),
    /// Damaged, or not the expected shape.
    Invalid(String),
}

/// The file's contents, or None when it doesn't exist.
pub fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, ReadError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ReadError::Unreadable(format!("Cannot read {}: {e}", path.display()))),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|e| ReadError::Invalid(format!("{} is not valid: {e}", path.display())))
}

/// Writes to a temporary file and renames it over the target, so a reader never sees half a file. The temporary name
/// is unique to this write, so writers in other processes or threads don't collide on it.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("json.{}-{}.tmp", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, path)).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {e}", path.display())
    })
}

#[cfg(test)]
#[allow(non_snake_case)]
pub(crate) mod tests {
    use super::*;

    pub fn pr(pr_id: i64, commit: &str) -> QueuedPr {
        QueuedPr {
            provider: AZURE_DEV_OPS.into(),
            pr_id,
            number: pr_id,
            project: "Contoso".into(),
            repo: "Api".into(),
            repo_id: "repo-id".into(),
            agent_authored: false,
            url: format!("https://dev.azure.com/org/Contoso/_git/Api/pullrequest/{pr_id}"),
            title: format!("PR {pr_id}"),
            author: "Author".into(),
            target: "dev".into(),
            state: "needsReview".into(),
            tier: "normal".into(),
            score: 50,
            age_hours: 8,
            draft: false,
            required: false,
            source_commit: Some(commit.into()),
            last_voted_commit: None,
        }
    }

    pub fn queue(branches: Vec<Branch>) -> Queue {
        Queue { schema_version: QUEUE_SCHEMA, refreshed: "2026-11-10T09:00:00Z".parse().unwrap(), organization: "org".into(), poll_minutes: 3, missing_sources: vec![], branches }
    }

    #[test]
    fn queue_writtenBeforeGitHubWasQueued_readsAsAzureDevOps() {
        let old = r#"{"refreshed":"2026-11-10T09:00:00Z","organization":"org","pollMinutes":3,"branches":[{"branch":"a","prs":[
            {"prId":7,"project":"P","repo":"R","repoId":"rid","url":"u","title":"t","author":"a","target":"dev","state":"needsReview",
             "tier":"normal","score":50,"ageHours":1,"draft":false,"required":false,"sourceCommit":"c","lastVotedCommit":null}]}]}"#;
        let q: Queue = serde_json::from_str(old).unwrap();
        let pr = &q.branches[0].prs[0];
        assert_eq!((pr.provider.as_str(), pr.pr_id, pr.agent_authored, q.missing_sources.len()), (AZURE_DEV_OPS, 7, false, 0));
    }

    /// 0.10.0's `Queue`, as an older `rado-mcp` still reads it; `Branch` and `QueuedPr` are unchanged since then.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct QueueBeforeSchema {
        refreshed: DateTime<Utc>,
        organization: String,
        poll_minutes: u32,
        #[serde(default)]
        missing_sources: Vec<String>,
        branches: Vec<Branch>,
    }

    #[test]
    fn queue_withASchema_staysReadableByOlderServers() {
        let q = queue(vec![branch("a", vec![pr(1, "c1")])]);
        let json = serde_json::to_string(&q).unwrap();

        assert!(json.contains(r#""schemaVersion":1"#), "{json}");
        let old: QueueBeforeSchema = serde_json::from_str(&json).unwrap();
        assert_eq!((old.refreshed, old.branches), (q.refreshed, q.branches));
    }

    fn branch(name: &str, prs: Vec<QueuedPr>) -> Branch {
        Branch { branch: name.into(), prs }
    }

    #[test]
    fn pending_draftedAtCurrentCommit_isLeftOut() {
        let q = queue(vec![branch("a", vec![pr(1, "c1"), pr(2, "c2")]), branch("b", vec![pr(3, "c3")])]);
        let drafts = Drafts::from([(1, "c1".to_string()), (3, "c3".to_string())]);

        let pending = pending(&q, &drafts);

        assert_eq!(pending, vec![branch("a", vec![pr(2, "c2")])]);
    }

    #[test]
    fn pending_newPushSinceDraft_comesBack() {
        let q = queue(vec![branch("a", vec![pr(1, "c2")])]);
        let drafts = Drafts::from([(1, "c1".to_string())]);

        assert_eq!(pending(&q, &drafts), q.branches);
    }

    #[test]
    fn recordDraft_pastTheCap_forgetsTheOldestMark() {
        let mut drafts: Drafts = (0..MAX_DRAFTS as i64).map(|id| (id, "c".to_string())).collect();
        record_draft(&mut drafts, 0, "new");

        record_draft(&mut drafts, 9999, "c9999");

        assert_eq!(drafts.len(), MAX_DRAFTS);
        assert!(!drafts.contains_key(&1), "1 is now the oldest mark");
        assert_eq!(drafts.get(&0).map(String::as_str), Some("new"), "marking again made 0 the newest, not the oldest");
        assert_eq!(drafts.last(), Some((&9999, &"c9999".to_string())));
    }

    #[test]
    fn drafts_roundTripInOrder_andStayReadableAsAPlainMap() {
        let drafts = Drafts::from([(42, "abc".to_string()), (7, "def".to_string())]);
        let json = serde_json::to_string(&drafts).unwrap();

        assert_eq!(json, r#"{"42":"abc","7":"def"}"#);
        assert_eq!(serde_json::from_str::<Drafts>(&json).unwrap(), drafts);
        let old = serde_json::from_str::<std::collections::HashMap<i64, String>>(&json).unwrap();
        assert_eq!(old, std::collections::HashMap::from([(42, "abc".to_string()), (7, "def".to_string())]));
    }

    #[test]
    fn staleAfterMinutes_isThreePollsWithA30MinuteFloor() {
        assert_eq!(stale_after_minutes(3), 30);
        assert_eq!(stale_after_minutes(20), 60);
    }
}
