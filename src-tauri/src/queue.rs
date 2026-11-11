//! The review queue that Rado exports for its MCP server (`src/bin/rado-mcp.rs`). This file uses only serde,
//! serde_json and chrono, so the server can include it without Tauri.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const QUEUE_FILE: &str = "queue.json";
pub const DRAFTS_FILE: &str = "agent-drafts.json";

/// The PRs waiting on my review at the last successful poll: not hidden, and not my own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub refreshed: DateTime<Utc>,
    pub organization: String,
    pub poll_minutes: u32,
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
    pub pr_id: i64,
    pub project: String,
    pub repo: String,
    pub repo_id: String,
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

/// The source commit at which the agent last drafted a review, keyed by PR id.
pub type Drafts = HashMap<i64, String>;

impl Queue {
    pub fn contains(&self, pr_id: i64) -> bool {
        self.branches.iter().flat_map(|b| &b.prs).any(|p| p.pr_id == pr_id)
    }
}

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

/// Forgets drafts for PRs that have left the queue.
pub fn prune(drafts: &mut Drafts, queue: &Queue) {
    drafts.retain(|id, _| queue.contains(*id));
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Writes to a temporary file and renames it over the target, so a reader never sees half a file.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
#[allow(non_snake_case)]
pub(crate) mod tests {
    use super::*;

    pub fn pr(pr_id: i64, commit: &str) -> QueuedPr {
        QueuedPr {
            pr_id,
            project: "Contoso".into(),
            repo: "Api".into(),
            repo_id: "repo-id".into(),
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
        Queue { refreshed: "2026-11-10T09:00:00Z".parse().unwrap(), organization: "org".into(), poll_minutes: 3, branches }
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
    fn prune_prLeftTheQueue_isForgotten() {
        let q = queue(vec![branch("a", vec![pr(1, "c1")])]);
        let mut drafts = Drafts::from([(1, "c1".to_string()), (9, "c9".to_string())]);

        prune(&mut drafts, &q);

        assert_eq!(drafts, Drafts::from([(1, "c1".to_string())]));
    }

    #[test]
    fn staleAfterMinutes_isThreePollsWithA30MinuteFloor() {
        assert_eq!(stale_after_minutes(3), 30);
        assert_eq!(stale_after_minutes(20), 60);
    }

    #[test]
    fn drafts_roundTripThroughJson() {
        let drafts = Drafts::from([(42, "abc".to_string())]);
        let json = serde_json::to_string(&drafts).unwrap();
        assert_eq!(serde_json::from_str::<Drafts>(&json).unwrap(), drafts);
    }
}
