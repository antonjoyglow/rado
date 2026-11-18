//! What one source's poll gives the session, in the same shape for Azure DevOps and GitHub. Each source's adapter
//! (`ado::digest`, `github::digest`) builds it; the session files it without knowing which source it came from.

use std::collections::HashMap;

use crate::group::Related;
use crate::score::{PrThreads, ReviewItem};

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
