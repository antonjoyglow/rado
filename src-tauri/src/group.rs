use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::score::{CommentStats, Provider, ReviewItem, ReviewState};

/// One PR inside a row; `state` is `None` for a PR on the same branch that I am not asked to review.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: i64,
    pub provider: Provider,
    pub number: i64,
    pub repo: String,
    pub title: String,
    pub url: String,
    pub state: Option<ReviewState>,
    pub created_at: DateTime<Utc>,
    /// `None` for a related PR (its threads aren't fetched) or when its threads failed to load.
    pub updated_at: Option<DateTime<Utc>>,
}

/// An active PR on the same source branch as one of mine, where I am not a reviewer.
#[derive(Debug, Clone)]
pub struct Related {
    pub id: i64,
    pub repo: String,
    pub title: String,
    pub branch: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
}

/// A list row: one PR, or every PR sharing a source branch. The flattened fields describe the row as a whole.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRow {
    pub key: String,
    #[serde(flatten)]
    pub summary: ReviewItem,
    pub members: Vec<Member>,
    /// Age of the row's most recently opened PR; "Your PRs" sorts on it, newest first.
    pub newest_age_hours: i64,
    /// The Jira issue key in the branch name, such as `ABC-12` in `feature/ABC-12-login`.
    pub ticket: Option<String>,
}

/// The first `KEY-123` in a branch: upper-case letters and digits starting with a letter, a dash and digits,
/// standing alone between non-alphanumeric characters.
pub fn ticket_key(branch: &str) -> Option<String> {
    let bytes = branch.as_bytes();
    let alnum = |i: usize| bytes.get(i).is_some_and(|b| b.is_ascii_alphanumeric());
    (0..bytes.len()).find_map(|start| {
        if !bytes[start].is_ascii_uppercase() || (start > 0 && alnum(start - 1)) {
            return None;
        }
        let mut i = start;
        while bytes.get(i).is_some_and(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
            i += 1;
        }
        if bytes.get(i) != Some(&b'-') {
            return None;
        }
        let digits = i + 1;
        let mut end = digits;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        (end > digits && !alnum(end)).then(|| branch[start..end].to_string())
    })
}

/// Groups my ranked PRs by source branch, adds the related PRs on those branches, and orders rows by score.
pub fn group(items: &[ReviewItem], related: &[Related], urls: &HashMap<i64, String>) -> Vec<ReviewRow> {
    let mut groups: Vec<(Provider, &str, Vec<&ReviewItem>)> = Vec::new();
    for item in items {
        match groups.iter_mut().find(|(provider, branch, _)| *provider == item.provider && *branch == item.branch) {
            Some((_, _, members)) => members.push(item),
            None => groups.push((item.provider, &item.branch, vec![item])),
        }
    }

    let mut rows: Vec<ReviewRow> = groups
        .into_iter()
        .map(|(provider, branch, mut mine)| {
            let stale = |i: &ReviewItem| i.state == ReviewState::Stale;
            mine.sort_by(|a, b| a.hidden.cmp(&b.hidden).then(stale(a).cmp(&stale(b))).then(b.score.cmp(&a.score)));
            let others: Vec<&Related> = related.iter().filter(|r| provider == Provider::Ado && r.branch == branch).collect();
            let lead = mine[0];
            let comments = mine.iter().filter_map(|m| m.comments).reduce(|a, b| CommentStats {
                total: a.total + b.total,
                open_threads: a.open_threads + b.open_threads,
            });
            let created_at = mine.iter().map(|m| m.created_at).min().unwrap_or(lead.created_at);
            // The row names a release target when any of my PRs on the branch merges into one, not only the lead.
            let release = mine.iter().find(|m| m.release_target);
            let summary = ReviewItem {
                target: release.map_or_else(|| lead.target.clone(), |m| m.target.clone()),
                release_target: release.is_some(),
                hidden: mine.iter().all(|m| m.hidden),
                required: mine.iter().any(|m| m.required),
                via_group: mine.iter().all(|m| m.via_group),
                draft: mine.iter().all(|m| m.draft),
                age_hours: mine.iter().map(|m| m.age_hours).max().unwrap_or_default(),
                created_at,
                // A sibling opened after the earliest PR is activity on the row too.
                updated_at: mine
                    .iter()
                    .map(|m| m.updated_at.unwrap_or(m.created_at))
                    .max()
                    .and_then(|at| crate::score::activity_after(at, created_at)),
                approvals: mine.iter().map(|m| m.approvals).sum(),
                objections: mine.iter().map(|m| m.objections).sum(),
                comments,
                actionable: mine.iter().any(|m| m.actionable && !m.hidden),
                ..lead.clone()
            };
            let members = mine
                .iter()
                .map(|m| Member {
                    id: m.id,
                    provider: m.provider,
                    number: m.number,
                    repo: m.repo.clone(),
                    title: m.title.clone(),
                    url: urls.get(&m.id).cloned().unwrap_or_default(),
                    state: Some(m.state),
                    created_at: m.created_at,
                    updated_at: m.updated_at,
                })
                .chain(others.iter().map(|r| Member {
                    id: r.id,
                    provider: Provider::Ado,
                    number: r.id,
                    repo: r.repo.clone(),
                    title: r.title.clone(),
                    url: r.url.clone(),
                    state: None,
                    created_at: r.created_at,
                    updated_at: None,
                }))
                .collect::<Vec<_>>();
            let key = if members.len() > 1 { format!("branch:{provider:?}:{branch}") } else { format!("pr:{}", lead.id) };
            let newest_age_hours = mine.iter().map(|m| m.age_hours).min().unwrap_or_default();
            // Jira keys come from Azure DevOps branches; on GitHub `fix/UTF-8` would link to a ticket that does not exist.
            let ticket = if provider == Provider::Ado { ticket_key(branch) } else { None };
            ReviewRow { key, summary, members, newest_age_hours, ticket }
        })
        .collect();
    rows.sort_by(|a, b| b.summary.score.cmp(&a.summary.score).then(b.summary.age_hours.cmp(&a.summary.age_hours)));
    rows
}

/// Text for sharing: a lone PR is its link; an aggregate is one `repo: link` line per PR.
pub fn share_text(members: &[Member]) -> String {
    match members {
        [only] => only.url.clone(),
        _ => members.iter().map(|m| format!("{}: {}", m.repo, m.url)).collect::<Vec<_>>().join("\n"),
    }
}

/// Links to open for a row, each tagged `#rado=<branch>` for the Rado browser extension (browser-extension/),
/// which titles the tab `repo:branch`. An aggregate adds `&group`, so its tabs share a tab group.
pub fn open_urls(row: &ReviewRow) -> Vec<String> {
    let group = if row.members.len() > 1 { "&group" } else { "" };
    let tag = format!("#rado={}{group}", crate::ado::urlencode(&row.summary.branch));
    row.members.iter().map(|m| format!("{}{tag}", m.url)).collect()
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::score::Tier;

    fn item(id: i64, repo: &str, branch: &str, score: i32) -> ReviewItem {
        ReviewItem {
            id,
            provider: Provider::Ado,
            number: id,
            title: format!("PR {id}"),
            repo: repo.into(),
            branch: branch.into(),
            author: "A".into(),
            target: "dev".into(),
            release_target: false,
            age_hours: id,
            created_at: at(-id),
            updated_at: None,
            required: false,
            via_group: false,
            draft: false,
            state: ReviewState::NeedsReview,
            score,
            tier: Tier::Normal,
            hidden: false,
            approvals: 1,
            objections: 0,
            comments: None,
            actionable: true,
        }
    }

    /// A time `hours` from a fixed noon; negative is earlier.
    fn at(hours: i64) -> DateTime<Utc> {
        "2026-11-04T12:00:00Z".parse::<DateTime<Utc>>().unwrap() + chrono::Duration::hours(hours)
    }

    #[test]
    fn group_aggregate_takesEarliestCreatedAndLatestKnownUpdate() {
        let mut a = item(1, "Api", "feature/x", 60);
        a.updated_at = Some(at(-1));
        let mut b = item(5, "Core", "feature/x", 50);
        b.updated_at = Some(at(-3));
        let c = item(3, "WebApp", "feature/x", 40);
        let rows = group(&[a, b, c], &[], &urls(&[1, 3, 5]));
        assert_eq!((rows[0].summary.created_at, rows[0].summary.updated_at), (at(-5), Some(at(-1))));
        assert_eq!(rows[0].members.iter().map(|m| m.updated_at).collect::<Vec<_>>(), [Some(at(-1)), Some(at(-3)), None]);
    }

    #[test]
    fn group_quietSiblingOpenedLater_isTheRowsLatestActivity() {
        let mut old = item(120, "Api", "feature/x", 60);
        old.updated_at = Some(at(-96));
        let fresh = item(1, "Core", "feature/x", 50);
        let rows = group(&[old, fresh], &[], &urls(&[1, 120]));
        assert_eq!((rows[0].summary.created_at, rows[0].summary.updated_at), (at(-120), Some(at(-1))));
    }

    #[test]
    fn group_noThreadsLoaded_hasNoUpdate() {
        let rows = group(&[item(1, "Api", "feature/x", 60)], &[], &urls(&[1]));
        assert_eq!(rows[0].summary.updated_at, None);
    }

    fn urls(ids: &[i64]) -> HashMap<i64, String> {
        ids.iter().map(|id| (*id, format!("https://x/{id}"))).collect()
    }

    #[test]
    fn group_sameBranchAcrossRepos_isOneRowWithAllMembers() {
        let items = [item(1, "Api", "feature/x", 60), item(2, "Core", "feature/x", 70), item(3, "WebApp", "feature/y", 50)];
        let rows = group(&items, &[], &urls(&[1, 2, 3]));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "branch:Ado:feature/x");
        assert_eq!(rows[0].summary.id, 2, "the highest-scored PR leads the row");
        assert_eq!(rows[0].summary.approvals, 2);
        assert_eq!((rows[0].summary.age_hours, rows[0].newest_age_hours), (2, 1), "oldest and newest PR ages");
        assert_eq!(rows[0].members.iter().map(|m| m.id).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(rows[1].key, "pr:3");
    }

    #[test]
    fn group_siblingIntoReleaseBranch_namesThatTargetOnTheRow() {
        let mut sibling = item(2, "Core", "feature/x", 50);
        (sibling.target, sibling.release_target) = ("qa".into(), true);
        let rows = group(&[item(1, "Api", "feature/x", 60), sibling], &[], &urls(&[1, 2]));
        assert_eq!(rows[0].summary.id, 1, "the lead merges into dev");
        assert_eq!((rows[0].summary.target.as_str(), rows[0].summary.release_target), ("qa", true));
        let plain = group(&[item(1, "Api", "feature/x", 60), item(2, "Core", "feature/x", 50)], &[], &urls(&[1, 2]));
        assert_eq!((plain[0].summary.target.as_str(), plain[0].summary.release_target), ("dev", false));
    }

    #[test]
    fn group_relatedPrOnMyBranch_joinsAsUnreviewedMember() {
        let related = [
            Related { id: 9, repo: "WebApp".into(), title: "ui".into(), branch: "feature/x".into(), url: "https://x/9".into(), created_at: at(-9) },
            Related { id: 8, repo: "WebApp".into(), title: "other".into(), branch: "feature/z".into(), url: "https://x/8".into(), created_at: at(-8) },
        ];
        let rows = group(&[item(1, "Api", "feature/x", 60)], &related, &urls(&[1]));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].members.len(), 2);
        assert!(rows[0].members[1].state.is_none());
    }

    #[test]
    fn group_partlyHidden_isVisibleAndLedByAVisiblePr() {
        let mut hidden = item(2, "Core", "feature/x", 90);
        hidden.hidden = true;
        let rows = group(&[hidden, item(1, "Api", "feature/x", 60)], &[], &urls(&[1, 2]));
        assert!(!rows[0].summary.hidden);
        assert_eq!(rows[0].summary.id, 1);
    }

    #[test]
    fn group_commentsSumOnlyKnownCounts() {
        let mut a = item(1, "Api", "feature/x", 60);
        a.comments = Some(CommentStats { total: 3, open_threads: 1 });
        let mut b = item(2, "Core", "feature/x", 50);
        b.comments = Some(CommentStats { total: 2, open_threads: 2 });
        let c = item(3, "WebApp", "feature/x", 40);
        let rows = group(&[a, b, c], &[], &urls(&[1, 2, 3]));
        assert_eq!(rows[0].summary.comments, Some(CommentStats { total: 5, open_threads: 3 }));
    }

    #[test]
    fn openUrls_aggregate_tagsEveryLinkWithTheEncodedBranch() {
        let items = [item(1, "Api", "feature/x y", 60), item(2, "Core", "feature/x y", 50)];
        let rows = group(&items, &[], &urls(&[1, 2]));
        assert_eq!(open_urls(&rows[0]), ["https://x/1#rado=feature%2Fx%20y&group", "https://x/2#rado=feature%2Fx%20y&group"]);
    }

    #[test]
    fn openUrls_singlePr_isTaggedWithoutGroup() {
        let rows = group(&[item(1, "Api", "feature/x", 60)], &[], &urls(&[1]));
        assert_eq!(open_urls(&rows[0]), ["https://x/1#rado=feature%2Fx"]);
    }

    #[test]
    fn group_staleAndFreshOnOneBranch_isLedByTheFreshPr() {
        let mut stale = item(2, "Core", "feature/x", 90);
        stale.state = ReviewState::Stale;
        stale.actionable = false;
        let rows = group(&[stale, item(1, "Api", "feature/x", 60)], &[], &urls(&[1, 2]));
        assert_eq!((rows[0].summary.id, rows[0].summary.state, rows[0].summary.actionable), (1, ReviewState::NeedsReview, true));
    }

    #[test]
    fn group_sameBranchOnGitHubAndAzureDevOps_isTwoRows() {
        let mut github = item(500, "rado", "feature/x", 70);
        (github.provider, github.number) = (Provider::GitHub, 4);
        let related = [Related { id: 9, repo: "WebApp".into(), title: "ui".into(), branch: "feature/x".into(), url: "https://x/9".into(), created_at: at(-9) }];
        let rows = group(&[github, item(1, "Api", "feature/x", 60)], &related, &urls(&[1, 500]));
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].key.as_str(), rows[0].members.len(), rows[0].members[0].number), ("pr:500", 1, 4));
        assert_eq!((rows[1].key.as_str(), rows[1].members.len()), ("branch:Ado:feature/x", 2), "Azure DevOps related PRs join only the Azure DevOps row");
    }

    #[test]
    fn group_githubBranchWithKeyLikeText_hasNoTicket() {
        let mut github = item(500, "rado", "fix/UTF-8", 70);
        github.provider = Provider::GitHub;
        assert_eq!(group(&[github], &[], &urls(&[500]))[0].ticket, None);
    }

    #[test]
    fn group_branchWithIssueKey_carriesTheTicket() {
        let rows = group(&[item(1, "Api", "feature/ABC-12-login", 60)], &[], &urls(&[1]));
        assert_eq!(rows[0].ticket.as_deref(), Some("ABC-12"));
    }

    #[test]
    fn ticketKey_findsTheKeyInCommonBranchShapes() {
        for (branch, key) in [
            ("ABC-12", Some("ABC-12")),
            ("feature/ABC-12", Some("ABC-12")),
            ("epic/ABC-4640", Some("ABC-4640")),
            ("bugfix/ABC-7_null-check", Some("ABC-7")),
            ("users/me/X2Y-99-fix", Some("X2Y-99")),
            ("feature/abc-12", None),
            ("feature/NOTE-", None),
            ("release-2026", None),
            ("feature/MYABC-12x", None),
            ("featureABC-12", None),
            ("main", None),
        ] {
            assert_eq!(ticket_key(branch).as_deref(), key, "{branch}");
        }
    }

    #[test]
    fn shareText_singlePr_isTheBareLink() {
        let rows = group(&[item(1, "Api", "feature/x", 60)], &[], &urls(&[1]));
        assert_eq!(share_text(&rows[0].members), "https://x/1");
    }

    #[test]
    fn shareText_aggregate_isRepoPrefixedLines() {
        let items = [item(1, "Api", "feature/x", 60), item(2, "Core", "feature/x", 50)];
        let rows = group(&items, &[], &urls(&[1, 2]));
        assert_eq!(share_text(&rows[0].members), "Api: https://x/1\nCore: https://x/2");
    }
}
