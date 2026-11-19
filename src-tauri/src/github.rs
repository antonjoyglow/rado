//! GitHub (github.com) pull requests: one GraphQL call per poll returns every PR waiting on my review,
//! the ones I reviewed, my own, and the open PRs of the workspace's watched repos, with what Rado shows for each.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::score::{self, CommentStats, PrFacts, PrThreads, Provider, ReviewItem, ReviewState};
use crate::source::{QueueRef, SourceError, SourcePoll, retry_at, throttled};

const ENDPOINT: &str = "https://api.github.com/graphql";

/// Requested covers requests to me and to my teams; reviewed catches new pushes after my review; watched runs only
/// when the workspace watches repos.
const QUERY: &str = r#"
fragment pr on PullRequest {
  databaseId number title url isDraft createdAt updatedAt headRefName baseRefName headRefOid
  author { __typename login }
  repository { name owner { login } }
  labels(first: 20) { nodes { name } }
  reviewRequests(first: 20) { nodes { requestedReviewer { __typename ... on User { login } } } }
  latestOpinionatedReviews(first: 30) { nodes { state author { login } commit { oid } } }
  votes: reviews(last: 100, states: [APPROVED, CHANGES_REQUESTED, DISMISSED]) { nodes { state author { login } commit { oid } } }
  reviewThreads(first: 100) { nodes { isResolved comments { totalCount } } }
  comments { totalCount }
}
query($watched: String!, $watching: Boolean!) {
  viewer { login }
  requested: search(query: "is:pr is:open archived:false review-requested:@me", type: ISSUE, first: 100) { issueCount nodes { ...pr } }
  reviewed: search(query: "is:pr is:open archived:false reviewed-by:@me", type: ISSUE, first: 100) { issueCount nodes { ...pr } }
  mine: search(query: "is:pr is:open archived:false author:@me", type: ISSUE, first: 100) { issueCount nodes { ...pr } }
  watched: search(query: $watched, type: ISSUE, first: 100) @include(if: $watching) { issueCount nodes { ...pr } }
}"#;

/// Open PRs in `repos` that are neither drafts nor mine; `None` without repos.
fn watched_query(repos: &[String]) -> Option<String> {
    (!repos.is_empty()).then(|| {
        let repos: Vec<String> = repos.iter().map(|r| format!("repo:{r}")).collect();
        format!("is:pr is:open archived:false draft:false -author:@me {}", repos.join(" "))
    })
}

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
    /// Absent when no repos are watched; `Some(None)` when the search failed.
    #[serde(default, deserialize_with = "present")]
    watched: Option<Option<Search>>,
}

fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<Search>>, D::Error> {
    Option::<Search>::deserialize(deserializer).map(Some)
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
    /// `User` or `Bot` (an app such as an agent or Dependabot); the viewer has none.
    #[serde(rename = "__typename", default)]
    kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Label {
    name: String,
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
    /// Only for the agent flag, so a missing or null list must not drop the PR.
    #[serde(default)]
    labels: Option<Nodes<Option<Label>>>,
}

#[derive(Debug, Clone, Deserialize)]
struct Repo {
    name: String,
    owner: Login,
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

/// Which searches found a PR.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Found {
    requested: bool,
    watched: bool,
}

/// One poll's GitHub PRs, each once, with the searches that found it.
pub struct Fetched {
    viewer: String,
    prs: Vec<(PullRequest, Found)>,
    /// GitHub's errors for a response that still had data; some PRs may be missing.
    pub warning: Option<String>,
    /// Reviews waiting on me may be missing: an error, or a review search over 100 results.
    pub partial: bool,
}

pub async fn fetch(token: &str, watched_repos: &[String]) -> Result<Fetched, SourceError> {
    let watched = watched_query(watched_repos);
    let variables = serde_json::json!({ "watched": watched.as_deref().unwrap_or_default(), "watching": watched.is_some() });
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("static reqwest config")
        .post(ENDPOINT)
        .bearer_auth(token)
        .header("User-Agent", "rado")
        .json(&serde_json::json!({ "query": QUERY, "variables": variables }))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    let status = response.status();
    let retry = retry_at(response.headers(), Utc::now());
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("token rejected (expired or revoked)".into());
    }
    if throttled(status, retry) {
        return Err(SourceError::rate_limited(retry));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}").into());
    }
    let body = response.bytes().await.map_err(|e| format!("network: {e}"))?;
    // GraphQL reports a spent rate limit as a 200 with an error and no data.
    parse(&body).map_err(|e| if retry.is_some() { SourceError::rate_limited(retry) } else { e.into() })
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
    let watched = match data.watched {
        None => None,
        Some(None) => return Err(failed()),
        Some(Some(search)) => Some(search),
    };
    let truncated = [Some(&requested), Some(&reviewed), Some(&mine), watched.as_ref()].iter().flatten().any(|s| s.issue_count > 100);
    // My own PRs never reach the review queue, so only errors or the other searches running over make it partial.
    let partial = !messages.is_empty() || [Some(&requested), Some(&reviewed), watched.as_ref()].iter().flatten().any(|s| s.issue_count > 100);
    let mut prs: Vec<(PullRequest, Found)> = Vec::new();
    let searches = [
        (Some(requested), Found { requested: true, watched: false }),
        (Some(reviewed), Found::default()),
        (Some(mine), Found::default()),
        (watched, Found { requested: false, watched: true }),
    ];
    for (search, found) in searches {
        for pr in search.into_iter().flat_map(Search::prs) {
            match prs.iter_mut().find(|(p, _)| p.database_id == pr.database_id) {
                Some((_, seen)) => seen.watched |= found.watched,
                None => prs.push((pr, found)),
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
    Ok(Fetched { viewer: data.viewer.login, prs, warning, partial })
}

/// The fetched PRs as a source poll. GitHub lists no related PRs.
pub fn digest(fetched: Fetched, now: DateTime<Utc>) -> SourcePoll {
    let (items, urls, threads) = items(&fetched, now);
    SourcePoll {
        items,
        urls,
        threads,
        related: Vec::new(),
        queue_refs: queue_refs(&fetched),
        warning: fetched.warning.map(|w| format!("GitHub: {w}")),
        queue_partial: fetched.partial,
    }
}

/// Ranked items, links and thread stats (comments and last activity) for the fetched PRs.
fn items(fetched: &Fetched, now: DateTime<Utc>) -> (Vec<ReviewItem>, HashMap<i64, String>, HashMap<i64, PrThreads>) {
    let mut items = Vec::new();
    let mut urls = HashMap::new();
    let mut threads = HashMap::new();
    for (pr, found) in &fetched.prs {
        let Some(item) = classify(pr, *found, &fetched.viewer, now) else { continue };
        urls.insert(pr.database_id, pr.url.clone());
        threads.insert(pr.database_id, pr_threads(pr));
        items.push(item);
    }
    (items, urls, threads)
}

/// Labels that mark a PR as written by an agent, compared whole and without case.
const AGENT_LABELS: [&str; 4] = ["agent", "ai", "ai-generated", "ai-assisted"];

/// What the MCP queue needs for each fetched PR, by id: the owner, head commit, my last vote's commit and whether
/// an agent wrote it.
fn queue_refs(fetched: &Fetched) -> HashMap<i64, QueueRef> {
    fetched
        .prs
        .iter()
        .map(|(pr, _)| {
            let agent = pr.author.as_ref().is_some_and(|a| a.kind.as_deref() == Some("Bot"))
                || pr.labels.iter().flat_map(|n| &n.nodes).flatten().any(|l| AGENT_LABELS.iter().any(|a| a.eq_ignore_ascii_case(l.name.trim())));
            let owner = &pr.repository.owner.login;
            let reference = QueueRef {
                project: owner.clone(),
                repo_id: format!("{owner}/{}", pr.repository.name),
                agent,
                source_commit: Some(pr.head_ref_oid.clone()),
                last_voted: my_last_vote(pr, &fetched.viewer).and_then(|r| r.commit.as_ref()).map(|c| c.oid.clone()),
            };
            (pr.database_id, reference)
        })
        .collect()
}

/// My latest approval, change request or dismissed vote; later comments don't replace it.
fn my_last_vote<'a>(pr: &'a PullRequest, viewer: &str) -> Option<&'a Review> {
    pr.votes.nodes.iter().rev().find(|r| r.author.as_ref().is_some_and(|l| l.login.eq_ignore_ascii_case(viewer)))
}

fn classify(pr: &PullRequest, found: Found, viewer: &str, now: DateTime<Utc>) -> Option<ReviewItem> {
    let is = |login: &Option<Login>| login.as_ref().is_some_and(|l| l.login.eq_ignore_ascii_case(viewer));
    let votes = || pr.latest_opinionated_reviews.nodes.iter().filter(|r| matches!(r.state.as_str(), "APPROVED" | "CHANGES_REQUESTED"));
    // A repo that dismisses stale approvals on push turns my vote into DISMISSED, which counts as a push since it.
    let my_last = my_last_vote(pr, viewer);
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
    } else if found.requested {
        ReviewState::NeedsReview
    } else if let Some(vote) = my_last {
        if vote.state == "APPROVED" { ReviewState::Approved } else { ReviewState::WaitingOnAuthor }
    } else if found.watched {
        ReviewState::NeedsReview
    } else {
        return None;
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
pub(crate) mod tests {
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
            "author": { "__typename": "User", "login": author }, "repository": { "name": "repo", "owner": { "login": "o" } },
            "labels": { "nodes": [] },
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

    fn with_watched(body: &[u8], watched: serde_json::Value) -> Vec<u8> {
        let mut json: serde_json::Value = serde_json::from_slice(body).unwrap();
        json["data"]["watched"] = watched;
        serde_json::to_vec(&json).unwrap()
    }

    fn watched(nodes: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({ "issueCount": nodes.len(), "nodes": nodes })
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

    /// A poll whose PRs all request my review directly.
    pub fn requested(ids: &[i64]) -> Fetched {
        parse(&body(ids.iter().map(|&id| node(id, "a", &[ME], &[])).collect(), vec![], vec![])).unwrap()
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
        assert!(fetched.prs[0].1.requested);
    }

    #[test]
    fn watchedQuery_listsEachRepoAndLeavesOutDraftsAndMine() {
        assert_eq!(watched_query(&[]), None);
        let query = watched_query(&["coding-tree-io/hoopwise.ai".into(), "o/r".into()]).unwrap();
        assert_eq!(query, "is:pr is:open archived:false draft:false -author:@me repo:coding-tree-io/hoopwise.ai repo:o/r");
    }

    #[test]
    fn watchedUnrequested_needsReviewViaGroup() {
        let item = &ranked(&with_watched(&body(vec![], vec![], vec![]), watched(vec![node(1, "a", &[], &[])])))[0];
        assert_eq!((item.state, item.via_group, item.actionable), (ReviewState::NeedsReview, true, true));
    }

    #[test]
    fn watchedRequestedDirectly_isADirectRequest() {
        let pr = node(1, "a", &[ME], &[]);
        let item = &ranked(&with_watched(&body(vec![pr.clone()], vec![], vec![]), watched(vec![pr])))[0];
        assert_eq!((item.state, item.via_group), (ReviewState::NeedsReview, false));
    }

    #[test]
    fn watchedApprovedAtHead_isApproved_andPushAfterwards_isReReview() {
        let approved = node(1, "a", &[], &[("me", "APPROVED", "head")]);
        let item = &ranked(&with_watched(&body(vec![], vec![approved.clone()], vec![]), watched(vec![approved])))[0];
        assert_eq!(item.state, ReviewState::Approved);
        let pushed = node(1, "a", &[], &[("me", "APPROVED", "old")]);
        assert_eq!(ranked(&with_watched(&body(vec![], vec![], vec![]), watched(vec![pushed])))[0].state, ReviewState::ReReview);
    }

    #[test]
    fn watchedOnlyCommented_stillNeedsReview() {
        let pr = node(1, "a", &[], &[("me", "COMMENTED", "head")]);
        let item = &ranked(&with_watched(&body(vec![], vec![pr.clone()], vec![]), watched(vec![pr])))[0];
        assert_eq!(item.state, ReviewState::NeedsReview);
    }

    #[test]
    fn watchedMyOwnPr_staysMine() {
        let pr = node(1, "me", &[], &[]);
        assert_eq!(ranked(&with_watched(&body(vec![], vec![], vec![pr.clone()]), watched(vec![pr])))[0].state, ReviewState::Mine);
    }

    #[test]
    fn watchedSearch_nullFails_over100Warns() {
        let failed = with_watched(&body(vec![], vec![], vec![]), serde_json::Value::Null);
        assert!(parse(&failed).is_err());
        let mut over = watched(vec![node(1, "a", &[], &[])]);
        over["issueCount"] = 140.into();
        let warning = parse(&with_watched(&body(vec![], vec![], vec![]), over)).unwrap().warning;
        assert!(warning.is_some_and(|w| w.contains("over 100")));
    }

    #[test]
    fn nonPrSearchNode_isSkipped() {
        assert_eq!(parse(&body(vec![serde_json::json!({}), node(1, "a", &[ME], &[])], vec![], vec![])).unwrap().prs.len(), 1);
    }

    #[test]
    fn queueRefs_carryOwnerHeadAndMyLastVotedCommit() {
        let reviews = [("me", "APPROVED", "old"), ("me", "COMMENTED", "head")];
        let refs = queue_refs(&parse(&body(vec![], vec![node(1, "a", &[], &reviews)], vec![])).unwrap());
        let r = &refs[&1];
        assert_eq!((r.project.as_str(), r.repo_id.as_str(), r.source_commit.as_deref()), ("o", "o/repo", Some("head")));
        assert_eq!((r.last_voted.as_deref(), r.agent), (Some("old"), false));
    }

    #[test]
    fn queueRefs_botAuthorOrAgentLabel_isAgentAuthored() {
        let mut bot = node(1, "copilot", &[ME], &[]);
        bot["author"]["__typename"] = "Bot".into();
        let mut labelled = node(2, "a", &[ME], &[]);
        labelled["labels"]["nodes"] = serde_json::json!([{ "name": "bug" }, { "name": "AI-Generated" }]);
        let mut other = node(3, "a", &[ME], &[]);
        other["labels"]["nodes"] = serde_json::json!([{ "name": "airflow" }, { "name": "agents" }]);
        let refs = queue_refs(&parse(&body(vec![bot, labelled, other], vec![], vec![])).unwrap());
        assert_eq!((refs[&1].agent, refs[&2].agent, refs[&3].agent), (true, true, false));
    }

    #[test]
    fn nullLabels_keepThePr() {
        let mut none = node(1, "a", &[ME], &[]);
        none["labels"] = serde_json::Value::Null;
        let mut null_node = node(2, "a", &[ME], &[]);
        null_node["labels"]["nodes"] = serde_json::json!([null, { "name": "agent" }]);
        let fetched = parse(&body(vec![none, null_node], vec![], vec![])).unwrap();
        assert_eq!(fetched.prs.len(), 2);
        assert!(queue_refs(&fetched)[&2].agent);
    }

    #[test]
    fn manyOfMyOwnPrs_warnButDontMakeTheReviewsPartial() {
        let mut json: serde_json::Value = serde_json::from_slice(&body(vec![], vec![], vec![node(1, "me", &[], &[])])).unwrap();
        json["data"]["mine"]["issueCount"] = 140.into();
        let fetched = parse(&serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(fetched.warning.is_some() && !fetched.partial);
        assert!(partial(&[1]).partial);
    }

    /// A poll whose PRs all request my review directly, from a search over 100 results.
    pub fn partial(ids: &[i64]) -> Fetched {
        let mut json: serde_json::Value = serde_json::from_slice(&body(ids.iter().map(|&id| node(id, "a", &[ME], &[])).collect(), vec![], vec![])).unwrap();
        json["data"]["requested"]["issueCount"] = 140.into();
        parse(&serde_json::to_vec(&json).unwrap()).unwrap()
    }

    #[test]
    fn graphQlErrorsWithoutData_areReported() {
        let error = parse(br#"{"data":null,"errors":[{"message":"Bad credentials"}]}"#).err().unwrap();
        assert_eq!(error, "Bad credentials");
    }
}
