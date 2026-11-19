use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};


/// Where a PR stands for me. `mine`: a PR I opened, shown for its review progress and never counted as waiting on me.
/// `stale`: a review with no activity for `STALE_AFTER_DAYS`, out of the count until something happens on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum ReviewState {
    NeedsReview,
    ReReview,
    WaitingOnAuthor,
    Approved,
    Mine,
    Stale,
}

const STALE_AFTER_DAYS: i64 = 21;

impl ReviewState {
    pub fn is_actionable(self) -> bool {
        matches!(self, ReviewState::NeedsReview | ReviewState::ReReview)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Ado,
    #[serde(rename = "github")]
    GitHub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    Low,
    Normal,
    Urgent,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(rename = "RowSummary")]
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
    /// Merges into a release branch (`is_release_target`); the list names the target only then.
    pub release_target: bool,
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
    /// Waiting on me: a review to do, or my own PR with rejections or unresolved threads. Never true for a hidden row.
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
/// need work: they count toward {n} and score above my other PRs. A PR whose threads weren't loaded this poll
/// (skipped, or failed) keeps the comments and last activity of its item in `previous`; with neither, it isn't judged quiet.
pub fn attach_threads(items: &mut [ReviewItem], threads: &HashMap<i64, PrThreads>, previous: &HashMap<i64, &ReviewItem>, now: DateTime<Utc>) {
    for item in items.iter_mut() {
        match threads.get(&item.id) {
            Some(loaded) => {
                item.comments = Some(loaded.comments);
                item.updated_at = loaded.last_activity.and_then(|at| activity_after(at, item.created_at));
            }
            None => {
                let before = previous.get(&item.id);
                item.comments = before.and_then(|b| b.comments);
                item.updated_at = before.and_then(|b| b.updated_at);
            }
        }
        // Comments are known exactly when threads loaded, this poll or an earlier one.
        let quiet = item.comments.is_some() && now - item.updated_at.unwrap_or(item.created_at) >= chrono::Duration::days(STALE_AFTER_DAYS);
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
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct CommentStats {
    pub total: u32,
    pub open_threads: u32,
}

/// What one threads call tells us about a PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrThreads {
    pub comments: CommentStats,
    pub last_activity: Option<DateTime<Utc>>,
}

/// The source commit seen when I last cast a non-zero vote, keyed by PR id.
pub type VoteMemory = HashMap<i64, VotedAt>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VotedAt {
    pub vote: i32,
    pub commit: String,
}

/// Target branches (without `refs/heads/`) whose PRs rank higher.
pub fn is_release_target(target: &str) -> bool {
    ["master", "main", "qa", "release"].contains(&target)
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
    let release_target = is_release_target(&pr.target);
    let score = score(state, required, via_group, pr.draft, release_target, age_hours);
    ReviewItem {
        id: pr.id,
        provider: pr.provider,
        number: pr.number,
        title: pr.title,
        repo: pr.repo,
        branch: pr.branch,
        author: pr.author,
        target: pr.target,
        release_target,
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
    use crate::ado::tests::{ME, now, pr, rank_one, reviewer};
    use chrono::Duration;

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
        attach_threads(&mut items, &HashMap::new(), &HashMap::new(), now());
        assert!(items[0].actionable);
        assert_eq!(items[0].tier, Tier::Normal);
    }

    #[test]
    fn attachThreads_myPrWithOpenThreads_becomesActionableAndScoresAboveMyOthers() {
        let mut items = vec![mine(1, 0), mine(2, 0)];
        let loaded = HashMap::from([(2, threads(3, 1, None)), (1, threads(5, 0, None))]);
        attach_threads(&mut items, &loaded, &HashMap::new(), now());
        assert_eq!(items.iter().map(|i| (i.id, i.actionable)).collect::<Vec<_>>(), [(1, false), (2, true)]);
        assert!(items[1].score > items[0].score);
    }

    #[test]
    fn attachThreads_reviewItem_keepsItsOwnActionability() {
        let mut items = vec![rank_one(pr(1, vec![reviewer(ME, 10, false, false)]), &mut VoteMemory::new()).unwrap()];
        attach_threads(&mut items, &HashMap::from([(1, threads(2, 2, None))]), &HashMap::new(), now());
        assert!(!items[0].actionable, "open threads on someone else's PR are not my work");
    }

    #[test]
    fn attachThreads_lastActivity_becomesUpdatedAt() {
        let mut items = vec![mine(1, 0)];
        let pushed = now() - Duration::hours(2);
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, Some(pushed)))]), &HashMap::new(), now());
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
        attach_threads(&mut items, &loaded, &HashMap::new(), now());
        assert!(items.iter().all(|i| i.updated_at.is_none()));
    }

    #[test]
    fn attachThreads_threadsNotLoadedOnAFirstSighting_leavesCommentsAndActivityUnknown() {
        let mut items = vec![mine(1, 0)];
        attach_threads(&mut items, &HashMap::new(), &HashMap::new(), now());
        assert_eq!((items[0].comments, items[0].updated_at), (None, None));
    }

    fn review_opened_days_ago(days: i64) -> ReviewItem {
        let mut p = pr(1, vec![reviewer(ME, 0, false, false)]);
        p.creation_date = now() - Duration::days(days);
        rank_one(p, &mut VoteMemory::new()).unwrap()
    }

    #[test]
    fn attachThreads_reviewQuietFor21Days_isStaleAndNotActionable() {
        let mut items = vec![review_opened_days_ago(30)];
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, Some(now() - Duration::days(21))))]), &HashMap::new(), now());
        assert_eq!((items[0].state, items[0].actionable, items[0].tier), (ReviewState::Stale, false, Tier::Low));
    }

    #[test]
    fn attachThreads_reviewWithNoActivitySinceOpening21DaysAgo_isStale() {
        let mut items = vec![review_opened_days_ago(21)];
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, None))]), &HashMap::new(), now());
        assert_eq!(items[0].state, ReviewState::Stale);
    }

    #[test]
    fn attachThreads_oldReviewWithRecentActivity_staysActionable() {
        let mut items = vec![review_opened_days_ago(60)];
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, Some(now() - Duration::days(20))))]), &HashMap::new(), now());
        assert_eq!((items[0].state, items[0].actionable), (ReviewState::NeedsReview, true));
    }

    #[test]
    fn attachThreads_threadsNotLoadedAndQuietSinceTheirLastLoad_isStale() {
        let mut items = vec![review_opened_days_ago(60)];
        let mut before = items[0].clone();
        before.comments = Some(CommentStats::default());
        before.updated_at = Some(now() - Duration::days(22));
        attach_threads(&mut items, &HashMap::new(), &HashMap::from([(1, &before)]), now());
        assert_eq!((items[0].state, items[0].actionable), (ReviewState::Stale, false));
    }

    #[test]
    fn attachThreads_threadsNotLoaded_keepsThePreviousCommentsAndActivity() {
        let mut items = vec![mine(1, 0)];
        let mut before = items[0].clone();
        before.comments = Some(CommentStats { total: 4, open_threads: 1 });
        before.updated_at = Some(now() - Duration::hours(3));
        attach_threads(&mut items, &HashMap::new(), &HashMap::from([(1, &before)]), now());
        assert_eq!((items[0].comments, items[0].updated_at), (before.comments, before.updated_at));
        assert!(items[0].actionable, "a carried open thread on my PR is still my work");
    }

    #[test]
    fn attachThreads_activeReviewWhoseThreadsFailedToLoad_staysActionable() {
        let mut items = vec![review_opened_days_ago(60)];
        attach_threads(&mut items, &HashMap::new(), &HashMap::new(), now());
        assert_eq!(items[0].state, ReviewState::NeedsReview);
    }

    #[test]
    fn attachThreads_myQuietPr_isNeverStale() {
        let mut p = pr(1, vec![]);
        p.created_by.id = ME.into();
        p.creation_date = now() - Duration::days(60);
        let mut items = vec![rank_one(p, &mut VoteMemory::new()).unwrap()];
        attach_threads(&mut items, &HashMap::from([(1, threads(0, 0, None))]), &HashMap::new(), now());
        assert_eq!(items[0].state, ReviewState::Mine);
    }
}
