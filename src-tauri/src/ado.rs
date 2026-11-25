use std::collections::{HashMap, HashSet};

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::group::Related;
use crate::score::{self, CommentStats, PrFacts, PrThreads, Provider, ReviewItem, ReviewState, VoteMemory, VotedAt};
use crate::source::{Pace, QueueRef, SourceError, SourcePoll, pace, retry_after, retry_at, throttled};

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

/// The latest time a `Retry-After` named, on any answer: Azure DevOps sends it on a 200 when a client nears its
/// limit, and asks for no request before then. It outlives the poll, so a Refresh right after honours it too.
pub type RetryAfter = std::sync::Mutex<Option<DateTime<Utc>>>;

pub struct Client<'a> {
    http: reqwest::Client,
    org: String,
    auth: String,
    retry_after: &'a RetryAfter,
}

impl<'a> Client<'a> {
    pub fn new(org: &str, pat: &str, retry_after: &'a RetryAfter) -> Self {
        let token = base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"));
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("static reqwest config"),
            org: org.to_string(),
            auth: format!("Basic {token}"),
            retry_after,
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

    async fn get<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, SourceError> {
        // Looked at again after a sleep: another request may have been asked to wait longer meanwhile.
        loop {
            let asked = *self.retry_after.lock().expect("retry-after lock");
            match pace(asked, Utc::now()) {
                Pace::Send => break,
                Pace::Sleep(wait) => tokio::time::sleep(wait).await,
                Pace::Hold(until) => return Err(SourceError::rate_limited(Some(until))),
            }
        }
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
        let now = Utc::now();
        let asked = retry_after(response.headers(), now);
        if let Some(at) = asked {
            let mut latest = self.retry_after.lock().expect("retry-after lock");
            *latest = latest.max(Some(at));
        }
        let retry = retry_at(response.headers(), now);
        if throttled(status, retry) {
            return Err(SourceError::rate_limited(retry));
        }
        let endpoint = url.split('?').next().unwrap_or(url).rsplit("_apis/").next().unwrap_or(url);
        if !status.is_success() {
            return Err(SourceError::http(format!("HTTP {status} from {endpoint}"), asked));
        }
        let body = response.bytes().await.map_err(|e| format!("network: {e}"))?;
        Ok(serde_json::from_slice::<T>(&body).map_err(|e| format!("unexpected {endpoint} response: {e}"))?)
    }

    pub async fn my_id(&self) -> Result<String, SourceError> {
        let conn: ConnectionData = self.get(&format!("https://dev.azure.com/{}/_apis/connectionData", self.org)).await?;
        Ok(conn.authenticated_user.id)
    }

    pub async fn active_pull_requests(&self) -> Result<Vec<PullRequest>, SourceError> {
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

    pub async fn threads(&self, project_id: &str, repo_id: &str, pr_id: i64) -> Result<PrThreads, SourceError> {
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

/// One poll's Azure DevOps answer, before it is ranked.
pub struct Fetched {
    pub prs: Vec<PullRequest>,
    pub my_id: String,
    pub teams: HashSet<String>,
    pub threads: HashMap<i64, PrThreads>,
    pub urls: HashMap<i64, String>,
}

/// Ranks the fetched PRs against `memory`, which it updates with my current votes, and finds the related PRs and
/// what the queue needs.
pub fn digest(fetched: Fetched, memory: &mut VoteMemory, now: DateTime<Utc>) -> SourcePoll {
    let me = Me { id: &fetched.my_id, teams: &fetched.teams };
    let items = classify_all(&fetched.prs, &me, memory, now);
    let mine: HashSet<i64> = items.iter().map(|i| i.id).collect();
    let branches: HashSet<&str> = items.iter().map(|i| i.branch.as_str()).collect();
    let related = fetched
        .prs
        .iter()
        .filter(|pr| !mine.contains(&pr.pull_request_id) && branches.contains(pr.branch()))
        .map(|pr| Related {
            provider: Provider::Ado,
            id: pr.pull_request_id,
            repo: pr.repository.name.clone(),
            title: pr.title.clone(),
            branch: pr.branch().to_string(),
            url: fetched.urls.get(&pr.pull_request_id).cloned().unwrap_or_default(),
            created_at: pr.creation_date,
        })
        .collect();
    let queue_refs = fetched
        .prs
        .iter()
        .filter(|pr| mine.contains(&pr.pull_request_id))
        .map(|pr| {
            let reference = QueueRef {
                project: pr.repository.project.name.clone(),
                repo_id: pr.repository.id.clone(),
                agent: false,
                source_commit: pr.last_merge_source_commit.as_ref().map(|c| c.commit_id.clone()),
                last_voted: memory.get(&pr.pull_request_id).map(|v| v.commit.clone()),
            };
            (pr.pull_request_id, reference)
        })
        .collect();
    SourcePoll { items, urls: fetched.urls, threads: fetched.threads, related, queue_refs, warning: None, queue_partial: false }
}

/// Who I am on Azure DevOps: my id, and the teams whose reviews count as mine.
pub struct Me<'a> {
    pub id: &'a str,
    /// Lower-case team names; a team reviewer shows as `[scope]\Name`.
    pub teams: &'a HashSet<String>,
}

impl Me<'_> {
    fn is_my_team(&self, reviewer: &Reviewer) -> bool {
        let name = reviewer.display_name.rsplit('\\').next().unwrap_or_default();
        reviewer.is_container && self.teams.contains(&name.to_lowercase())
    }
}

/// My part in a PR.
enum Role<'a> {
    Author,
    /// Asked in person (`mine`), through my teams (`teams`), or both.
    Reviewer { mine: Option<&'a Reviewer>, teams: Vec<&'a Reviewer> },
}

/// None when I neither wrote the PR nor am asked to review it, or I declined.
fn role<'a>(pr: &'a PullRequest, me: &Me) -> Option<Role<'a>> {
    if pr.created_by.id.eq_ignore_ascii_case(me.id) {
        return Some(Role::Author);
    }
    let mine = pr.reviewers.iter().find(|r| r.id.eq_ignore_ascii_case(me.id));
    let teams: Vec<&Reviewer> = pr.reviewers.iter().filter(|r| me.is_my_team(r)).collect();
    match mine {
        Some(r) if r.has_declined => None,
        None if teams.is_empty() => None,
        _ => Some(Role::Reviewer { mine, teams }),
    }
}

/// The PRs whose threads to load: those in my list, except the `settled` ones (by head commit) with no push since.
pub fn needing_threads<'a>(prs: &'a [PullRequest], me: &Me, settled: &HashMap<i64, String>) -> Vec<&'a PullRequest> {
    prs.iter()
        .filter(|pr| role(pr, me).is_some())
        .filter(|pr| {
            let head = pr.last_merge_source_commit.as_ref().map(|c| &c.commit_id);
            head.is_none() || settled.get(&pr.pull_request_id) != head
        })
        .collect()
}

/// The PRs I wrote or am asked to review, unsorted; updates `memory` with my current votes.
fn classify_all(prs: &[PullRequest], me: &Me, memory: &mut VoteMemory, now: DateTime<Utc>) -> Vec<ReviewItem> {
    let items: Vec<ReviewItem> = prs.iter().filter_map(|pr| classify(pr, me, memory, now)).collect();
    let open: HashSet<i64> = prs.iter().map(|pr| pr.pull_request_id).collect();
    memory.retain(|id, _| open.contains(id));
    items
}

fn classify(pr: &PullRequest, me: &Me, memory: &mut VoteMemory, now: DateTime<Utc>) -> Option<ReviewItem> {
    let (mine, teams) = match role(pr, me)? {
        Role::Author => return Some(item(pr, ReviewState::Mine, false, false, now)),
        Role::Reviewer { mine, teams } => (mine, teams),
    };
    let (vote, via_group) = match mine {
        Some(r) => (r.vote, false),
        // A team's vote is the vote a teammate cast for it; `role` saw at least one of my teams.
        None => (teams[0].vote, true),
    };
    let required = mine.is_some_and(|r| r.is_required) || teams.iter().any(|g| g.is_required);
    let commit = pr.last_merge_source_commit.as_ref().map(|c| c.commit_id.clone()).unwrap_or_default();

    let state = if vote == 0 {
        memory.remove(&pr.pull_request_id);
        ReviewState::NeedsReview
    } else {
        let remembered = memory
            .entry(pr.pull_request_id)
            .or_insert_with(|| VotedAt { vote, commit: commit.clone() });
        if remembered.vote != vote {
            *remembered = VotedAt { vote, commit: commit.clone() };
        }
        if remembered.commit != commit {
            ReviewState::ReReview
        } else if vote > 0 {
            ReviewState::Approved
        } else {
            ReviewState::WaitingOnAuthor
        }
    };

    Some(item(pr, state, required, via_group, now))
}

fn item(pr: &PullRequest, state: ReviewState, required: bool, via_group: bool, now: DateTime<Utc>) -> ReviewItem {
    let facts = PrFacts {
        id: pr.pull_request_id,
        provider: Provider::Ado,
        number: pr.pull_request_id,
        title: pr.title.clone(),
        repo: pr.repository.name.clone(),
        branch: pr.branch().to_string(),
        author: pr.created_by.display_name.clone(),
        target: pr.target_ref_name.trim_start_matches("refs/heads/").to_string(),
        created_at: pr.creation_date,
        draft: pr.is_draft,
        approvals: individual_votes(pr).filter(|v| *v > 0).count() as u32,
        objections: individual_votes(pr).filter(|v| *v < 0).count() as u32,
    };
    score::review_item(facts, state, required, via_group, now)
}

fn individual_votes(pr: &PullRequest) -> impl Iterator<Item = i32> + '_ {
    pr.reviewers.iter().filter(|r| !r.is_container).map(|r| r.vote)
}

#[cfg(test)]
#[allow(non_snake_case)]
pub(crate) mod tests {
    use super::*;
    use crate::score::Tier;
    use chrono::Duration;

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

    pub const ME: &str = "me-id";
    pub const GROUP: &str = "TeamAlpha";

    pub fn now() -> DateTime<Utc> {
        "2026-11-04T12:00:00Z".parse().unwrap()
    }

    pub fn reviewer(id: &str, vote: i32, required: bool, container: bool) -> Reviewer {
        let display_name = if container { format!("[contoso]\\{id}") } else { id.to_string() };
        Reviewer { id: id.into(), display_name, vote, is_required: required, is_container: container, has_declined: false }
    }

    pub fn pr(id: i64, reviewers: Vec<Reviewer>) -> PullRequest {
        PullRequest {
            pull_request_id: id,
            title: format!("PR {id}"),
            created_by: IdentityRef { id: "author-id".into(), display_name: "Author".into() },
            creation_date: now() - Duration::hours(8),
            repository: Repository {
                id: "repo-id".into(),
                name: "Api".into(),
                project: Project { id: "project-id".into(), name: "Api".into() },
            },
            source_ref_name: "refs/heads/feature/x".into(),
            target_ref_name: "refs/heads/dev".into(),
            is_draft: false,
            reviewers,
            last_merge_source_commit: Some(Commit { commit_id: "sha1".into() }),
        }
    }

    pub fn teams() -> HashSet<String> {
        HashSet::from([GROUP.to_lowercase()])
    }

    pub fn rank_one(p: PullRequest, memory: &mut VoteMemory) -> Option<ReviewItem> {
        let t = teams();
        classify_all(&[p], &Me { id: ME, teams: &t }, memory, now()).into_iter().next()
    }

    #[test]
    fn rank_unvotedDirectReviewer_needsReview() {
        let item = rank_one(pr(1, vec![reviewer(ME, 0, false, false)]), &mut VoteMemory::new()).unwrap();
        assert_eq!(item.state, ReviewState::NeedsReview);
        assert_eq!(item.tier, Tier::Normal);
        assert!(!item.via_group);
    }

    #[test]
    fn rank_requiredOnReleaseBranchAndOld_isUrgent() {
        let mut p = pr(1, vec![reviewer(ME, 0, true, false)]);
        p.target_ref_name = "refs/heads/qa".into();
        p.creation_date = now() - Duration::days(3);
        let item = rank_one(p, &mut VoteMemory::new()).unwrap();
        assert_eq!(item.tier, Tier::Urgent);
        assert_eq!((item.target.as_str(), item.release_target), ("qa", true));
    }

    #[test]
    fn rank_intoDev_isNotAReleaseTarget() {
        let item = rank_one(pr(1, vec![reviewer(ME, 0, false, false)]), &mut VoteMemory::new()).unwrap();
        assert_eq!((item.target.as_str(), item.release_target), ("dev", false));
    }

    #[test]
    fn rank_myOwnPr_isMineAndNotActionable() {
        let mut p = pr(1, vec![reviewer("a", 10, true, false), reviewer("b", -5, false, false)]);
        p.created_by.id = ME.into();
        let mut memory = VoteMemory::new();
        let item = rank_one(p, &mut memory).unwrap();
        assert_eq!(item.state, ReviewState::Mine);
        assert!(!item.state.is_actionable());
        assert_eq!(item.tier, Tier::Low);
        assert_eq!((item.approvals, item.objections), (1, 1));
        assert!(memory.is_empty());
        assert!(!item.actionable, "an objection alone is counted only once comments are attached");
    }

    #[test]
    fn rank_notAReviewer_isSkipped() {
        assert!(rank_one(pr(1, vec![reviewer("someone", 0, true, false)]), &mut VoteMemory::new()).is_none());
    }

    #[test]
    fn rank_declined_isSkipped() {
        let mut r = reviewer(ME, 0, false, false);
        r.has_declined = true;
        assert!(rank_one(pr(1, vec![r]), &mut VoteMemory::new()).is_none());
    }

    #[test]
    fn rank_myGroupUnvoted_needsReviewViaGroup() {
        let item = rank_one(pr(1, vec![reviewer(GROUP, 0, true, true)]), &mut VoteMemory::new()).unwrap();
        assert_eq!(item.state, ReviewState::NeedsReview);
        assert!(item.via_group);
        assert!(item.required);
    }

    #[test]
    fn rank_myTeamApprovedByTeammate_isApprovedViaGroup() {
        let item = rank_one(pr(1, vec![reviewer(GROUP, 10, true, true)]), &mut VoteMemory::new()).unwrap();
        assert_eq!(item.state, ReviewState::Approved);
        assert!(item.via_group);
    }

    #[test]
    fn rank_otherTeamOnly_isSkipped() {
        assert!(rank_one(pr(1, vec![reviewer("TeamRed", 0, true, true)]), &mut VoteMemory::new()).is_none());
    }

    #[test]
    fn rank_meAndMyTeam_usesMyOwnVote() {
        let reviewers = vec![reviewer(GROUP, 10, false, true), reviewer(ME, 0, false, false)];
        let item = rank_one(pr(1, reviewers), &mut VoteMemory::new()).unwrap();
        assert_eq!(item.state, ReviewState::NeedsReview);
        assert!(!item.via_group);
    }

    #[test]
    fn rank_votes_countIndividualApprovalsAndObjectionsOnly() {
        let reviewers = vec![
            reviewer(ME, 0, false, false),
            reviewer("a", 10, false, false),
            reviewer("b", 5, false, false),
            reviewer("c", -5, false, false),
            reviewer("d", 0, false, false),
            reviewer(GROUP, 10, false, true),
        ];
        let item = rank_one(pr(1, reviewers), &mut VoteMemory::new()).unwrap();
        assert_eq!((item.approvals, item.objections), (2, 1));
    }

    #[test]
    fn rank_teamNameCaseDiffers_stillMatches() {
        let item = rank_one(pr(1, vec![reviewer("TEAMALPHA", 0, false, true)]), &mut VoteMemory::new());
        assert!(item.is_some());
    }

    #[test]
    fn rank_approvedAndUnchanged_isApprovedLowTier() {
        let item = rank_one(pr(1, vec![reviewer(ME, 10, false, false)]), &mut VoteMemory::new()).unwrap();
        assert_eq!(item.state, ReviewState::Approved);
        assert_eq!(item.tier, Tier::Low);
    }

    #[test]
    fn rank_waitingAndUnchanged_isWaitingOnAuthor() {
        let item = rank_one(pr(1, vec![reviewer(ME, -5, false, false)]), &mut VoteMemory::new()).unwrap();
        assert_eq!(item.state, ReviewState::WaitingOnAuthor);
    }

    #[test]
    fn rank_newPushAfterMyVote_isReReview() {
        let mut memory = VoteMemory::new();
        rank_one(pr(1, vec![reviewer(ME, -5, false, false)]), &mut memory);
        let mut pushed = pr(1, vec![reviewer(ME, -5, false, false)]);
        pushed.last_merge_source_commit = Some(Commit { commit_id: "sha2".into() });
        let item = rank_one(pushed, &mut memory).unwrap();
        assert_eq!(item.state, ReviewState::ReReview);
        assert_eq!(item.tier, Tier::Normal);
    }

    #[test]
    fn rank_revoteAfterPush_clearsReReview() {
        let mut memory = VoteMemory::new();
        rank_one(pr(1, vec![reviewer(ME, -5, false, false)]), &mut memory);
        let mut revoted = pr(1, vec![reviewer(ME, 10, false, false)]);
        revoted.last_merge_source_commit = Some(Commit { commit_id: "sha2".into() });
        let item = rank_one(revoted, &mut memory).unwrap();
        assert_eq!(item.state, ReviewState::Approved);
    }

    #[test]
    fn rank_voteResetToZero_forgetsMemory() {
        let mut memory = VoteMemory::new();
        rank_one(pr(1, vec![reviewer(ME, 10, false, false)]), &mut memory);
        rank_one(pr(1, vec![reviewer(ME, 0, false, false)]), &mut memory);
        assert!(memory.is_empty());
    }

    #[test]
    fn rank_closedPr_isDroppedFromMemory() {
        let mut memory = VoteMemory::from([(99, VotedAt { vote: 10, commit: "x".into() })]);
        rank_one(pr(1, vec![reviewer(ME, 0, false, false)]), &mut memory);
        assert!(!memory.contains_key(&99));
    }

    #[test]
    fn rank_draft_dropsToLowTier() {
        let mut p = pr(1, vec![reviewer(ME, 0, false, false)]);
        p.is_draft = true;
        assert_eq!(rank_one(p, &mut VoteMemory::new()).unwrap().tier, Tier::Low);
    }

    #[test]
    fn rank_multiple_scoresRequiredFirstAndApprovedLast() {
        let t = teams();
        let mut required = pr(2, vec![reviewer(ME, 0, true, false)]);
        required.creation_date = now() - Duration::hours(1);
        let prs = [pr(1, vec![reviewer(ME, 0, false, false)]), required, pr(3, vec![reviewer(ME, 10, false, false)])];
        let mut ranked = classify_all(&prs, &Me { id: ME, teams: &t }, &mut VoteMemory::new(), now());
        ranked.sort_by_key(|i| std::cmp::Reverse(i.score));
        let ids: Vec<i64> = ranked.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![2, 1, 3]);
    }

    #[test]
    fn deserialize_adoPullRequestJson_parsesOptionalFields() {
        let json = r#"{"pullRequestId":7,"title":"t","createdBy":{"id":"a","displayName":"A"},
            "creationDate":"2026-11-01T10:00:00.123Z","repository":{"id":"rid","name":"r","project":{"id":"pid","name":"p"}},
            "sourceRefName":"refs/heads/f","targetRefName":"refs/heads/dev","reviewers":[{"id":"me-id","vote":0}],"status":"active"}"#;
        let p: PullRequest = serde_json::from_str(json).unwrap();
        assert!(!p.is_draft);
        assert!(p.last_merge_source_commit.is_none());
        assert!(!p.reviewers[0].is_required);
    }

    #[test]
    fn needingThreads_nothingSettled_isExactlyThePrsRankLists() {
        let declined_with_my_team = {
            let mut me = reviewer(ME, 0, false, false);
            me.has_declined = true;
            pr(5, vec![me, reviewer(GROUP, 0, false, true)])
        };
        let mut authored = pr(6, vec![]);
        authored.created_by.id = ME.into();
        let prs = [
            pr(1, vec![reviewer(ME, 10, false, false)]),
            pr(2, vec![reviewer(GROUP, 0, false, true)]),
            pr(3, vec![reviewer("TeamRed", 0, false, true)]),
            pr(4, vec![reviewer("someone", 0, true, false)]),
            declined_with_my_team,
            authored,
        ];
        let t = teams();
        let me = Me { id: ME, teams: &t };
        let fetched: Vec<i64> = needing_threads(&prs, &me, &HashMap::new()).iter().map(|p| p.pull_request_id).collect();
        let ranked: Vec<i64> = prs.iter().filter(|p| rank_one((*p).clone(), &mut VoteMemory::new()).is_some()).map(|p| p.pull_request_id).collect();
        assert_eq!(fetched, ranked);
        assert!(!fetched.contains(&5), "declining in person outweighs my team being asked");
    }

    #[test]
    fn needingThreads_settledPr_loadsAgainOnlyAfterAPush() {
        let mut no_head = pr(3, vec![reviewer(ME, 10, false, false)]);
        no_head.last_merge_source_commit = None;
        let prs = [pr(1, vec![reviewer(ME, 10, false, false)]), pr(2, vec![reviewer(ME, 10, false, false)]), no_head];
        let settled = HashMap::from([(1, "sha1".to_string()), (2, "sha0".to_string()), (3, "sha1".to_string())]);
        let t = teams();
        let fetched: Vec<i64> = needing_threads(&prs, &Me { id: ME, teams: &t }, &settled).iter().map(|p| p.pull_request_id).collect();
        assert_eq!(fetched, [2, 3]);
    }

    #[test]
    fn digest_relatesOthersOnMyBranches_andQueuesMineWithTheirLastVote() {
        let mut other = pr(2, vec![]);
        other.source_ref_name = "refs/heads/feature/x".into();
        let mut elsewhere = pr(3, vec![]);
        elsewhere.source_ref_name = "refs/heads/feature/y".into();
        let fetched = Fetched {
            prs: vec![pr(1, vec![reviewer(ME, -5, false, false)]), other, elsewhere],
            my_id: ME.into(),
            teams: teams(),
            threads: HashMap::new(),
            urls: HashMap::from([(2, "https://ado/2".to_string())]),
        };
        let mut memory = VoteMemory::from([(1, VotedAt { vote: -5, commit: "sha0".into() })]);
        let poll = digest(fetched, &mut memory, now());
        assert_eq!(poll.items.iter().map(|i| (i.id, i.state)).collect::<Vec<_>>(), [(1, ReviewState::ReReview)]);
        assert_eq!(poll.related.iter().map(|r| (r.id, r.provider, r.url.as_str())).collect::<Vec<_>>(), [(2, Provider::Ado, "https://ado/2")]);
        let queued = &poll.queue_refs[&1];
        assert_eq!((queued.project.as_str(), queued.repo_id.as_str(), queued.agent), ("Api", "repo-id", false));
        assert_eq!((queued.source_commit.as_deref(), queued.last_voted.as_deref()), (Some("sha1"), Some("sha0")));
        assert_eq!((poll.warning, poll.queue_partial), (None, false));
    }
}
