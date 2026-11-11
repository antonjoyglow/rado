//! GitHub (github.com) pull requests: one GraphQL call per poll returns every PR waiting on my review,
//! the ones I reviewed, and my own, with what Rado shows for each.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::ado::PrThreads;
use crate::score::{self, CommentStats, PrFacts, Provider, ReviewItem, ReviewState};

const ENDPOINT: &str = "https://api.github.com/graphql";

/// Three searches: requested covers requests to me and to my teams; reviewed catches new pushes after my review.
const QUERY: &str = r#"
fragment pr on PullRequest {
  databaseId number title url isDraft createdAt updatedAt headRefName baseRefName headRefOid
  author { login }
  repository { name }
  reviewRequests(first: 20) { nodes { requestedReviewer { __typename ... on User { login } } } }
  latestOpinionatedReviews(first: 30) { nodes { state author { login } commit { oid } } }
  votes: reviews(last: 100, states: [APPROVED, CHANGES_REQUESTED, DISMISSED]) { nodes { state author { login } commit { oid } } }
  reviewThreads(first: 100) { nodes { isResolved comments { totalCount } } }
  comments { totalCount }
}
query {
  viewer { login }
  requested: search(query: "is:pr is:open archived:false review-requested:@me", type: ISSUE, first: 100) { issueCount nodes { ...pr } }
  reviewed: search(query: "is:pr is:open archived:false reviewed-by:@me", type: ISSUE, first: 100) { issueCount nodes { ...pr } }
  mine: search(query: "is:pr is:open archived:false author:@me", type: ISSUE, first: 100) { issueCount nodes { ...pr } }
}"#;

#[derive(Debug, Deserialize)]
struct Response {
    data: Option<Data>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

/// A search that timed out or was refused comes back `null`, with the reason in `errors`.
#[derive(Debug, Deserialize)]
struct Data {
    viewer: Login,
    requested: Option<Search>,
    reviewed: Option<Search>,
    mine: Option<Search>,
}

#[derive(Debug, Deserialize)]
struct Search {
    #[serde(rename = "issueCount", default)]
    issue_count: usize,
    /// A search can return non-PR nodes as `{}`; those fail to parse as a PR and are skipped.
    nodes: Vec<serde_json::Value>,
}

impl Search {
    fn prs(self) -> impl Iterator<Item = PullRequest> {
        self.nodes.into_iter().filter_map(|n| serde_json::from_value(n).ok())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Login {
    login: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    database_id: i64,
    number: i64,
    title: String,
    pub url: String,
    is_draft: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    head_ref_name: String,
    base_ref_name: String,
    head_ref_oid: String,
    /// `None` when the author's account was deleted.
    author: Option<Login>,
    repository: Repo,
    review_requests: Nodes<ReviewRequest>,
    /// Each reviewer's latest approval or change request; a later comment doesn't replace it.
    latest_opinionated_reviews: Nodes<Review>,
    /// Votes and dismissals in order; my last one is my vote, even after a later comment or a dismissal.
    votes: Nodes<Review>,
    review_threads: Nodes<Thread>,
    comments: Count,
}

#[derive(Debug, Clone, Deserialize)]
struct Repo {
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRequest {
    requested_reviewer: Option<Reviewer>,
}

#[derive(Debug, Clone, Deserialize)]
struct Reviewer {
    /// Teams and bots have no `login` in the query.
    login: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Review {
    state: String,
    author: Option<Login>,
    commit: Option<Oid>,
}

#[derive(Debug, Clone, Deserialize)]
struct Oid {
    oid: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Thread {
    is_resolved: bool,
    comments: Count,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Count {
    total_count: u32,
}

/// One poll's GitHub PRs, each once, with whether it came from the review-requested search.
pub struct Fetched {
    viewer: String,
    prs: Vec<(PullRequest, bool)>,
    /// GitHub's errors for a response that still had data; some PRs may be missing.
    pub warning: Option<String>,
}

pub async fn fetch(token: &str) -> Result<Fetched, String> {
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("static reqwest config")
        .post(ENDPOINT)
        .bearer_auth(token)
        .header("User-Agent", "rado")
        .json(&serde_json::json!({ "query": QUERY }))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("token rejected (expired or revoked)".into());
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let body = response.bytes().await.map_err(|e| format!("network: {e}"))?;
    parse(&body)
}

fn parse(body: &[u8]) -> Result<Fetched, String> {
    let response: Response = serde_json::from_slice(body).map_err(|e| format!("unexpected response: {e}"))?;
    let mut messages: Vec<String> = Vec::new();
    for error in response.errors {
        if !messages.contains(&error.message) {
            messages.push(error.message);
        }
    }
    let messages = messages.join("; ");
    let failed = || if messages.is_empty() { "a search came back empty".to_string() } else { messages.clone() };
    let Some(data) = response.data else { return Err(failed()) };
    let (Some(requested), Some(reviewed), Some(mine)) = (data.requested, data.reviewed, data.mine) else { return Err(failed()) };
    let mut prs: Vec<(PullRequest, bool)> = Vec::new();
    let truncated = [&requested, &reviewed, &mine].iter().any(|s| s.issue_count > 100);
    for (search, from_requested) in [(requested, true), (reviewed, false), (mine, false)] {
        for pr in search.prs() {
            if !prs.iter().any(|(p, _)| p.database_id == pr.database_id) {
                prs.push((pr, from_requested));
            }
        }
    }
    // Errors next to data, such as a repo the token may not read under SAML SSO, leave out only those PRs.
    let warning = match (messages.is_empty(), truncated) {
        (true, false) => None,
        (true, true) => Some("over 100 PRs in a search; only the first 100 are listed".into()),
        (false, false) => Some(messages),
        (false, true) => Some(format!("{messages}; over 100 PRs in a search")),
    };
    Ok(Fetched { viewer: data.viewer.login, prs, warning })
}

/// Ranked items, links and thread stats (comments and last activity) for the fetched PRs.
pub fn items(fetched: &Fetched, now: DateTime<Utc>) -> (Vec<ReviewItem>, HashMap<i64, String>, HashMap<i64, PrThreads>) {
    let mut items = Vec::new();
    let mut urls = HashMap::new();
    let mut threads = HashMap::new();
    for (pr, requested) in &fetched.prs {
        let Some(item) = classify(pr, *requested, &fetched.viewer, now) else { continue };
        urls.insert(pr.database_id, pr.url.clone());
        threads.insert(pr.database_id, pr_threads(pr));
        items.push(item);
    }
    (items, urls, threads)
}

fn classify(pr: &PullRequest, requested: bool, viewer: &str, now: DateTime<Utc>) -> Option<ReviewItem> {
    let is = |login: &Option<Login>| login.as_ref().is_some_and(|l| l.login.eq_ignore_ascii_case(viewer));
    let votes = || pr.latest_opinionated_reviews.nodes.iter().filter(|r| matches!(r.state.as_str(), "APPROVED" | "CHANGES_REQUESTED"));
    // A repo that dismisses stale approvals on push turns my vote into DISMISSED, which counts as a push since it.
    let my_last = pr.votes.nodes.iter().rev().find(|r| is(&r.author));
    let dismissed = my_last.is_some_and(|r| r.state == "DISMISSED");
    let pushed_since_vote = dismissed || my_last.is_some_and(|r| r.commit.as_ref().is_none_or(|c| c.oid != pr.head_ref_oid));
    let direct = pr
        .review_requests
        .nodes
        .iter()
        .any(|r| r.requested_reviewer.as_ref().and_then(|x| x.login.as_deref()).is_some_and(|l| l.eq_ignore_ascii_case(viewer)));

    let state = if is(&pr.author) {
        ReviewState::Mine
    } else if pushed_since_vote {
        ReviewState::ReReview
    } else if requested {
        ReviewState::NeedsReview
    } else {
        match my_last?.state.as_str() {
            "APPROVED" => ReviewState::Approved,
            _ => ReviewState::WaitingOnAuthor,
        }
    };
    let facts = PrFacts {
        id: pr.database_id,
        provider: Provider::GitHub,
        number: pr.number,
        title: pr.title.clone(),
        repo: pr.repository.name.clone(),
        branch: pr.head_ref_name.clone(),
        author: pr.author.as_ref().map_or_else(|| "ghost".into(), |a| a.login.clone()),
        target: pr.base_ref_name.clone(),
        created_at: pr.created_at,
        draft: pr.is_draft,
        approvals: votes().filter(|r| r.state == "APPROVED").count() as u32,
        objections: votes().filter(|r| r.state == "CHANGES_REQUESTED").count() as u32,
    };
    let via_group = state == ReviewState::NeedsReview && !direct;
    // GitHub has no per-reviewer "required" flag in the API; branch protection decides that.
    Some(score::review_item(facts, state, false, via_group, now))
}

fn pr_threads(pr: &PullRequest) -> PrThreads {
    let threads = &pr.review_threads.nodes;
    PrThreads {
        comments: CommentStats {
            total: pr.comments.total_count + threads.iter().map(|t| t.comments.total_count).sum::<u32>(),
            open_threads: threads.iter().filter(|t| !t.is_resolved).count() as u32,
        },
        last_activity: Some(pr.updated_at),
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    const ME: &str = "me";

    fn now() -> DateTime<Utc> {
        "2026-11-11T12:00:00Z".parse().unwrap()
    }

    /// A PR node as the query returns it; `reviews` are (login, state, commit) in order, and the opinionated
    /// list keeps the approvals and change requests among them.
    fn node(id: i64, author: &str, requests: &[&str], reviews: &[(&str, &str, &str)]) -> serde_json::Value {
        let json = |list: Vec<&(&str, &str, &str)>| -> Vec<serde_json::Value> {
            list.into_iter()
                .map(|(login, state, commit)| serde_json::json!({ "state": state, "author": { "login": login }, "commit": { "oid": commit } }))
                .collect()
        };
        let opinions = json(reviews.iter().filter(|r| matches!(r.1, "APPROVED" | "CHANGES_REQUESTED")).collect());
        let votes = json(reviews.iter().filter(|r| r.1 != "COMMENTED").collect());
        serde_json::json!({
            "databaseId": id, "number": id % 1000, "title": format!("PR {id}"), "url": format!("https://github.com/o/repo/pull/{}", id % 1000),
            "isDraft": false, "createdAt": "2026-11-10T12:00:00Z", "updatedAt": "2026-11-11T09:00:00Z",
            "headRefName": "feature/ABC-1", "baseRefName": "main", "headRefOid": "head",
            "author": { "login": author }, "repository": { "name": "repo" },
            "reviewRequests": { "nodes": requests.iter().map(|r| if *r == "TEAM" {
                serde_json::json!({ "requestedReviewer": { "__typename": "Team" } })
            } else {
                serde_json::json!({ "requestedReviewer": { "__typename": "User", "login": r } })
            }).collect::<Vec<_>>() },
            "latestOpinionatedReviews": { "nodes": opinions },
            "votes": { "nodes": votes },
            "reviewThreads": { "nodes": [
                { "isResolved": false, "comments": { "totalCount": 2 } },
                { "isResolved": true, "comments": { "totalCount": 1 } }
            ] },
            "comments": { "totalCount": 4 }
        })
    }

    fn body(requested: Vec<serde_json::Value>, reviewed: Vec<serde_json::Value>, mine: Vec<serde_json::Value>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "data": {
            "viewer": { "login": "Me" },
            "requested": { "issueCount": requested.len(), "nodes": requested },
            "reviewed": { "issueCount": reviewed.len(), "nodes": reviewed },
            "mine": { "issueCount": mine.len(), "nodes": mine }
        }}))
        .unwrap()
    }

    fn ranked(body: &[u8]) -> Vec<ReviewItem> {
        items(&parse(body).unwrap(), now()).0
    }

    #[test]
    fn requestedDirectly_needsReview() {
        let item = &ranked(&body(vec![node(9_000_001, "a", &[ME], &[])], vec![], vec![]))[0];
        assert_eq!((item.state, item.via_group, item.provider, item.number), (ReviewState::NeedsReview, false, Provider::GitHub, 1));
        assert!(item.actionable);
    }

    #[test]
    fn requestedOnlyThroughATeam_needsReviewViaGroup() {
        let item = &ranked(&body(vec![node(9_000_001, "a", &["TEAM"], &[])], vec![], vec![]))[0];
        assert_eq!((item.state, item.via_group), (ReviewState::NeedsReview, true));
    }

    #[test]
    fn approvedAtHead_isApproved_andPushAfterwards_isReReview() {
        let approved = ranked(&body(vec![], vec![node(1, "a", &[], &[("me", "APPROVED", "head")])], vec![]));
        assert_eq!(approved[0].state, ReviewState::Approved);
        let pushed = ranked(&body(vec![], vec![node(1, "a", &[], &[("me", "APPROVED", "old")])], vec![]));
        assert_eq!((pushed[0].state, pushed[0].actionable), (ReviewState::ReReview, true));
    }

    #[test]
    fn changesRequestedAtHead_isWaitingOnAuthor() {
        let item = &ranked(&body(vec![], vec![node(1, "a", &[], &[("me", "CHANGES_REQUESTED", "head")])], vec![]))[0];
        assert_eq!(item.state, ReviewState::WaitingOnAuthor);
    }

    #[test]
    fn commentAfterMyApproval_thenPush_isStillReReview() {
        let reviews = [("me", "APPROVED", "old"), ("me", "COMMENTED", "head")];
        assert_eq!(ranked(&body(vec![], vec![node(1, "a", &[], &reviews)], vec![]))[0].state, ReviewState::ReReview);
    }

    #[test]
    fn myDismissedApproval_isReReview() {
        let item = &ranked(&body(vec![], vec![node(1, "a", &[], &[("me", "DISMISSED", "old")])], vec![]))[0];
        assert_eq!((item.state, item.approvals), (ReviewState::ReReview, 0));
    }

    #[test]
    fn approvalDismissedAfterALaterComment_isStillReReview() {
        let reviews = [("me", "DISMISSED", "old"), ("me", "COMMENTED", "head")];
        assert_eq!(ranked(&body(vec![], vec![node(1, "a", &[], &reviews)], vec![]))[0].state, ReviewState::ReReview);
    }

    #[test]
    fn dismissedThenApprovedAgainAtHead_isApproved() {
        let reviews = [("me", "DISMISSED", "old"), ("me", "APPROVED", "head")];
        assert_eq!(ranked(&body(vec![], vec![node(1, "a", &[], &reviews)], vec![]))[0].state, ReviewState::Approved);
    }

    #[test]
    fn searchOver100Results_isAWarning() {
        let mut json: serde_json::Value = serde_json::from_slice(&body(vec![node(1, "a", &[ME], &[])], vec![], vec![])).unwrap();
        json["data"]["requested"]["issueCount"] = 140.into();
        let warning = parse(&serde_json::to_vec(&json).unwrap()).unwrap().warning;
        assert!(warning.is_some_and(|w| w.contains("over 100")));
    }

    #[test]
    fn reviewerWhoCommentedAfterRequestingChanges_stillObjectsOnMyPr() {
        let reviews = [("x", "CHANGES_REQUESTED", "head"), ("x", "COMMENTED", "head")];
        let item = &ranked(&body(vec![], vec![], vec![node(1, "me", &[], &reviews)]))[0];
        assert_eq!(item.objections, 1);
    }

    #[test]
    fn nullSearch_failsWithGitHubsMessage() {
        let body = br#"{"data":{"viewer":{"login":"me"},"requested":null,"reviewed":{"nodes":[]},"mine":{"nodes":[]}},"errors":[{"message":"timeout"}]}"#;
        assert_eq!(parse(body).err().unwrap(), "timeout");
    }

    #[test]
    fn errorsNextToData_areAWarning() {
        let body = br#"{"data":{"viewer":{"login":"me"},"requested":{"nodes":[]},"reviewed":{"nodes":[]},"mine":{"nodes":[]}},"errors":[{"message":"SAML enforcement"}]}"#;
        assert_eq!(parse(body).unwrap().warning.as_deref(), Some("SAML enforcement"));
    }

    #[test]
    fn onlyCommentedAndNotRequested_isLeftOut() {
        assert!(ranked(&body(vec![], vec![node(1, "a", &[], &[("me", "COMMENTED", "head")])], vec![])).is_empty());
    }

    #[test]
    fn myOwnPr_isMineWithVotesAndThreadCounts() {
        let fetched = parse(&body(vec![], vec![], vec![node(1, "me", &[], &[("x", "APPROVED", "head"), ("y", "CHANGES_REQUESTED", "head")])])).unwrap();
        let (items, urls, threads) = items(&fetched, now());
        assert_eq!((items[0].state, items[0].approvals, items[0].objections), (ReviewState::Mine, 1, 1));
        assert_eq!(urls[&1], "https://github.com/o/repo/pull/1");
        assert_eq!(threads[&1].comments, CommentStats { total: 7, open_threads: 1 });
        assert_eq!(threads[&1].last_activity, Some("2026-11-11T09:00:00Z".parse().unwrap()));
    }

    #[test]
    fn prInSeveralSearches_isListedOnceAsRequested() {
        let pr = node(1, "a", &[ME], &[]);
        let fetched = parse(&body(vec![pr.clone()], vec![pr], vec![])).unwrap();
        assert_eq!(fetched.prs.len(), 1);
        assert!(fetched.prs[0].1);
    }

    #[test]
    fn nonPrSearchNode_isSkipped() {
        assert_eq!(parse(&body(vec![serde_json::json!({}), node(1, "a", &[ME], &[])], vec![], vec![])).unwrap().prs.len(), 1);
    }

    #[test]
    fn graphQlErrorsWithoutData_areReported() {
        let error = parse(br#"{"data":null,"errors":[{"message":"Bad credentials"}]}"#).err().unwrap();
        assert_eq!(error, "Bad credentials");
    }
}
