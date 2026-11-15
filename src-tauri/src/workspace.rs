//! Workspaces: named sets of sources (Azure DevOps, GitHub) plus a tracker. Only the active one is polled.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    /// Stable across renames; names this workspace's keyring entries and data folder.
    pub id: String,
    pub name: String,
    /// On once a PAT is saved for this workspace.
    pub azure_dev_ops: AdoSource,
    /// On once a GitHub token is saved for this workspace.
    pub github: GithubSource,
    #[serde(deserialize_with = "lenient_tracker")]
    pub tracker: Tracker,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdoSource {
    pub organization: String,
    /// Team reviewers whose PRs count as mine, by name (`TeamAlpha` matches `[contoso]\TeamAlpha`).
    pub teams: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GithubSource {
    /// Repos (`owner/name`) whose open PRs, other than drafts and mine, all count as waiting on my review.
    pub repos: Vec<String>,
}

/// Where ticket keys in branch names link to. `keys` are the team or project keys, such as `HOO`; when set,
/// only branch keys with one of them count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Tracker {
    #[default]
    None,
    /// `site` is the host, such as `acme.atlassian.net`.
    Jira {
        site: String,
        #[serde(default, deserialize_with = "lenient_keys")]
        keys: Vec<String>,
    },
    /// `workspace` is the URL slug, `acme` in `linear.app/acme/issue/ABC-12`.
    Linear {
        workspace: String,
        #[serde(default, deserialize_with = "lenient_keys")]
        keys: Vec<String>,
    },
}

impl Tracker {
    /// The issue page for a ticket key such as `ABC-12`.
    pub fn ticket_url(&self, key: &str) -> Option<String> {
        match self {
            Tracker::None => None,
            Tracker::Jira { site, .. } => Some(format!("https://{site}/browse/{key}")),
            Tracker::Linear { workspace, .. } => Some(format!("https://linear.app/{workspace}/issue/{key}")),
        }
    }

    fn clamped(self) -> Self {
        let tracker = match self {
            Tracker::Jira { site, keys } => Tracker::Jira { site: jira_host(&site), keys },
            Tracker::Linear { workspace, keys } => Tracker::Linear { workspace: linear_slug(&workspace), keys },
            Tracker::None => Tracker::None,
        };
        match &tracker {
            Tracker::Jira { site: value, .. } | Tracker::Linear { workspace: value, .. } if value.is_empty() => Tracker::None,
            _ => tracker,
        }
    }
}

/// Junk keys are no keys, so the tracker itself still loads.
fn lenient_keys<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(deserializer)?).unwrap_or_default())
}

fn lenient_tracker<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Tracker, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// A malformed entry keeps its id and name, so its secrets and data folder stay reachable.
pub fn lenient_workspaces<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<Workspace>, D::Error> {
    let Value::Array(entries) = Value::deserialize(deserializer)? else { return Ok(Vec::new()) };
    Ok(entries
        .into_iter()
        .filter(Value::is_object)
        .map(|entry| {
            serde_json::from_value(entry.clone()).unwrap_or_else(|_| Workspace {
                id: entry["id"].as_str().unwrap_or_default().into(),
                name: entry["name"].as_str().unwrap_or_default().into(),
                ..Workspace::default()
            })
        })
        .collect())
}

impl Workspace {
    /// Drops watched repos that aren't `owner/name`; `bad_repo` names them before a save.
    pub fn clamped(self) -> Self {
        let teams = self.azure_dev_ops.teams.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        let mut repos: Vec<String> = Vec::new();
        for repo in self.github.repos.iter().filter_map(|r| github_repo(r)) {
            if !repos.iter().any(|r| r.eq_ignore_ascii_case(&repo)) {
                repos.push(repo);
            }
        }
        Self {
            name: self.name.trim().to_string(),
            azure_dev_ops: AdoSource { organization: self.azure_dev_ops.organization.trim().to_string(), teams },
            github: GithubSource { repos },
            tracker: self.tracker.clamped(),
            ..self
        }
    }

    /// The first typed watched repo that isn't `owner/name` or its link.
    pub fn bad_repo(&self) -> Option<&str> {
        self.github.repos.iter().map(|r| r.trim()).find(|r| !r.is_empty() && github_repo(r).is_none())
    }
}

/// `owner/name` from a typed repo or its link: ` https://github.com/acme/web.app/pulls ` gives `acme/web.app`.
/// `None` for anything else, since it goes into a search query.
fn github_repo(typed: &str) -> Option<String> {
    let typed = typed.trim();
    let without_scheme = typed.find("://").map_or(typed, |i| &typed[i + 3..]);
    let path = without_scheme.strip_prefix("github.com/").or_else(|| without_scheme.strip_prefix("www.github.com/")).unwrap_or(without_scheme);
    let mut segments = path.split('/');
    let (owner, name) = (segments.next()?, segments.next()?);
    let safe = |s: &str| !s.is_empty() && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    (safe(owner) && safe(name)).then(|| format!("{owner}/{name}"))
}

/// The host of a typed Jira site: `HTTPS://acme.atlassian.net/jira/boards` gives `acme.atlassian.net`.
fn jira_host(site: &str) -> String {
    let site = site.trim();
    let without_scheme = site.find("://").map_or(site, |i| &site[i + 3..]);
    without_scheme.split(['/', '?', '#', '\\']).next().unwrap_or_default().to_string()
}

/// The slug of a typed Linear workspace: `https://linear.app/Acme/issue/ABC-1` and ` acme ` give `acme`.
/// Empty when it holds anything but letters, digits, `-` and `_`, since it goes into a URL.
fn linear_slug(typed: &str) -> String {
    let typed = typed.trim();
    let without_scheme = typed.find("://").map_or(typed, |i| &typed[i + 3..]);
    let mut segments = without_scheme.split(['/', '?', '#', '\\']);
    let first = segments.next().unwrap_or_default();
    let slug = if first.eq_ignore_ascii_case("linear.app") { segments.next().unwrap_or_default() } else { first };
    if slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') { slug.to_ascii_lowercase() } else { String::new() }
}

/// The settings of the single Azure DevOps org from before workspaces, read from the same file.
#[derive(Debug, Default)]
pub struct Legacy {
    pub organization: String,
    pub teams: Vec<String>,
    pub jira_site: String,
}

impl Legacy {
    pub fn read(file: &Value) -> Self {
        let field = |key: &str| file.get(key).cloned().unwrap_or(Value::Null);
        Self {
            organization: field("organization").as_str().unwrap_or_default().to_string(),
            teams: serde_json::from_value(field("teams")).unwrap_or_default(),
            jira_site: field("jiraSite").as_str().unwrap_or_default().to_string(),
        }
    }

    /// The first workspace: named after the org, else "GitHub" when only a GitHub token was saved, else "Default".
    pub fn into_workspace(self, id: String, has_github_token: bool) -> Workspace {
        let name = match self.organization.trim() {
            "" if has_github_token => "GitHub".to_string(),
            "" => "Default".to_string(),
            org => org.to_string(),
        };
        Workspace {
            id,
            name,
            azure_dev_ops: AdoSource { organization: self.organization, teams: self.teams },
            github: GithubSource::default(),
            tracker: Tracker::Jira { site: self.jira_site, keys: Vec::new() },
        }
        .clamped()
    }
}

/// Gives every workspace a safe, unique id and a name, and points `active` at one of them. True when it changed anything.
pub fn normalize(workspaces: &mut [Workspace], active: &mut String, mut new_id: impl FnMut() -> String) -> bool {
    let mut changed = false;
    let mut seen = std::collections::HashSet::new();
    for (i, ws) in workspaces.iter_mut().enumerate() {
        if !crate::queue::is_workspace_id(&ws.id) || !seen.insert(ws.id.clone()) {
            ws.id = new_id();
            seen.insert(ws.id.clone());
            changed = true;
        }
        if ws.name.trim().is_empty() {
            ws.name = format!("Workspace {}", i + 1);
            changed = true;
        }
    }
    if !workspaces.iter().any(|ws| ws.id == *active)
        && let Some(first) = workspaces.first()
    {
        active.clone_from(&first.id);
        changed = true;
    }
    changed
}

/// Keyring user names; the service is the app's.
pub fn pat_key(id: &str) -> String {
    format!("ado-pat:{id}")
}

pub fn github_key(id: &str) -> String {
    format!("github-token:{id}")
}

/// Before workspaces, one PAT and one GitHub token.
pub const LEGACY_PAT_KEY: &str = "ado-pat";
pub const LEGACY_GITHUB_KEY: &str = "github-token";

/// A short id that is safe as a folder and keyring name.
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}", (nanos as u64).wrapping_add(n))
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn jira(site: &str) -> Tracker {
        Tracker::Jira { site: site.into(), keys: Vec::new() }
    }

    fn linear(workspace: &str) -> Tracker {
        Tracker::Linear { workspace: workspace.into(), keys: Vec::new() }
    }

    #[test]
    fn tracker_jiraSite_keepsOnlyTheHostAndEmptyIsNone() {
        for (typed, expected) in [
            ("acme.atlassian.net", jira("acme.atlassian.net")),
            (" HTTPS://acme.atlassian.net/ ", jira("acme.atlassian.net")),
            ("https://acme.atlassian.net/jira/software/projects/X", jira("acme.atlassian.net")),
            ("https://acme.atlassian.net?atlOrigin=x#top", jira("acme.atlassian.net")),
            (r"acme.atlassian.net\", jira("acme.atlassian.net")),
            ("https://", Tracker::None),
            ("  ", Tracker::None),
        ] {
            assert_eq!(jira(typed).clamped(), expected, "{typed:?}");
        }
    }

    #[test]
    fn tracker_linearWorkspace_keepsOnlyTheSlugAndEmptyOrUnsafeIsNone() {
        for (typed, expected) in [
            ("acme", linear("acme")),
            (" Acme-Labs ", linear("acme-labs")),
            ("https://linear.app/acme/issue/ABC-12/title", linear("acme")),
            ("Linear.app/acme", linear("acme")),
            ("https://linear.app/", Tracker::None),
            ("acme corp", Tracker::None),
            ("acme?x=1", linear("acme")),
            ("  ", Tracker::None),
        ] {
            assert_eq!(linear(typed).clamped(), expected, "{typed:?}");
        }
    }

    #[test]
    fn tracker_emptyKeys_areStillWritten() {
        assert_eq!(serde_json::to_value(jira("acme.atlassian.net")).unwrap()["keys"], serde_json::json!([]));
    }

    #[test]
    fn tracker_ticketUrl_linksToTheIssueOfEachTracker() {
        assert_eq!(jira("acme.atlassian.net").ticket_url("ABC-12").as_deref(), Some("https://acme.atlassian.net/browse/ABC-12"));
        assert_eq!(linear("acme").ticket_url("ENG-7").as_deref(), Some("https://linear.app/acme/issue/ENG-7"));
        assert_eq!(Tracker::None.ticket_url("ABC-12"), None);
    }

    #[test]
    fn tracker_storedForms_loadAndJunkIsNone() {
        let tracker = |json: &str| {
            #[derive(Deserialize)]
            struct File {
                #[serde(deserialize_with = "lenient_tracker")]
                tracker: Tracker,
            }
            serde_json::from_str::<File>(&format!(r#"{{"tracker":{json}}}"#)).unwrap().tracker
        };
        assert_eq!(tracker(r#"{"kind":"jira","site":"acme.atlassian.net"}"#), jira("acme.atlassian.net"));
        assert_eq!(tracker(r#"{"kind":"linear","workspace":"acme"}"#), linear("acme"));
        assert_eq!(tracker(r#"{"kind":"linear"}"#), Tracker::None);
        assert_eq!(
            tracker(r#"{"kind":"linear","workspace":"acme","keys":["HOO"]}"#),
            Tracker::Linear { workspace: "acme".into(), keys: vec!["HOO".into()] }
        );
        assert_eq!(tracker(r#"{"kind":"jira","site":"acme.atlassian.net","keys":"HOO"}"#), jira("acme.atlassian.net"));
        assert_eq!(tracker(r#"{"kind":"youtrack","site":"x"}"#), Tracker::None);
    }

    #[test]
    fn workspaces_malformedEntry_keepsItsIdAndName() {
        #[derive(Deserialize)]
        struct File {
            #[serde(deserialize_with = "lenient_workspaces")]
            workspaces: Vec<Workspace>,
        }
        let json = r#"{"workspaces":[{"id":"a1","name":"Acme","azureDevOps":{"teams":7}},7,{"id":"b2","name":"Ok","tracker":"junk"}]}"#;
        let file: File = serde_json::from_str(json).unwrap();
        assert_eq!(file.workspaces.iter().map(|w| (w.id.as_str(), w.name.as_str())).collect::<Vec<_>>(), [("a1", "Acme"), ("b2", "Ok")]);
        assert_eq!(file.workspaces[1].tracker, Tracker::None);
    }

    #[test]
    fn workspace_githubRepos_keepOwnerAndNameAndRejectTheRest() {
        let with = |repos: &[&str]| Workspace { github: GithubSource { repos: repos.iter().map(|r| r.to_string()).collect() }, ..Workspace::default() };
        let good = with(&[" coding-tree-io/hoopwise.ai ", "https://github.com/acme/web/pulls", "github.com/Acme/Web", "", "o/r_1"]);
        assert_eq!(good.bad_repo(), None);
        assert_eq!(good.clamped().github.repos, ["coding-tree-io/hoopwise.ai", "acme/web", "o/r_1"]);
        for bad in ["hoopwise.ai", "acme/web app", "acme/", "acme/.x", "o/r:x"] {
            let typed = with(&["o/r", bad]);
            assert_eq!(typed.bad_repo(), Some(bad));
            assert_eq!(typed.clamped().github.repos, ["o/r"], "{bad}");
        }
    }

    #[test]
    fn workspace_withoutGithub_loadsWithNoReposAndWritesThem() {
        let ws: Workspace = serde_json::from_str(r#"{"id":"a1","name":"A","azureDevOps":{"organization":"o","teams":[]}}"#).unwrap();
        assert!(ws.github.repos.is_empty());
        assert_eq!(serde_json::to_value(&ws).unwrap()["github"], serde_json::json!({ "repos": [] }));
    }

    #[test]
    fn workspaces_notAList_isEmpty() {
        #[derive(Deserialize)]
        struct File {
            #[serde(deserialize_with = "lenient_workspaces")]
            workspaces: Vec<Workspace>,
        }
        assert!(serde_json::from_str::<File>(r#"{"workspaces":"x"}"#).unwrap().workspaces.is_empty());
    }

    #[test]
    fn normalize_badOrDuplicateIds_getNewOnesAndActiveFallsBackToTheFirst() {
        let ws = |id: &str, name: &str| Workspace { id: id.into(), name: name.into(), ..Workspace::default() };
        let mut list = vec![ws("a", "A"), ws("a", ""), ws("../x", "X")];
        let mut active = "gone".to_string();
        let mut n = 0;
        assert!(normalize(&mut list, &mut active, || {
            n += 1;
            format!("new{n}")
        }));
        assert_eq!(list.iter().map(|w| (w.id.as_str(), w.name.as_str())).collect::<Vec<_>>(), [("a", "A"), ("new1", "Workspace 2"), ("new2", "X")]);
        assert_eq!(active, "a");
        assert!(!normalize(&mut list, &mut active, || unreachable!()));
    }

    #[test]
    fn legacy_namesTheWorkspaceAfterTheOrgThenGitHubThenDefault() {
        let legacy = |org: &str| Legacy { organization: org.into(), ..Legacy::default() };
        assert_eq!(legacy("contoso").into_workspace("i".into(), true).name, "contoso");
        assert_eq!(legacy("").into_workspace("i".into(), true).name, "GitHub");
        assert_eq!(legacy(" ").into_workspace("i".into(), false).name, "Default");
    }

    #[test]
    fn newId_isAWorkspaceIdAndUnique() {
        let (a, b) = (new_id(), new_id());
        assert!(crate::queue::is_workspace_id(&a));
        assert_ne!(a, b);
    }
}
