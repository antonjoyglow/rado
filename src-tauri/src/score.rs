use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ado::{PrThreads, PullRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewState {
    NeedsReview,
    ReReview,
    WaitingOnAuthor,
    Approved,
    /// A PR I opened; shown for its review progress, never counted as waiting on me.
    Mine,
    /// A review with no activity for `STALE_AFTER_DAYS`; out of the count until something happens on it.
    Stale,
}

const STALE_AFTER_DAYS: i64 = 21;

impl ReviewState {
    pub fn is_actionable(self) -> bool {
        matches!(self, ReviewState::NeedsReview | ReviewState::ReReview)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Ado,
    #[serde(rename = "github")]
    GitHub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    Low,
    Normal,
    Urgent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewItem {
    /// Unique across sources: the Azure DevOps PR id, or the GitHub PR's global `databaseId`.
    pub id: i64,
    pub provider: Provider,
    /// The number people see: `!id` on Azure DevOps, `#number` within the GitHub repo.
    pub number: i64,
    pub title: String,
    pub repo: String,
    /// Source branch without `refs/heads/`; PRs sharing it form one aggregate row.
    pub branch: String,
    pub author: String,
    pub target: String,
    pub age_hours: i64,
    pub created_at: DateTime<Utc>,
    /// Last thread update (push, vote, comment, reviewer or status change) after opening;
    /// `None` when there was none or the PR's threads didn't load.
    pub updated_at: Option<DateTime<Utc>>,
    pub required: bool,
    pub via_group: bool,
    pub draft: bool,
    pub state: ReviewState,
    pub score: i32,
    pub tier: Tier,
    pub hidden: bool,
    /// Individual reviewers who approved (with or without suggestions).
    pub approvals: u32,
    /// Individual reviewers who rejected or are waiting for the author.
    pub objections: u32,
    pub comments: Option<CommentStats>,
    /// Waiting on me: a review to do, or my own PR with rejections or unresolved threads.
    pub actionable: bool,
}

impl ReviewItem {
    fn needs_my_work(&self) -> bool {
        self.state == ReviewState::Mine && (self.objections > 0 || self.comments.is_some_and(|c| c.open_threads > 0))
    }
}

/// `at` if it is real activity after `created`: ADO writes a reviewers thread seconds after a PR opens,
/// so anything within the first five minutes counts as part of opening it.
pub fn activity_after(at: DateTime<Utc>, created: DateTime<Utc>) -> Option<DateTime<Utc>> {
    (at > created + chrono::Duration::minutes(5)).then_some(at)
}

/// Adds comment counts and last activity, moves quiet reviews to `Stale`, then promotes my own PRs that
/// need work: they count toward {n} and rank first among mine. `was_stale` holds the previous poll's
/// `Stale` ids: a PR whose threads failed to load keeps that verdict.
pub fn attach_threads(items: &mut [ReviewItem], threads: &HashMap<i64, PrThreads>, was_stale: &HashSet<i64>, now: DateTime<Utc>) {
    for item in items.iter_mut() {
        let loaded = threads.get(&item.id);
        item.comments = loaded.map(|t| t.comments);
        item.updated_at = loaded.and_then(|t| t.last_activity).and_then(|at| activity_after(at, item.created_at));
        let quiet = match loaded {
            Some(_) => now - item.updated_at.unwrap_or(item.created_at) >= chrono::Duration::days(STALE_AFTER_DAYS),
            None => was_stale.contains(&item.id),
        };
        if item.state.is_actionable() && quiet {
            item.state = ReviewState::Stale;
            item.actionable = false;
            item.tier = Tier::Low;
        }
        if item.needs_my_work() {
            item.actionable = true;
            item.score += 40;
            item.tier = Tier::Normal;
        }
    }
    items.sort_by(|a, b| b.score.cmp(&a.score).then(b.age_hours.cmp(&a.age_hours)));
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentStats {
    pub total: u32,
    pub open_threads: u32,
}

/// The source commit seen when I last cast a non-zero vote, keyed by PR id.
pub type VoteMemory = HashMap<i64, VotedAt>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VotedAt {
    pub vote: i32,
    pub commit: String,
}

pub struct Me<'a> {
    pub id: &'a str,
    /// Lower-case team names; a team reviewer shows as `[scope]\Name`.
    pub teams: &'a HashSet<String>,
}

impl Me<'_> {
    fn is_my_team(&self, reviewer: &crate::ado::Reviewer) -> bool {
        let name = reviewer.display_name.rsplit('\\').next().unwrap_or_default();
        reviewer.is_container && self.teams.contains(&name.to_lowercase())
    }
}

/// Target branches (without `refs/heads/`) whose PRs rank higher.
pub fn is_release_target(target: &str) -> bool {
    ["master", "main", "qa", "release"].contains(&target)
}

/// Ranks the PRs I am asked to review and updates `memory` with my current votes.
pub fn rank(prs: &[PullRequest], me: &Me, memory: &mut VoteMemory, now: DateTime<Utc>) -> Vec<ReviewItem> {
    let mut items: Vec<ReviewItem> = prs.iter().filter_map(|pr| classify(pr, me, memory, now)).collect();
    let open: HashSet<i64> = prs.iter().map(|pr| pr.pull_request_id).collect();
    memory.retain(|id, _| open.contains(id));
    items.sort_by(|a, b| b.score.cmp(&a.score).then(b.age_hours.cmp(&a.age_hours)));
    items
}

fn classify(pr: &PullRequest, me: &Me, memory: &mut VoteMemory, now: DateTime<Utc>) -> Option<ReviewItem> {
    if pr.created_by.id.eq_ignore_ascii_case(me.id) {
        return Some(item(pr, ReviewState::Mine, false, false, now));
    }
    let mine = pr.reviewers.iter().find(|r| r.id.eq_ignore_ascii_case(me.id));
    let my_groups: Vec<_> = pr.reviewers.iter().filter(|r| me.is_my_team(r)).collect();

    let (vote, via_group) = match (mine, my_groups.first()) {
        (Some(r), _) if r.has_declined => return None,
        (Some(r), _) => (r.vote, false),
        // A team's vote is the vote a teammate cast for it.
        (None, Some(team)) => (team.vote, true),
        (None, None) => return None,
    };
    let required = mine.is_some_and(|r| r.is_required) || my_groups.iter().any(|g| g.is_required);
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
    review_item(facts, state, required, via_group, now)
}

/// What a source knows about a PR before Rado ranks it.
pub struct PrFacts {
    pub id: i64,
    pub provider: Provider,
    pub number: i64,
    pub title: String,
    pub repo: String,
    pub branch: String,
    pub author: String,
    /// Target branch without `refs/heads/`.
    pub target: String,
    pub created_at: DateTime<Utc>,
    pub draft: bool,
    pub approvals: u32,
    pub objections: u32,
}

/// A ranked item for a PR in `state`; comments and last activity come later from `attach_threads`.
pub fn review_item(pr: PrFacts, state: ReviewState, required: bool, via_group: bool, now: DateTime<Utc>) -> ReviewItem {
    let age_hours = (now - pr.created_at).num_hours().max(0);
    let score = score(state, required, via_group, pr.draft, is_release_target(&pr.target), age_hours);
    ReviewItem {
        id: pr.id,
        provider: pr.provider,
        number: pr.number,
        title: pr.title,
        repo: pr.repo,
        branch: pr.branch,
        author: pr.author,
        target: pr.target,
        age_hours,
        created_at: pr.created_at,
        updated_at: None,
        required,
        via_group,
        draft: pr.draft,
        state,
        score,
        tier: tier(state, score),
        hidden: false,
        approvals: pr.approvals,
        objections: pr.objections,
        comments: None,
        actionable: state.is_actionable(),
    }
}

fn individual_votes(pr: &PullRequest) -> impl Iterator<Item = i32> + '_ {
    pr.reviewers.iter().filter(|r| !r.is_container).map(|r| r.vote)
}

fn score(state: ReviewState, required: bool, via_group: bool, draft: bool, release_target: bool, age_hours: i64) -> i32 {
    let base = match state {
        ReviewState::NeedsReview => 50,
        ReviewState::ReReview => 45,
        ReviewState::WaitingOnAuthor => 10,
        ReviewState::Approved | ReviewState::Mine | ReviewState::Stale => 0,
    };
    // One point per 4 hours waiting, capped at five days.
    let age = (age_hours / 4).min(30) as i32;
    base + age
        + if required { 25 } else { 0 }
        + if release_target { 15 } else { 0 }
        - if via_group { 10 } else { 0 }
        - if draft { 40 } else { 0 }
}

fn tier(state: ReviewState, score: i32) -> Tier {
    match (state.is_actionable(), score) {
        (true, s) if s >= 85 => Tier::Urgent,
        (true, s) if s >= 40 => Tier::Normal,
        _ => Tier::Low,
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::ado::{Commit, IdentityRef, Project, Repository, Reviewer};
    use chrono::Duration;

    const ME: &str = "me-id";
    const GROUP: &str = "TeamAlpha";

    fn now() -> DateTime<Utc> {
        "2026-11-04T12:00:00Z".parse().unwrap()
    }

    fn reviewer(id: &str, vote: i32, required: bool, container: bool) -> Reviewer {
        let display_name = if container { format!("[contoso]\\{id}") } else { id.to_string() };
        Reviewer { id: id.into(), display_name, vote, is_required: required, is_container: container, has_declined: false }
    }

    fn pr(id: i64, reviewers: Vec<Reviewer>) -> PullRequest {
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

    fn teams() -> HashSet<String> {
        HashSet::from([GROUP.to_lowercase()])
    }

    fn rank_one(p: PullRequest, memory: &mut VoteMemory) -> Option<ReviewItem> {
        let t = teams();
        rank(&[p], &Me { id: ME, teams: &t }, memory, now()).into_iter().next()
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
        assert_eq!(item.target, "qa");
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

    fn mine(id: i64, objections: u32) -> ReviewItem {
        let mut p = pr(id, (0..objections).map(|i| reviewer(&format!("r{i}"), -10, false, false)).collect());
        p.created_by.id = ME.into();
        rank_one(p, &mut VoteMemory::new()).unwrap()
    }

    fn threads(total: u32, open_threads: u32, last_activity: Option<DateTime<Utc>>) -> PrThreads {
        PrThreads { comments: CommentStats { total, open_threads }, last_activity }
    }

    #[test]
    fn attachThreads_myPrWithRejection_becomesActionable() {
        let mut items = vec![mine(1, 1)];
        attach_threads(&mut items, &HashMap::new(), &HashSet::new(), now());
        assert!(items[0].actionable);
        assert_eq!(items[0].tier, Tier::Normal);
    }

    #[test]
    fn attachThreads_myPrWithOpenThreads_becomesActionableAndRanksFirst() {
        let mut items = vec![mine(1, 0), mine(2, 0)];
        let loaded = HashMap::from([(2, threads(3, 1, None)), (1, threads(5, 0, None))]);
        attach_threads(&mut items, &loaded, &HashSet::new(), now());
        assert_eq!(items.iter().map(|i| (i.id, i.actionable)).collect::<Vec<_>>(), [(2, true), (1, false)]);
    }

    #[test]
    fn attachThreads_reviewItem_keepsItsOwnActionability() {
        let mut items = rank(&[pr(1, vec![reviewer(ME, 10, false, false)])], &Me { id: ME, teams: &teams() }, &mut VoteMemory::new(), now());
        attach_threads(&mut items, &HashMap::from([(1, threads(2, 2, None))]), &HashSet::new(), now());
        assert!(!items[0].actionable, "open threads on someone else's PR are not my work");
    }

    #[test]
    fn attachThreads_lastActivity_becomesUpdatedAt() {
        let mut items = vec![mine(1, 0)];
        let pushed = now() - Duration::hours(2);
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, Some(pushed)))]), &HashSet::new(), now());
        assert_eq!(items[0].updated_at, Some(pushed));
    }

    #[test]
    fn attachThreads_noActivityAfterOpening_hasNoUpdate() {
        let mut items = vec![mine(1, 0), mine(2, 0), mine(3, 0)];
        let created = items[0].created_at;
        let loaded = HashMap::from([
            (1, threads(0, 0, None)),
            (2, threads(0, 0, Some(created + Duration::seconds(7)))),
            (3, threads(0, 0, Some(created - Duration::days(1)))),
        ]);
        attach_threads(&mut items, &loaded, &HashSet::new(), now());
        assert!(items.iter().all(|i| i.updated_at.is_none()));
    }

    #[test]
    fn attachThreads_threadsFailedToLoad_leavesUpdatedAtUnknown() {
        let mut items = vec![mine(1, 0)];
        attach_threads(&mut items, &HashMap::new(), &HashSet::new(), now());
        assert_eq!(items[0].updated_at, None);
    }

    fn review_opened_days_ago(days: i64) -> ReviewItem {
        let mut p = pr(1, vec![reviewer(ME, 0, false, false)]);
        p.creation_date = now() - Duration::days(days);
        rank_one(p, &mut VoteMemory::new()).unwrap()
    }

    #[test]
    fn attachThreads_reviewQuietFor21Days_isStaleAndNotActionable() {
        let mut items = vec![review_opened_days_ago(30)];
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, Some(now() - Duration::days(21))))]), &HashSet::new(), now());
        assert_eq!((items[0].state, items[0].actionable, items[0].tier), (ReviewState::Stale, false, Tier::Low));
    }

    #[test]
    fn attachThreads_reviewWithNoActivitySinceOpening21DaysAgo_isStale() {
        let mut items = vec![review_opened_days_ago(21)];
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, None))]), &HashSet::new(), now());
        assert_eq!(items[0].state, ReviewState::Stale);
    }

    #[test]
    fn attachThreads_oldReviewWithRecentActivity_staysActionable() {
        let mut items = vec![review_opened_days_ago(60)];
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, Some(now() - Duration::days(20))))]), &HashSet::new(), now());
        assert_eq!((items[0].state, items[0].actionable), (ReviewState::NeedsReview, true));
    }

    #[test]
    fn attachThreads_quietReviewWhoseThreadsFailedToLoad_staysStale() {
        let mut items = vec![review_opened_days_ago(60)];
        attach_threads(&mut items, &HashMap::new(), &HashSet::from([1]), now());
        assert_eq!((items[0].state, items[0].actionable), (ReviewState::Stale, false));
    }

    #[test]
    fn attachThreads_activeReviewWhoseThreadsFailedToLoad_staysActionable() {
        let mut items = vec![review_opened_days_ago(60)];
        attach_threads(&mut items, &HashMap::new(), &HashSet::new(), now());
        assert_eq!(items[0].state, ReviewState::NeedsReview);
    }

    #[test]
    fn attachThreads_myQuietPr_isNeverStale() {
        let mut p = pr(1, vec![]);
        p.created_by.id = ME.into();
        p.creation_date = now() - Duration::days(60);
        let mut items = vec![rank_one(p, &mut VoteMemory::new()).unwrap()];
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, None))]), &HashSet::new(), now());
        assert_eq!(items[0].state, ReviewState::Mine);
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
    fn rank_multiple_sortedByScoreDescending() {
        let t = teams();
        let mut required = pr(2, vec![reviewer(ME, 0, true, false)]);
        required.creation_date = now() - Duration::hours(1);
        let prs = [pr(1, vec![reviewer(ME, 0, false, false)]), required, pr(3, vec![reviewer(ME, 10, false, false)])];
        let ids: Vec<i64> = rank(&prs, &Me { id: ME, teams: &t }, &mut VoteMemory::new(), now())
            .iter()
            .map(|i| i.id)
            .collect();
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
}
