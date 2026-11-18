use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::queue::{AZURE_DEV_OPS, Branch, GITHUB, Queue, QueuedPr};
use crate::score::{Provider, ReviewItem, ReviewState, Tier};
use crate::source::QueueRef;

pub struct Context<'a> {
    pub organization: &'a str,
    pub poll_minutes: u32,
    pub now: DateTime<Utc>,
}

/// One poll's queue refs per fresh source. A source that is off or failed has none, and its PRs are left out.
pub struct Sources {
    pub refs: HashMap<Provider, HashMap<i64, QueueRef>>,
    pub missing: Vec<String>,
}

/// A source's name in the queue file.
pub fn source_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Ado => AZURE_DEV_OPS,
        Provider::GitHub => GITHUB,
    }
}

/// The reviews waiting on me, grouped by branch in rank order, with what an agent needs to call the provider.
/// Leaves out hidden PRs, my own PRs, PRs I approved or am waiting on, and PRs of a source that wasn't fresh.
pub fn queue(items: &[ReviewItem], sources: &Sources, urls: &HashMap<i64, String>, ctx: &Context) -> Queue {
    let mut branches: Vec<Branch> = Vec::new();
    for item in items.iter().filter(|i| i.state.is_actionable() && !i.hidden) {
        let Some(r) = sources.refs.get(&item.provider).and_then(|refs| refs.get(&item.id)) else { continue };
        let provider = source_name(item.provider);
        let re_review = item.state == ReviewState::ReReview;
        let entry = QueuedPr {
            provider: provider.into(),
            pr_id: item.id,
            number: item.number,
            project: r.project.clone(),
            repo: item.repo.clone(),
            repo_id: r.repo_id.clone(),
            agent_authored: r.agent,
            url: urls.get(&item.id).cloned().unwrap_or_default(),
            title: item.title.clone(),
            author: item.author.clone(),
            target: item.target.clone(),
            state: state_name(item.state).into(),
            tier: tier_name(item.tier).into(),
            score: item.score,
            age_hours: item.age_hours,
            draft: item.draft,
            required: item.required,
            // A vote dismissed without a push leaves nothing new to diff against, so the whole PR is the review.
            last_voted_commit: r.last_voted.clone().filter(|v| re_review && Some(v) != r.source_commit.as_ref()),
            source_commit: r.source_commit.clone(),
        };
        // The same branch name on two providers is two changes, and on GitHub under two owners too (`patch-1`).
        let same_change = |first: &QueuedPr| first.provider == provider && (provider == AZURE_DEV_OPS || first.project == entry.project);
        match branches.iter_mut().find(|b| b.branch == item.branch && same_change(&b.prs[0])) {
            Some(b) => b.prs.push(entry),
            None => branches.push(Branch { branch: item.branch.clone(), prs: vec![entry] }),
        }
    }
    Queue {
        refreshed: ctx.now,
        organization: ctx.organization.to_string(),
        poll_minutes: ctx.poll_minutes,
        missing_sources: sources.missing.clone(),
        branches,
    }
}

fn state_name(state: ReviewState) -> &'static str {
    match state {
        ReviewState::NeedsReview => "needsReview",
        ReviewState::ReReview => "reReview",
        ReviewState::WaitingOnAuthor => "waitingOnAuthor",
        ReviewState::Approved => "approved",
        ReviewState::Mine => "mine",
        ReviewState::Stale => "stale",
    }
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Urgent => "urgent",
        Tier::Normal => "normal",
        Tier::Low => "low",
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-11-10T09:00:00Z".parse().unwrap()
    }

    fn item(id: i64, branch: &str, state: ReviewState, score: i32) -> ReviewItem {
        ReviewItem {
            id,
            provider: Provider::Ado,
            number: id,
            title: format!("PR {id}"),
            repo: format!("Repo{id}"),
            branch: branch.into(),
            author: "Author".into(),
            target: "dev".into(),
            release_target: false,
            age_hours: 8,
            created_at: now(),
            updated_at: None,
            required: false,
            via_group: false,
            draft: false,
            state,
            score,
            tier: Tier::Normal,
            hidden: false,
            approvals: 0,
            objections: 0,
            comments: None,
            actionable: state.is_actionable(),
        }
    }

    fn ctx() -> Context<'static> {
        Context { organization: "org", poll_minutes: 3, now: now() }
    }

    /// An Azure DevOps PR's queue ref, at `commit`, never voted on.
    fn ado_ref(id: i64, commit: &str) -> QueueRef {
        QueueRef { project: "Contoso".into(), repo_id: format!("repo-{id}"), agent: false, source_commit: Some(commit.into()), last_voted: None }
    }

    fn github_ref(agent: bool) -> QueueRef {
        QueueRef { project: "acme".into(), repo_id: "acme/web".into(), agent, source_commit: Some("h2".into()), last_voted: Some("h1".into()) }
    }

    fn sources(ado: Option<Vec<(i64, QueueRef)>>, github: Option<Vec<(i64, QueueRef)>>) -> Sources {
        let refs = [(Provider::Ado, ado), (Provider::GitHub, github)]
            .into_iter()
            .filter_map(|(provider, refs)| Some((provider, refs?.into_iter().collect())))
            .collect();
        Sources { refs, missing: vec![] }
    }

    fn on_github(id: i64, branch: &str, state: ReviewState) -> ReviewItem {
        ReviewItem { provider: Provider::GitHub, number: 12, repo: "web".into(), ..item(id, branch, state, 50) }
    }

    #[test]
    fn queue_gitHubPr_carriesOwnerRepoNumberAndHead() {
        let refs = vec![(9_000_001, github_ref(true)), (9_000_002, github_ref(false))];
        let items = vec![on_github(9_000_001, "feat", ReviewState::NeedsReview), on_github(9_000_002, "feat", ReviewState::ReReview)];

        let q = queue(&items, &sources(None, Some(refs)), &HashMap::new(), &ctx());

        let (first, second) = (&q.branches[0].prs[0], &q.branches[0].prs[1]);
        assert_eq!((first.provider.as_str(), first.project.as_str(), first.repo_id.as_str(), first.number), ("gitHub", "acme", "acme/web", 12));
        assert_eq!((first.source_commit.as_deref(), first.agent_authored, first.last_voted_commit.as_deref()), (Some("h2"), true, None));
        assert_eq!((second.last_voted_commit.as_deref(), second.agent_authored), (Some("h1"), false));
    }

    #[test]
    fn queue_sourceNotFresh_isLeftOutAndNamed() {
        let items = vec![item(1, "a", ReviewState::NeedsReview, 60), on_github(9_000_001, "b", ReviewState::NeedsReview)];
        let sources = Sources { missing: vec!["gitHub".into()], ..sources(Some(vec![(1, ado_ref(1, "c1"))]), None) };

        let q = queue(&items, &sources, &HashMap::new(), &ctx());

        assert_eq!(q.branches.iter().map(|b| b.branch.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(q.missing_sources, ["gitHub"]);
    }

    #[test]
    fn queue_refOfTheOtherSource_isNotUsed() {
        let items = vec![item(7, "a", ReviewState::NeedsReview, 60)];
        let q = queue(&items, &sources(None, Some(vec![(7, github_ref(false))])), &HashMap::new(), &ctx());
        assert!(q.branches.is_empty(), "an Azure DevOps PR kept from a failed poll must not pick up a GitHub PR's ref with its id");
    }

    #[test]
    fn queue_sameBranchOnBothProviders_isTwoBranches() {
        let items = vec![item(1, "feat", ReviewState::NeedsReview, 60), on_github(9_000_001, "feat", ReviewState::NeedsReview)];
        let sources = sources(Some(vec![(1, ado_ref(1, "c1"))]), Some(vec![(9_000_001, github_ref(false))]));

        let q = queue(&items, &sources, &HashMap::new(), &ctx());

        assert_eq!(q.branches.len(), 2);
    }

    #[test]
    fn queue_gitHubSameBranchUnderTwoOwners_isTwoBranches() {
        let other = QueueRef { project: "other".into(), repo_id: "other/web".into(), ..github_ref(false) };
        let refs = vec![(9_000_001, github_ref(false)), (9_000_002, other)];
        let items = vec![on_github(9_000_001, "patch-1", ReviewState::NeedsReview), on_github(9_000_002, "patch-1", ReviewState::NeedsReview)];

        assert_eq!(queue(&items, &sources(None, Some(refs)), &HashMap::new(), &ctx()).branches.len(), 2);
    }

    #[test]
    fn queue_voteDismissedWithoutAPush_hasNoLastVotedCommit() {
        let dismissed = QueueRef { last_voted: Some("h2".into()), ..github_ref(false) };

        let q = queue(&[on_github(9_000_001, "feat", ReviewState::ReReview)], &sources(None, Some(vec![(9_000_001, dismissed)])), &HashMap::new(), &ctx());

        assert_eq!(q.branches[0].prs[0].last_voted_commit, None);
    }

    #[test]
    fn queue_keepsOnlyReviewsToDo_groupedByBranchInRankOrder() {
        let refs = [(1, "c1"), (2, "c2"), (3, "c3"), (4, "c4"), (5, "c5"), (6, "c6")].map(|(id, commit)| (id, ado_ref(id, commit)));
        let mut hidden = item(5, "d", ReviewState::NeedsReview, 70);
        hidden.hidden = true;
        let items = vec![
            item(2, "b", ReviewState::NeedsReview, 80),
            hidden,
            item(1, "a", ReviewState::NeedsReview, 60),
            item(3, "a", ReviewState::ReReview, 55),
            item(4, "c", ReviewState::Approved, 0),
            item(6, "e", ReviewState::Mine, 40),
        ];
        let urls = HashMap::from([(1, "https://pr/1".to_string())]);

        let q = queue(&items, &sources(Some(refs.to_vec()), None), &urls, &ctx());

        let shape: Vec<(&str, Vec<i64>)> = q.branches.iter().map(|b| (b.branch.as_str(), b.prs.iter().map(|p| p.pr_id).collect())).collect();
        assert_eq!(shape, vec![("b", vec![2]), ("a", vec![1, 3])]);
        let first = &q.branches[1].prs[0];
        assert_eq!((first.project.as_str(), first.repo_id.as_str(), first.url.as_str()), ("Contoso", "repo-1", "https://pr/1"));
        assert_eq!(first.source_commit.as_deref(), Some("c1"));
        assert_eq!((q.organization.as_str(), q.poll_minutes, q.refreshed), ("org", 3, now()));
    }

    #[test]
    fn queue_reReview_carriesTheCommitOfMyLastVote() {
        let voted_before_push = QueueRef { last_voted: Some("old".into()), ..ado_ref(1, "new") };
        let voted_on_head = QueueRef { last_voted: Some("c2".into()), ..ado_ref(2, "c2") };
        let items = vec![item(1, "a", ReviewState::ReReview, 60), item(2, "b", ReviewState::NeedsReview, 50)];

        let q = queue(&items, &sources(Some(vec![(1, voted_before_push), (2, voted_on_head)]), None), &HashMap::new(), &ctx());

        assert_eq!(q.branches[0].prs[0].last_voted_commit.as_deref(), Some("old"));
        assert_eq!(q.branches[0].prs[0].state, "reReview");
        assert_eq!(q.branches[1].prs[0].last_voted_commit, None);
    }
}
