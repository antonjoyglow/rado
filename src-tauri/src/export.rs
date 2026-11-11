use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::ado::PullRequest;
use crate::queue::{Branch, Queue, QueuedPr};
use crate::score::{Provider, ReviewItem, ReviewState, Tier, VoteMemory};

pub struct Context<'a> {
    pub organization: &'a str,
    pub poll_minutes: u32,
    pub now: DateTime<Utc>,
}

/// The reviews waiting on me, grouped by branch in rank order, with what an agent needs to call Azure DevOps.
/// Leaves out hidden PRs, my own PRs, PRs I approved or am waiting on, and GitHub PRs (not queued yet).
pub fn queue(items: &[ReviewItem], prs: &[PullRequest], memory: &VoteMemory, urls: &HashMap<i64, String>, ctx: &Context) -> Queue {
    let by_id: HashMap<i64, &PullRequest> = prs.iter().map(|pr| (pr.pull_request_id, pr)).collect();
    let mut branches: Vec<Branch> = Vec::new();
    for item in items.iter().filter(|i| i.provider == Provider::Ado && i.state.is_actionable() && !i.hidden) {
        let Some(pr) = by_id.get(&item.id) else { continue };
        let entry = QueuedPr {
            pr_id: item.id,
            project: pr.repository.project.name.clone(),
            repo: item.repo.clone(),
            repo_id: pr.repository.id.clone(),
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
            source_commit: pr.last_merge_source_commit.as_ref().map(|c| c.commit_id.clone()),
            last_voted_commit: (item.state == ReviewState::ReReview)
                .then(|| memory.get(&item.id).map(|v| v.commit.clone()))
                .flatten(),
        };
        match branches.iter_mut().find(|b| b.branch == item.branch) {
            Some(b) => b.prs.push(entry),
            None => branches.push(Branch { branch: item.branch.clone(), prs: vec![entry] }),
        }
    }
    Queue { refreshed: ctx.now, organization: ctx.organization.to_string(), poll_minutes: ctx.poll_minutes, branches }
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
    use crate::ado::{Commit, IdentityRef, Project, Repository};
    use crate::score::VotedAt;

    fn now() -> DateTime<Utc> {
        "2026-11-10T09:00:00Z".parse().unwrap()
    }

    fn pr(id: i64, branch: &str, commit: &str) -> PullRequest {
        PullRequest {
            pull_request_id: id,
            title: format!("PR {id}"),
            created_by: IdentityRef { id: "author-id".into(), display_name: "Author".into() },
            creation_date: now(),
            repository: Repository {
                id: format!("repo-{id}"),
                name: format!("Repo{id}"),
                project: Project { id: "project-id".into(), name: "Contoso".into() },
            },
            source_ref_name: format!("refs/heads/{branch}"),
            target_ref_name: "refs/heads/dev".into(),
            is_draft: false,
            reviewers: vec![],
            last_merge_source_commit: Some(Commit { commit_id: commit.into() }),
        }
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

    #[test]
    fn queue_keepsOnlyReviewsToDo_groupedByBranchInRankOrder() {
        let prs = vec![pr(1, "a", "c1"), pr(2, "b", "c2"), pr(3, "a", "c3"), pr(4, "c", "c4"), pr(5, "d", "c5"), pr(6, "e", "c6")];
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

        let q = queue(&items, &prs, &VoteMemory::new(), &urls, &ctx());

        let shape: Vec<(&str, Vec<i64>)> = q.branches.iter().map(|b| (b.branch.as_str(), b.prs.iter().map(|p| p.pr_id).collect())).collect();
        assert_eq!(shape, vec![("b", vec![2]), ("a", vec![1, 3])]);
        let first = &q.branches[1].prs[0];
        assert_eq!((first.project.as_str(), first.repo_id.as_str(), first.url.as_str()), ("Contoso", "repo-1", "https://pr/1"));
        assert_eq!(first.source_commit.as_deref(), Some("c1"));
        assert_eq!((q.organization.as_str(), q.poll_minutes, q.refreshed), ("org", 3, now()));
    }

    #[test]
    fn queue_reReview_carriesTheCommitOfMyLastVote() {
        let prs = vec![pr(1, "a", "new"), pr(2, "b", "c2")];
        let items = vec![item(1, "a", ReviewState::ReReview, 60), item(2, "b", ReviewState::NeedsReview, 50)];
        let memory = VoteMemory::from([
            (1, VotedAt { vote: -5, commit: "old".into() }),
            (2, VotedAt { vote: 10, commit: "c2".into() }),
        ]);

        let q = queue(&items, &prs, &memory, &HashMap::new(), &ctx());

        assert_eq!(q.branches[0].prs[0].last_voted_commit.as_deref(), Some("old"));
        assert_eq!(q.branches[0].prs[0].state, "reReview");
        assert_eq!(q.branches[1].prs[0].last_voted_commit, None);
    }
}
