//! The workspace session: the settings and workspaces, what each workspace has seen, and every rule about when a
//! poll is stale, when a source's baseline resets and when the poller must wake. It owns the `Store`, so the files
//! and secrets it writes change together with the state they belong to. Nothing here knows about Tauri: the app's
//! commands call one method each and then emit the new status, wake the poller if `take_wake` says so, and show what
//! `apply_poll` asks to announce.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::score::{self, Provider, ReviewItem, VoteMemory};
use crate::store::{self, Settings, Store};
use crate::workspace::{self, Tracker, Workspace};
use crate::{ado, export, github, group, queue};

/// What one workspace has seen. The active workspace's lives in `Session::ws`; the others wait in `Session::parked`
/// until switched back to, so their lists, vote memory and baselines survive a switch.
#[derive(Default)]
struct WorkspaceState {
    items: Vec<ReviewItem>,
    urls: HashMap<i64, String>,
    /// Each listed PR's head commit, which a draft mark must match.
    commits: HashMap<i64, String>,
    /// The listed PRs an agent drafted at their head commit, with the draft's link; read at each poll.
    agent_drafts: HashMap<i64, Option<String>>,
    memory: VoteMemory,
    hidden: HashSet<i64>,
    related: Vec<group::Related>,
    last_refreshed: Option<DateTime<Utc>>,
    error: Option<String>,
    /// Per source, the PRs already waiting on me at its last good poll. A source with no baseline yet records
    /// its PRs without notifying, so a new token, a recovered source or a first visit doesn't announce everything at once.
    baseline: HashMap<Provider, HashSet<i64>>,
    /// The source of each hidden PR, saved with it, so a closed one is pruned only by its own source.
    hidden_source: HashMap<i64, Provider>,
    /// Each source's last good poll in this run.
    source_refreshed: HashMap<Provider, DateTime<Utc>>,
}

impl WorkspaceState {
    fn load(store: &Store, id: &str) -> Self {
        let hidden = store.hidden(id);
        Self {
            memory: store.memory(id),
            hidden: hidden.keys().copied().collect(),
            hidden_source: hidden.into_iter().filter_map(|(id, source)| Some((id, source?))).collect(),
            ..Self::default()
        }
    }
}

/// Which secrets a workspace has saved; the values never leave the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pat: bool,
    github_token: bool,
}

impl Saved {
    fn set(&mut self, provider: Provider, saved: bool) {
        match provider {
            Provider::Ado => self.pat = saved,
            Provider::GitHub => self.github_token = saved,
        }
    }

    fn any(self) -> bool {
        self.pat || self.github_token
    }
}

/// The session's part of the snapshot the windows render.
#[derive(Clone, Serialize, ts_rs::TS)]
#[ts(rename = "SessionStatus")]
#[serde(rename_all = "camelCase")]
pub struct Status {
    items: Vec<group::ReviewRow>,
    /// The counter's numbers for `items`.
    counts: group::Counts,
    last_refreshed: Option<DateTime<Utc>>,
    polling: bool,
    error: Option<String>,
    has_pat: bool,
    has_github_token: bool,
    settings: Settings,
    /// By workspace id.
    saved: HashMap<String, Saved>,
}

/// One source's result for a poll.
pub enum Poll<T> {
    /// No token for this source.
    Off,
    /// Its PRs from the last good poll stay listed.
    Failed(String),
    Fresh(T),
}

impl<T> Poll<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Poll<U> {
        match self {
            Poll::Off => Poll::Off,
            Poll::Failed(error) => Poll::Failed(error),
            Poll::Fresh(answer) => Poll::Fresh(f(answer)),
        }
    }
}

/// What a poll needs, read under the lock with the generation it belongs to.
pub struct Ticket {
    pub workspace: Workspace,
    pub generation: u64,
    pub pat: Result<Option<String>, String>,
    pub github_token: Result<Option<String>, String>,
    /// Approved or hidden Azure DevOps PRs whose threads are already known, by the head commit they were seen at:
    /// their threads only matter for the row's comment count and activity, so they load again after a push.
    pub settled: HashMap<i64, String>,
}

/// What a filed poll asks the app to show.
#[derive(Default)]
pub struct Outcome {
    /// Newly waiting on me since the workspace's last poll.
    pub fresh: Vec<ReviewItem>,
    /// Whether to raise a notification for `fresh`, per the settings.
    pub notify: bool,
}

/// What a general settings save changed that only the app's windows can act on.
#[derive(Debug, Default, PartialEq)]
pub struct SettingsChange {
    /// The counter's new size, when it changed.
    pub pill_scale: Option<u32>,
    pub list_resized: bool,
}

/// What a poll's result asks for: files to save, and who to notify.
#[derive(Default)]
struct Applied {
    outcome: Outcome,
    memory: Option<VoteMemory>,
    hidden: Option<store::Hidden>,
    queue: Option<queue::Queue>,
}

pub struct Session {
    store: Store,
    settings: Settings,
    ws: WorkspaceState,
    parked: HashMap<String, WorkspaceState>,
    saved: HashMap<String, Saved>,
    polling: bool,
    /// Changes when a poll in flight becomes stale: a switch, a new or removed secret, a changed source.
    generation: u64,
    /// Set when the poller must run now rather than at its next interval; drained by `take_wake`.
    wake: bool,
}

/// A token that couldn't be read counts as saved: its source then fails with the read error instead of turning off,
/// which would ask for a token that exists and drop the source's hidden PRs.
fn token_saved(read: &Result<Option<String>, String>) -> bool {
    !matches!(read, Ok(None))
}

impl Session {
    /// Reads the settings, which secrets each workspace has, and the active workspace's files.
    /// The warning says what went wrong loading the settings.
    pub fn load(store: Store) -> (Self, Option<String>) {
        let loaded = store.load();
        let saved = loaded
            .settings
            .workspaces
            .iter()
            .map(|w| {
                let (pat, token) = (store.read_pat(&w.id), store.read_github_token(&w.id));
                (w.id.clone(), Saved { pat: token_saved(&pat), github_token: token_saved(&token) })
            })
            .collect();
        let ws = WorkspaceState::load(&store, &loaded.settings.active_workspace);
        let session = Self { store, settings: loaded.settings, ws, parked: HashMap::new(), saved, polling: false, generation: 0, wake: false };
        (session, loaded.warning)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// True once after any change that needs a poll now.
    pub fn take_wake(&mut self) -> bool {
        std::mem::take(&mut self.wake)
    }

    pub fn status(&self) -> Status {
        let saved = self.active_saved();
        let items = self.rows();
        Status {
            counts: group::counts(&items),
            items,
            last_refreshed: self.ws.last_refreshed,
            polling: self.polling,
            error: self.ws.error.clone(),
            has_pat: saved.pat,
            has_github_token: saved.github_token,
            settings: self.settings.clone(),
            saved: self.saved.clone(),
        }
    }

    fn rows(&self) -> Vec<group::ReviewRow> {
        let tracker = self.settings.active().map(|w| &w.tracker).unwrap_or(&Tracker::None);
        let mut rows = group::group(&self.ws.items, &self.ws.related, &self.ws.urls, tracker);
        for member in rows.iter_mut().flat_map(|r| &mut r.members) {
            member.agent_draft = self.ws.agent_drafts.get(&member.id).map(|url| group::AgentDraft { host: url.as_deref().map(|u| queue::url_authority(u).to_string()) });
        }
        rows
    }

    pub fn row(&self, key: &str) -> Result<group::ReviewRow, String> {
        self.rows().into_iter().find(|r| r.key == key).ok_or_else(|| "That row is gone; refresh the list".to_string())
    }

    pub fn url(&self, id: i64) -> Option<String> {
        self.ws.urls.get(&id).cloned()
    }

    pub fn draft_url(&self, id: i64) -> Result<String, String> {
        self.ws.agent_drafts.get(&id).cloned().flatten().ok_or_else(|| "That PR has no linked draft now; refresh the list".to_string())
    }

    /// The tracker link for the ticket a row's branch names.
    pub fn ticket_url(&self, key: &str) -> Result<String, String> {
        let row = self.row(key)?;
        let tracker = self.settings.active().map(|w| w.tracker.clone()).unwrap_or_default();
        let ticket = row.ticket.ok_or("That row's branch names no ticket")?;
        Ok(tracker.ticket_url(&ticket).ok_or("Set this workspace's tracker in settings")?)
    }

    /// Shows `error` in the status line until the next poll replaces it.
    pub fn report(&mut self, error: String) {
        self.ws.error = Some(error);
    }

    fn active_saved(&self) -> Saved {
        self.saved.get(&self.settings.active_workspace).copied().unwrap_or_default()
    }

    fn known(&self, id: &str) -> Result<(), String> {
        self.settings.workspaces.iter().any(|w| w.id == id).then_some(()).ok_or_else(|| "That workspace is gone".to_string())
    }

    /// A workspace's state, active or parked.
    fn state_of(&mut self, id: &str) -> Option<&mut WorkspaceState> {
        if id == self.settings.active_workspace { Some(&mut self.ws) } else { self.parked.get_mut(id) }
    }

    /// Makes `id` the active workspace, parking the current one and reading `id`'s files if it wasn't visited since
    /// start. Makes the poll in flight stale and wakes the poller.
    fn switch_to(&mut self, id: &str) {
        if id == self.settings.active_workspace || self.known(id).is_err() {
            return;
        }
        let target = self.parked.remove(id).unwrap_or_else(|| WorkspaceState::load(&self.store, id));
        let previous = std::mem::replace(&mut self.ws, target);
        let previous_id = std::mem::replace(&mut self.settings.active_workspace, id.to_string());
        self.parked.insert(previous_id, previous);
        self.generation += 1;
        self.wake = true;
        self.polling = self.active_saved().any();
    }

    /// Starts a poll of the active workspace: None when there is nothing to poll. With no token saved, the
    /// workspace's rows and queue go, and the status line asks for a token.
    pub fn begin_poll(&mut self) -> Option<Ticket> {
        let workspace = self.settings.active()?.clone();
        let (pat, github_token) = (self.store.read_pat(&workspace.id), self.store.read_github_token(&workspace.id));
        self.saved.insert(workspace.id.clone(), Saved { pat: token_saved(&pat), github_token: token_saved(&github_token) });
        if matches!((&pat, &github_token), (Ok(None), Ok(None))) {
            self.polling = false;
            // After the last token is removed, its rows must not stay listed, nor queued for an agent, as if current.
            let ws = &mut self.ws;
            ws.items.clear();
            ws.urls.clear();
            ws.commits.clear();
            ws.agent_drafts.clear();
            ws.related.clear();
            ws.source_refreshed.clear();
            ws.last_refreshed = None;
            // Without the file the MCP server says to add a token, rather than reporting an empty queue.
            let _ = self.store.remove_queue(&workspace.id);
            ws.error = Some("Add an Azure DevOps PAT or a GitHub token to this workspace in settings".into());
            return None;
        }
        self.polling = true;
        let settled = self
            .ws
            .items
            .iter()
            .filter(|i| i.provider == Provider::Ado && (i.hidden || i.state == score::ReviewState::Approved) && i.comments.is_some())
            .filter_map(|i| Some((i.id, self.ws.commits.get(&i.id)?.clone())))
            .collect();
        Some(Ticket { workspace, generation: self.generation, pat, github_token, settled })
    }

    /// Files a poll's result under the active workspace and saves what it changed. Nothing happens when the poll is
    /// stale: a switch, a new token or a changed source since `generation` was handed out.
    pub fn apply_poll(&mut self, generation: u64, ado: Poll<ado::Fetched>, github: Poll<github::Fetched>, now: DateTime<Utc>) -> Outcome {
        let Some(applied) = self.file_poll(generation, ado, github, now) else { return Outcome::default() };
        let id = &self.settings.active_workspace;
        if let Some(memory) = &applied.memory {
            let _ = self.store.save_memory(id, memory);
        }
        if let Some(hidden) = &applied.hidden {
            let _ = self.store.save_hidden(id, hidden);
        }
        if let Some(queue) = &applied.queue {
            let _ = self.store.save_queue(id, queue);
        }
        applied.outcome
    }

    fn file_poll(&mut self, generation: u64, ado: Poll<ado::Fetched>, github: Poll<github::Fetched>, now: DateTime<Utc>) -> Option<Applied> {
        // Before digesting: Azure DevOps's digest updates the vote memory, which a stale poll must not touch.
        if self.generation != generation {
            return None;
        }
        let mut applied = Applied::default();
        let organization = self.settings.active().map(|w| w.azure_dev_ops.organization.clone()).unwrap_or_default();
        let ws = &mut self.ws;
        let (old_items, old_urls, old_commits) = (std::mem::take(&mut ws.items), std::mem::take(&mut ws.urls), std::mem::take(&mut ws.commits));
        let previous = |provider: Provider| -> HashMap<i64, &ReviewItem> { old_items.iter().filter(|i| i.provider == provider).map(|i| (i.id, i)).collect() };
        let mut items = Vec::new();
        let mut urls = HashMap::new();
        let mut commits = HashMap::new();
        let mut errors = Vec::new();
        let mut fresh_sources = Vec::new();
        // Sources whose full list is in hand (fresh without a warning), and sources with no token saved.
        let mut answered: HashSet<Provider> = HashSet::new();
        let mut off: HashSet<Provider> = HashSet::new();
        let mut refs = HashMap::new();
        let mut missing = Vec::new();
        // In this order: the status line's errors, the queue's missing sources and the announcements follow it.
        let polls = [
            (Provider::Ado, ado.map(|fetched| ado::digest(fetched, &mut ws.memory, now))),
            (Provider::GitHub, github.map(|fetched| github::digest(fetched, now))),
        ];
        for (provider, poll) in polls {
            match poll {
                Poll::Off => {
                    ws.related.retain(|r| r.provider != provider);
                    ws.baseline.remove(&provider);
                    off.insert(provider);
                }
                // Its PRs, links and related PRs from the last good poll stay.
                Poll::Failed(error) => {
                    errors.push(error);
                    missing.push(export::source_name(provider).to_string());
                    for item in old_items.iter().filter(|i| i.provider == provider) {
                        if let Some(url) = old_urls.get(&item.id) {
                            urls.insert(item.id, url.clone());
                        }
                        if let Some(commit) = old_commits.get(&item.id) {
                            commits.insert(item.id, commit.clone());
                        }
                        items.push(item.clone());
                    }
                }
                Poll::Fresh(mut poll) => {
                    fresh_sources.push((provider, poll.warning.is_some()));
                    match poll.warning {
                        Some(warning) => errors.push(warning),
                        None => {
                            answered.insert(provider);
                        }
                    }
                    if poll.queue_partial {
                        missing.push(export::source_name(provider).to_string());
                    }
                    // Only Azure DevOps keeps a vote memory; its digest has just updated it.
                    if provider == Provider::Ado {
                        applied.memory = Some(ws.memory.clone());
                    }
                    score::attach_threads(&mut poll.items, &poll.threads, &previous(provider), now);
                    ws.related.retain(|r| r.provider != provider);
                    ws.related.extend(poll.related);
                    urls.extend(poll.urls);
                    items.extend(poll.items);
                    commits.extend(poll.queue_refs.iter().filter_map(|(id, r)| Some((*id, r.source_commit.clone()?))));
                    refs.insert(provider, poll.queue_refs);
                }
            }
        }
        items.sort_by(|a, b| b.score.cmp(&a.score).then(b.age_hours.cmp(&a.age_hours)));
        ws.items = items;
        ws.urls = urls;
        ws.agent_drafts = self.store.agent_drafts(&self.settings.active_workspace, &commits);
        ws.commits = commits;
        self.polling = false;
        ws.error = (!errors.is_empty()).then(|| errors.join(" · "));
        for (source, _) in &fresh_sources {
            ws.source_refreshed.insert(*source, now);
        }
        // A token removed and saved again must not bring back the time of its old answer.
        ws.source_refreshed.retain(|source, _| !off.contains(source));
        ws.last_refreshed = oldest_refresh(&ws.source_refreshed, &off);
        if fresh_sources.is_empty() {
            return Some(applied);
        }
        if prune_hidden(ws, &answered, &off) {
            applied.hidden = Some(hidden_file(ws));
        }
        mark_hidden(ws);
        for (source, partial) in fresh_sources {
            let listed: Vec<&ReviewItem> = ws.items.iter().filter(|i| i.provider == source).collect();
            let (baseline, new) = next_baseline(ws.baseline.get(&source), &listed, partial);
            applied.outcome.fresh.extend(listed.into_iter().filter(|i| new.contains(&i.id)).cloned());
            ws.baseline.insert(source, baseline);
        }
        applied.outcome.notify = self.settings.notify;
        // At least one source is fresh here; a failed one is named in the queue so an agent knows its PRs are missing.
        let ctx = export::Context { organization: &organization, poll_minutes: self.settings.poll_minutes, now };
        applied.queue = Some(export::queue(&ws.items, &export::Sources { refs, missing }, &ws.urls, &ctx));
        Some(applied)
    }

    /// Hides or shows every PR of mine in the row, and saves the hidden list.
    pub fn set_hidden(&mut self, key: &str, hidden: bool) -> Result<(), String> {
        let row = self.row(key)?;
        let ws = &mut self.ws;
        for (id, provider) in row.members.iter().filter(|m| m.state.is_some()).map(|m| (m.id, m.provider)) {
            if hidden {
                ws.hidden.insert(id);
                ws.hidden_source.insert(id, provider);
            } else {
                ws.hidden.remove(&id);
                ws.hidden_source.remove(&id);
                // Unhiding is not news: keep it out of the next poll's notification.
                if let Some(baseline) = ws.baseline.get_mut(&provider) {
                    baseline.insert(id);
                }
            }
        }
        mark_hidden(ws);
        self.store.save_hidden(&self.settings.active_workspace, &hidden_file(&self.ws))
    }

    /// Saves a workspace's PAT or GitHub token, trimmed.
    pub fn save_secret(&mut self, id: &str, provider: Provider, value: &str) -> Result<(), String> {
        let value = value.trim();
        if value.is_empty() {
            return Err(match provider {
                Provider::Ado => "The PAT is empty",
                Provider::GitHub => "The GitHub token is empty",
            }
            .into());
        }
        self.known(id)?;
        match provider {
            Provider::Ado => self.store.save_pat(id, value)?,
            Provider::GitHub => self.store.save_github_token(id, value)?,
        }
        self.secret_changed(id, provider, true);
        Ok(())
    }

    /// Turns a workspace's source off; its rows leave at the next poll.
    pub fn remove_secret(&mut self, id: &str, provider: Provider) -> Result<(), String> {
        self.known(id)?;
        match provider {
            Provider::Ado => self.store.remove_pat(id)?,
            Provider::GitHub => self.store.remove_github_token(id)?,
        }
        self.secret_changed(id, provider, false);
        Ok(())
    }

    /// After a secret for `id` is saved or removed: a poll for it running now is stale, and its next good poll is a baseline.
    fn secret_changed(&mut self, id: &str, provider: Provider, now_saved: bool) {
        self.saved.entry(id.to_string()).or_default().set(provider, now_saved);
        if id == self.settings.active_workspace {
            self.ws.error = None;
            self.generation += 1;
            self.wake = true;
        }
        if let Some(state) = self.state_of(id) {
            state.baseline.remove(&provider);
        }
    }

    /// Saves the general settings; the workspaces change only through their own methods. A poll interval change wakes
    /// the poller; a size or notification change needs no new data.
    pub fn save_settings(&mut self, settings: Settings) -> Result<SettingsChange, String> {
        let settings = Settings {
            workspaces: self.settings.workspaces.clone(),
            active_workspace: self.settings.active_workspace.clone(),
            migrating_to: self.settings.migrating_to.clone(),
            ..settings
        }
        .clamped();
        self.store.save_settings(&settings)?;
        let old = &self.settings;
        let change = SettingsChange {
            pill_scale: (old.pill_scale != settings.pill_scale).then_some(settings.pill_scale),
            list_resized: (old.list_width_percent, old.list_height_percent) != (settings.list_width_percent, settings.list_height_percent),
        };
        self.wake |= old.poll_minutes != settings.poll_minutes;
        self.settings = settings;
        Ok(change)
    }

    /// Applies `change` and saves the settings. A failed save leaves the change in place for this run and says so.
    fn edit_workspaces<T>(&mut self, change: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let value = change(self)?;
        self.store.save_settings(&self.settings).map_err(|e| format!("Changed until Rado quits, but not saved: {e}"))?;
        Ok(value)
    }

    /// Adds a workspace and returns its id.
    pub fn add_workspace(&mut self, name: &str) -> Result<String, String> {
        self.edit_workspaces(|s| {
            let id = workspace::new_id();
            let name = match name.trim() {
                "" => format!("Workspace {}", s.settings.workspaces.len() + 1),
                name => name.to_string(),
            };
            s.settings.workspaces.push(Workspace { id: id.clone(), name, ..Workspace::default() });
            Ok(id)
        })
    }

    /// Saves a workspace's name, Azure DevOps organization and teams, watched GitHub repos, and tracker. A changed
    /// source starts a new baseline, and on the active workspace makes the poll in flight stale and polls again.
    /// Written before it is applied: any error means nothing changed, which is what the editor's rollback relies on.
    pub fn save_workspace(&mut self, workspace: Workspace) -> Result<(), String> {
        if let Some(repo) = workspace.bad_repo() {
            return Err(format!("Not saved: “{repo}” isn't a GitHub repo (owner/name)"));
        }
        let typed_tracker = workspace.tracker != Tracker::None;
        let mut workspace = workspace.clamped();
        if typed_tracker && workspace.tracker == Tracker::None {
            return Err("Not saved: that isn't a Jira site or a Linear workspace".into());
        }
        let mut settings = self.settings.clone();
        let slot = settings.workspaces.iter_mut().find(|w| w.id == workspace.id).ok_or("Not saved: that workspace is gone")?;
        if workspace.name.is_empty() {
            workspace.name.clone_from(&slot.name);
        }
        let ado_changed = slot.azure_dev_ops != workspace.azure_dev_ops;
        let github_changed = slot.github != workspace.github;
        let id = workspace.id.clone();
        *slot = workspace;
        self.store.save_settings(&settings).map_err(|e| format!("Not saved: {e}"))?;
        self.settings = settings;
        if let Some(state) = self.state_of(&id) {
            if ado_changed {
                state.baseline.remove(&Provider::Ado);
            }
            if github_changed {
                state.baseline.remove(&Provider::GitHub);
            }
        }
        if id == self.settings.active_workspace && (ado_changed || github_changed) {
            self.generation += 1;
            self.wake = true;
        }
        Ok(())
    }

    pub fn switch(&mut self, id: &str) -> Result<(), String> {
        self.edit_workspaces(|s| {
            s.known(id)?;
            s.switch_to(id);
            Ok(())
        })
    }

    /// Deletes a workspace with its secrets and data. The last one stays; deleting the active one switches to its neighbour.
    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        let mut removed = false;
        let result = self.edit_workspaces(|s| {
            let list = &s.settings.workspaces;
            let index = list.iter().position(|w| w.id == id).ok_or("That workspace is gone")?;
            if list.len() == 1 {
                return Err("Keep at least one workspace".into());
            }
            if id == s.settings.active_workspace {
                let neighbour = list[if index + 1 < list.len() { index + 1 } else { index - 1 }].id.clone();
                s.switch_to(&neighbour);
            }
            s.settings.workspaces.remove(index);
            s.parked.remove(id);
            s.saved.remove(id);
            removed = true;
            Ok(())
        });
        // Its secrets go even when the save failed: a PAT left with nothing pointing at it would never be removed.
        if removed {
            self.store.forget(id).map_err(|e| format!("Deleted the workspace, but its saved data stayed behind: {e}"))?;
        }
        result
    }
}

/// One source's PRs waiting on me after a poll, and which of them are new since `baseline` (none when the
/// source has no baseline yet). A partial answer keeps baseline PRs it didn't list, so they aren't announced
/// again when they reappear; a PR it did list is judged on what it says now.
fn next_baseline(baseline: Option<&HashSet<i64>>, listed: &[&ReviewItem], partial: bool) -> (HashSet<i64>, HashSet<i64>) {
    let mut waiting: HashSet<i64> = listed.iter().filter(|i| i.actionable && !i.hidden).map(|i| i.id).collect();
    let Some(baseline) = baseline else { return (waiting, HashSet::new()) };
    let new = waiting.difference(baseline).copied().collect();
    if partial {
        let present: HashSet<i64> = listed.iter().map(|i| i.id).collect();
        waiting.extend(baseline.iter().filter(|id| !present.contains(id)));
    }
    (waiting, new)
}

/// Drops hidden PRs that closed: ones no longer listed by their source's full answer. A source that is off never
/// answered, so its hidden PRs wait for it. A PR with no known source (hidden before sources were saved) goes once
/// every source either answered or is off. True when the saved files must change.
fn prune_hidden(ws: &mut WorkspaceState, answered: &HashSet<Provider>, off: &HashSet<Provider>) -> bool {
    let mut changed = false;
    for item in ws.items.iter().filter(|i| ws.hidden.contains(&i.id)) {
        changed |= ws.hidden_source.insert(item.id, item.provider) != Some(item.provider);
    }
    let every = [Provider::Ado, Provider::GitHub].iter().all(|p| answered.contains(p) || off.contains(p));
    let before = ws.hidden.len();
    let (urls, sources) = (&ws.urls, &ws.hidden_source);
    ws.hidden.retain(|id| urls.contains_key(id) || !sources.get(id).map_or(every, |p| answered.contains(p)));
    let hidden = &ws.hidden;
    ws.hidden_source.retain(|id, _| hidden.contains(id));
    changed || ws.hidden.len() != before
}

fn hidden_file(ws: &WorkspaceState) -> store::Hidden {
    ws.hidden.iter().map(|id| (*id, ws.hidden_source.get(id).copied())).collect()
}

/// How old the listed data is: the oldest last good poll among sources with a token, so one failing source isn't
/// hidden by another's fresh answer. None while such a source has never answered in this run.
fn oldest_refresh(refreshed: &HashMap<Provider, DateTime<Utc>>, off: &HashSet<Provider>) -> Option<DateTime<Utc>> {
    [Provider::Ado, Provider::GitHub]
        .iter()
        .filter(|p| !off.contains(p))
        .map(|p| refreshed.get(p).copied())
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .min()
}

fn mark_hidden(ws: &mut WorkspaceState) {
    for item in &mut ws.items {
        item.hidden = ws.hidden.contains(&item.id);
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::score::{PrFacts, ReviewState};
    use crate::store::tests::{MemorySecrets, TempDir};
    use crate::workspace::github_key;

    /// An Azure DevOps PR on `branch`, with me (`me-id`) as a reviewer who cast `vote`, or not a reviewer at all.
    fn ado_pr(id: i64, branch: &str, my_vote: Option<i32>) -> ado::PullRequest {
        let reviewers = my_vote
            .map(|vote| ado::Reviewer { id: "me-id".into(), display_name: "Me".into(), vote, is_required: false, is_container: false, has_declined: false })
            .into_iter()
            .collect();
        ado::PullRequest {
            pull_request_id: id,
            title: format!("PR {id}"),
            created_by: ado::IdentityRef { id: "author-id".into(), display_name: "Author".into() },
            creation_date: Utc::now() - chrono::Duration::hours(8),
            repository: ado::Repository {
                id: "repo-id".into(),
                name: "Api".into(),
                project: ado::Project { id: "project-id".into(), name: "Contoso".into() },
            },
            source_ref_name: format!("refs/heads/{branch}"),
            target_ref_name: "refs/heads/dev".into(),
            is_draft: false,
            reviewers,
            last_merge_source_commit: Some(ado::Commit { commit_id: "sha1".into() }),
        }
    }

    fn ado(prs: Vec<ado::PullRequest>) -> Poll<ado::Fetched> {
        let urls = prs.iter().map(|pr| (pr.pull_request_id, format!("https://ado/{}", pr.pull_request_id))).collect();
        Poll::Fresh(ado::Fetched { prs, my_id: "me-id".into(), teams: HashSet::new(), threads: HashMap::new(), urls })
    }

    fn related_ids(s: &Session) -> Vec<i64> {
        s.ws.related.iter().map(|r| r.id).collect()
    }

    fn pr(id: i64, state: ReviewState) -> ReviewItem {
        let facts = PrFacts {
            id,
            provider: Provider::GitHub,
            number: id,
            title: String::new(),
            repo: String::new(),
            branch: format!("b{id}"),
            author: String::new(),
            target: "main".into(),
            created_at: Utc::now(),
            draft: false,
            approvals: 0,
            objections: 0,
        };
        score::review_item(facts, state, false, false, Utc::now())
    }

    fn ids(list: &[i64]) -> HashSet<i64> {
        list.iter().copied().collect()
    }

    /// A session over a temporary folder and in-memory secrets; the folder goes when this is dropped.
    struct Fixture {
        s: Session,
        secrets: MemorySecrets,
        dir: TempDir,
    }

    /// Two workspaces, `a` (active) and `b`, both with a GitHub token.
    fn two_workspaces(name: &str) -> Fixture {
        let dir = TempDir::new(&format!("session-{name}"));
        let secrets = MemorySecrets::with(&[(&github_key("a"), "gh-a"), (&github_key("b"), "gh-b")]);
        let ws = |id: &str| Workspace { id: id.into(), name: id.to_uppercase(), ..Workspace::default() };
        let saved = Saved { pat: false, github_token: true };
        let s = Session {
            store: Store::new(&dir.0, Box::new(secrets.clone())),
            settings: Settings { workspaces: vec![ws("a"), ws("b")], active_workspace: "a".into(), ..Settings::default() },
            ws: WorkspaceState::default(),
            parked: HashMap::new(),
            saved: HashMap::from([("a".into(), saved), ("b".into(), saved)]),
            polling: false,
            generation: 0,
            wake: false,
        };
        Fixture { s, secrets, dir }
    }

    /// Files a GitHub poll listing `prs` at the current generation and returns the ids it would announce.
    fn poll(s: &mut Session, prs: &[i64]) -> HashSet<i64> {
        let generation = s.generation;
        s.apply_poll(generation, Poll::Off, github(prs), Utc::now()).fresh.iter().map(|i| i.id).collect()
    }

    /// Files a poll at the current generation and returns what it would save.
    fn file(s: &mut Session, ado: Poll<ado::Fetched>, github: Poll<github::Fetched>) -> Applied {
        let generation = s.generation;
        s.file_poll(generation, ado, github, Utc::now()).expect("current poll")
    }

    fn github(prs: &[i64]) -> Poll<github::Fetched> {
        Poll::Fresh(github::tests::requested(prs))
    }

    fn listed(s: &Session) -> HashSet<i64> {
        s.ws.items.iter().map(|i| i.id).collect()
    }

    fn complete(sources: &[Provider]) -> HashSet<Provider> {
        sources.iter().copied().collect()
    }

    /// Hidden 1 (Azure DevOps, still listed by a failed source), 2 (GitHub, closed) and 3 (source never seen, closed).
    fn hidden_state() -> WorkspaceState {
        let ado = ReviewItem { provider: Provider::Ado, ..pr(1, ReviewState::NeedsReview) };
        let mut ws = WorkspaceState { items: vec![ado], hidden: ids(&[1, 2, 3]), ..WorkspaceState::default() };
        ws.hidden_source.insert(2, Provider::GitHub);
        ws
    }

    // Agent drafts.

    /// Writes `agent-drafts.json` and `agent-draft-links.json` for workspace `a` by hand, as `rado-mcp` would.
    fn agent_files(dir: &TempDir, drafts: &[(i64, &str)], links: &[(i64, &str, &str)]) {
        let folder = queue::workspace_dir(&dir.0, "a");
        std::fs::create_dir_all(&folder).unwrap();
        let drafts: queue::Drafts = drafts.iter().map(|(id, commit)| (*id, commit.to_string())).collect();
        let links: queue::DraftLinks =
            links.iter().map(|(id, commit, url)| (*id, queue::DraftLink { commit: commit.to_string(), url: url.to_string() })).collect();
        queue::write_json_atomic(&folder.join(queue::DRAFTS_FILE), &drafts).unwrap();
        queue::write_json_atomic(&folder.join(queue::DRAFT_LINKS_FILE), &links).unwrap();
    }

    fn agent_draft(s: &Session, id: i64) -> Option<group::AgentDraft> {
        s.rows().iter().flat_map(|r| &r.members).find(|m| m.id == id).expect("listed").agent_draft.clone()
    }

    #[test]
    fn agentDrafts_markedAtTheHeadCommit_showOnTheirRowsAfterAPoll() {
        let Fixture { mut s, dir, .. } = two_workspaces("agent-drafts");
        poll(&mut s, &[1, 2, 3]);
        agent_files(&dir, &[(1, "head"), (2, "head"), (3, "older")], &[(1, "head", "https://d/1"), (3, "older", "https://d/3")]);
        assert_eq!(agent_draft(&s, 1), None, "read at the next poll");

        poll(&mut s, &[1, 2, 3]);

        let drafts: Vec<_> = [1, 2, 3].iter().map(|id| agent_draft(&s, *id)).collect();
        assert_eq!(drafts, [Some(group::AgentDraft { host: Some("d".into()) }), Some(group::AgentDraft { host: None }), None]);
        assert_eq!(s.draft_url(1), Ok("https://d/1".into()));
        assert!(s.draft_url(2).is_err() && s.draft_url(3).is_err());
    }

    #[test]
    fn agentDrafts_sourceFailing_keepItsPrsMarkers() {
        let Fixture { mut s, dir, .. } = two_workspaces("agent-drafts-failed");
        agent_files(&dir, &[(5, "sha1")], &[(5, "sha1", "https://d/5")]);
        file(&mut s, ado(vec![ado_pr(5, "feature/x", Some(0))]), github(&[7]));
        assert_eq!(agent_draft(&s, 5), Some(group::AgentDraft { host: Some("d".into()) }));

        file(&mut s, Poll::Failed("Azure DevOps: HTTP 503".into()), github(&[7]));

        assert_eq!(s.draft_url(5), Ok("https://d/5".into()));
    }

    #[test]
    fn agentDrafts_damagedFile_showsNoMarkerAndNoError() {
        let Fixture { mut s, dir, .. } = two_workspaces("agent-drafts-damaged");
        agent_files(&dir, &[], &[]);
        std::fs::write(queue::workspace_dir(&dir.0, "a").join(queue::DRAFTS_FILE), "{").unwrap();

        poll(&mut s, &[1]);

        assert_eq!((agent_draft(&s, 1), s.ws.error.as_deref()), (None, None));
    }

    #[test]
    fn agentDrafts_damagedLinksFile_showsTheMarksWithoutLinks() {
        let Fixture { mut s, dir, .. } = two_workspaces("agent-links-damaged");
        agent_files(&dir, &[(1, "head")], &[]);
        std::fs::write(queue::workspace_dir(&dir.0, "a").join(queue::DRAFT_LINKS_FILE), "[").unwrap();

        poll(&mut s, &[1]);

        assert_eq!(agent_draft(&s, 1), Some(group::AgentDraft { host: None }));
    }

    // Saving and removing secrets.

    #[test]
    fn saveSecret_onTheActiveWorkspace_makesThePollInFlightStale_andStartsANewBaselineForThatSource() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("save-secret");
        poll(&mut s, &[1]);
        s.ws.baseline.insert(Provider::Ado, ids(&[9]));
        let started = s.generation;

        s.save_secret("a", Provider::GitHub, "  new-token  ").expect("saved");

        assert_eq!(s.store.read_github_token("a"), Ok(Some("new-token".into())), "trimmed");
        assert!(s.take_wake());
        assert!(s.file_poll(started, Poll::Off, github(&[1, 2]), Utc::now()).is_none(), "the poll in flight is stale");
        assert!(!s.ws.baseline.contains_key(&Provider::GitHub));
        assert_eq!(s.ws.baseline.get(&Provider::Ado), Some(&ids(&[9])), "the other source keeps its baseline");
        assert_eq!(poll(&mut s, &[1, 2]), ids(&[]), "the next good poll is a baseline, not a burst");
    }

    #[test]
    fn saveSecret_onAParkedWorkspace_leavesTheActivePollAlone() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("save-secret-parked");
        let generation = s.generation;
        s.save_secret("b", Provider::Ado, "pat").expect("saved");
        assert_eq!((s.generation, s.take_wake()), (generation, false));
        assert!(s.status().saved["b"].pat);
    }

    #[test]
    fn saveSecret_emptyOrUnknownWorkspace_isRefusedAndSavesNothing() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("save-secret-bad");
        assert_eq!(s.save_secret("a", Provider::Ado, "   "), Err("The PAT is empty".into()));
        assert_eq!(s.save_secret("zz", Provider::GitHub, "t"), Err("That workspace is gone".into()));
        assert_eq!((s.store.read_pat("a"), s.store.read_github_token("zz"), s.take_wake()), (Ok(None), Ok(None), false));
    }

    #[test]
    fn beginPoll_unreadableToken_stillPolls_andKeepsTheList() {
        let Fixture { mut s, secrets, dir: _dir } = two_workspaces("begin-poll-unreadable");
        poll(&mut s, &[1]);
        secrets.fail_reads(true);
        let ticket = s.begin_poll().expect("a ticket: the source fails with the read error instead of turning off");
        assert!(ticket.github_token.is_err());
        assert_eq!(listed(&s), ids(&[1]));
        assert!(s.status().saved["a"].github_token, "an unreadable token counts as saved");
    }

    #[test]
    fn beginPoll_noTokens_removesTheQueueFile() {
        let Fixture { mut s, dir, .. } = two_workspaces("begin-poll-queue");
        poll(&mut s, &[1]);
        let queue_file = queue::workspace_dir(&dir.0, "a").join(queue::QUEUE_FILE);
        assert!(queue_file.exists(), "the poll wrote a queue");
        s.remove_secret("a", Provider::GitHub).expect("removed");
        assert!(s.begin_poll().is_none());
        assert!(!queue_file.exists());
    }

    // General settings and adding a workspace.

    #[test]
    fn saveSettings_keepsTheWorkspaces_reportsWindowChanges_andWakesOnlyForTheInterval() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("save-settings");
        let sent = Settings { workspaces: Vec::new(), active_workspace: "zz".into(), pill_scale: s.settings.pill_scale + 20, ..s.settings.clone() };
        let change = s.save_settings(sent).expect("saved");
        assert_eq!(change, SettingsChange { pill_scale: Some(s.settings.pill_scale), list_resized: false });
        assert_eq!((s.settings.workspaces.len(), s.settings.active_workspace.as_str()), (2, "a"), "the workspaces change only through their own methods");
        assert!(!s.take_wake(), "a size change needs no new data");

        let sent = Settings { poll_minutes: s.settings.poll_minutes + 1, ..s.settings.clone() };
        assert_eq!(s.save_settings(sent).expect("saved"), SettingsChange::default());
        assert!(s.take_wake());
    }

    #[test]
    fn addWorkspace_namesAnUnnamedOne_andSavesIt() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("add-workspace");
        let id = s.add_workspace("  ").expect("added");
        let added = s.settings.workspaces.iter().find(|w| w.id == id).expect("listed");
        assert_eq!(added.name, "Workspace 3");
        assert_eq!(s.store.load().settings.workspaces.len(), 3, "saved");
        assert_eq!(s.settings.active_workspace, "a");
    }

    #[test]
    fn ticketUrl_rowWithoutATicket_saysSo() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("ticket-url");
        poll(&mut s, &[1]);
        let key = s.rows()[0].key.clone();
        assert_eq!(s.ticket_url(&key), Err("That row's branch names no ticket".into()));
        assert_eq!(s.ticket_url("nope"), Err("That row is gone; refresh the list".into()));
    }

    #[test]
    fn status_keepsTheSnapshotKeysTheWindowsRead() {
        let Fixture { s, dir: _dir, .. } = two_workspaces("status-keys");
        let value = serde_json::to_value(s.status()).expect("serializes");
        let mut keys: Vec<&str> = value.as_object().expect("an object").keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["counts", "error", "hasGithubToken", "hasPat", "items", "lastRefreshed", "polling", "saved", "settings"]);
    }

    #[test]
    fn removeSecret_turnsTheSourceOff_andTheNextPollClearsTheListWhenItWasTheLast() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("remove-secret");
        poll(&mut s, &[1]);
        s.remove_secret("a", Provider::GitHub).expect("removed");
        assert!(!s.status().saved["a"].github_token);
        assert!(s.take_wake());

        assert!(s.begin_poll().is_none());
        assert_eq!((listed(&s), s.polling), (ids(&[]), false));
        assert_eq!(s.ws.error.as_deref(), Some("Add an Azure DevOps PAT or a GitHub token to this workspace in settings"));
    }

    #[test]
    fn beginPoll_withAToken_handsOutTheCurrentGenerationAndShowsPolling() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("begin-poll");
        let ticket = s.begin_poll().expect("a ticket");
        assert_eq!((ticket.workspace.id.as_str(), ticket.generation), ("a", s.generation));
        assert_eq!((ticket.pat, ticket.github_token), (Ok(None), Ok(Some("gh-a".into()))));
        assert!(s.polling);
    }

    #[test]
    fn beginPoll_settlesApprovedAndHiddenAdoPrsWithKnownThreads_andTheirRowsKeepThem() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("settled");
        let known = score::PrThreads { comments: score::CommentStats { total: 3, open_threads: 1 }, last_activity: None };
        // 1 approved, 2 waiting on me, 3 hidden, 4 approved with threads that failed to load.
        let prs = vec![ado_pr(1, "b1", Some(10)), ado_pr(2, "b2", Some(0)), ado_pr(3, "b3", Some(0)), ado_pr(4, "b4", Some(10))];
        let Poll::Fresh(mut fetched) = ado(prs) else { unreachable!() };
        fetched.threads = HashMap::from([(1, known), (2, known), (3, known)]);
        s.ws.hidden.insert(3);
        file(&mut s, Poll::Fresh(fetched), Poll::Off);

        let ticket = s.begin_poll().expect("a ticket");
        assert_eq!(ticket.settled, HashMap::from([(1, "sha1".to_string()), (3, "sha1".to_string())]));

        let prs = vec![ado_pr(1, "b1", Some(10)), ado_pr(2, "b2", Some(0)), ado_pr(3, "b3", Some(0)), ado_pr(4, "b4", Some(10))];
        file(&mut s, ado(prs), Poll::Off);
        let comments: HashMap<i64, Option<score::CommentStats>> = s.ws.items.iter().map(|i| (i.id, i.comments)).collect();
        assert_eq!(comments, HashMap::from([(1, Some(known.comments)), (2, Some(known.comments)), (3, Some(known.comments)), (4, None)]));
    }

    // Saving a workspace.

    #[test]
    fn saveWorkspace_changedSource_resetsOnlyThatSourcesBaseline_andPollsAgain() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("save-workspace");
        s.ws.baseline.insert(Provider::Ado, ids(&[1]));
        s.ws.baseline.insert(Provider::GitHub, ids(&[2]));
        let generation = s.generation;

        let mut changed = s.settings.workspaces[0].clone();
        changed.github.repos = vec!["o/repo".into()];
        s.save_workspace(changed.clone()).expect("saved");

        assert_eq!(s.ws.baseline.get(&Provider::Ado), Some(&ids(&[1])));
        assert!(!s.ws.baseline.contains_key(&Provider::GitHub));
        assert_ne!(s.generation, generation);
        assert!(s.take_wake());

        let generation = s.generation;
        changed.name = "Renamed".into();
        s.save_workspace(changed).expect("saved");
        assert_eq!((s.generation, s.take_wake()), (generation, false), "a rename needs no poll");
        assert_eq!(s.ws.baseline.get(&Provider::Ado), Some(&ids(&[1])));
    }

    #[test]
    fn saveWorkspace_badRepo_changesNothing() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("save-workspace-bad");
        let mut changed = s.settings.workspaces[0].clone();
        changed.github.repos = vec!["not a repo".into()];
        s.save_workspace(changed).expect_err("refused");
        assert!(s.settings.workspaces[0].github.repos.is_empty());
    }

    #[test]
    fn saveWorkspace_failedWrite_changesNothing_andNeedsNoPoll() {
        let Fixture { mut s, dir, .. } = two_workspaces("save-workspace-unwritable");
        // A folder where settings.json goes: the rename into place fails.
        std::fs::create_dir_all(crate::queue::settings_path(&dir.0)).expect("folder");
        let generation = s.generation;
        let mut changed = s.settings.workspaces[0].clone();
        changed.github.repos = vec!["owner/name".into()];
        let error = s.save_workspace(changed).expect_err("not written");
        assert!(error.starts_with("Not saved"), "{error}");
        assert!(s.settings.workspaces[0].github.repos.is_empty());
        assert_eq!((s.generation, s.take_wake()), (generation, false));
    }

    // Hiding.

    #[test]
    fn setHidden_unhidingIsNotAnnouncedOnTheNextPoll_andTheHiddenListIsSaved() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("hide");
        poll(&mut s, &[1]);
        let key = s.rows()[0].key.clone();

        s.set_hidden(&key, true).expect("hidden");
        assert_eq!(s.store.hidden("a"), store::Hidden::from([(1, Some(Provider::GitHub))]));
        assert_eq!(poll(&mut s, &[1]), ids(&[]), "hidden PRs are not news");
        assert!(!s.ws.baseline[&Provider::GitHub].contains(&1), "a hidden PR leaves the baseline");

        s.set_hidden(&key, false).expect("shown");
        assert!(s.store.hidden("a").is_empty());
        assert_eq!(poll(&mut s, &[1]), ids(&[]), "unhiding is not news either");
    }

    #[test]
    fn setHidden_rowThatIsGone_isRefused() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("hide-gone");
        assert_eq!(s.set_hidden("nope", true), Err("That row is gone; refresh the list".into()));
    }

    // Switching and deleting workspaces.

    #[test]
    fn delete_theActiveWorkspace_switchesToItsNeighbour_andForgetsItsSecrets() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("delete-active");
        s.store.save_pat("a", "pat-a").expect("saved");
        s.delete("a").expect("deleted");
        assert_eq!(s.settings.active_workspace, "b");
        assert_eq!(s.settings.workspaces.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!((s.store.read_pat("a"), s.store.read_github_token("a")), (Ok(None), Ok(None)));
        assert_eq!(s.store.read_github_token("b"), Ok(Some("gh-b".into())));
        assert!(!s.status().saved.contains_key("a"));
        assert!(s.take_wake());
    }

    #[test]
    fn delete_theLastWorkspace_isRefused() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("delete-last");
        s.delete("b").expect("deleted");
        assert_eq!(s.delete("a"), Err("Keep at least one workspace".into()));
        assert_eq!(s.store.read_github_token("a"), Ok(Some("gh-a".into())));
    }

    #[test]
    fn switch_unknownWorkspace_isRefused_andChangesNothing() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("switch-unknown");
        let generation = s.generation;
        assert_eq!(s.switch("zz"), Err("That workspace is gone".into()));
        assert!(s.switch("a").is_ok(), "already active");
        let wake = s.take_wake();
        assert_eq!((s.settings.active_workspace.as_str(), s.generation, wake), ("a", generation, false));
    }

    #[test]
    fn switch_keepsEachWorkspacesHiddenPrsApart() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("switch-apart");
        s.store.save_hidden("b", &store::Hidden::from([(9, None)])).expect("saved");
        s.ws.hidden.insert(1);
        s.switch("b").expect("switched");
        assert_eq!(s.ws.hidden, ids(&[9]), "read from b's file on first visit");
        s.switch("a").expect("switched");
        assert_eq!(s.ws.hidden, ids(&[1]));
    }

    #[test]
    fn switchTo_workspaceWithNoSecrets_isNotShownAsPolling() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("switch-no-secrets");
        s.saved.insert("b".into(), Saved::default());
        s.switch("b").expect("switched");
        assert!(!s.polling);
    }

    // Filing a poll.

    #[test]
    fn applyPoll_oneSourceFailed_keepsItsPrsListed_andPrunesNoneOfItsHiddenOnes() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("one-failed");
        s.ws.items = vec![ReviewItem { provider: Provider::Ado, ..pr(1, ReviewState::NeedsReview) }];
        s.ws.urls.insert(1, "https://x/1".into());
        s.ws.hidden = ids(&[1, 5]);
        let generation = s.generation;
        s.apply_poll(generation, Poll::Failed("Azure DevOps: HTTP 503".into()), github(&[7]), Utc::now());
        assert_eq!(listed(&s), ids(&[1, 7]));
        assert_eq!(s.ws.hidden, ids(&[1, 5]), "nothing pruned while Azure DevOps is down");
        assert_eq!(s.store.hidden("a"), store::Hidden::from([(1, Some(Provider::Ado)), (5, None)]), "1's source was learned and saved");
        assert_eq!(s.ws.error.as_deref(), Some("Azure DevOps: HTTP 503"));
    }

    #[test]
    fn applyPoll_startedBeforeASwitch_isDiscardedAndLeavesTheNewWorkspaceAlone() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("stale-switch");
        poll(&mut s, &[1]);
        let started = s.generation;

        s.switch("b").expect("switched");
        let late = s.apply_poll(started, Poll::Off, github(&[1, 2]), Utc::now());

        assert!(late.fresh.is_empty());
        assert_eq!((s.settings.active_workspace.as_str(), listed(&s), s.ws.last_refreshed, s.polling), ("b", ids(&[]), None, true));
        assert_eq!(s.parked["a"].items.len(), 1);
    }

    #[test]
    fn applyPoll_baselineIsPerWorkspace_soSwitchingAnnouncesOnlyWhatIsNewSinceThatWorkspacesLastPoll() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("baseline-per-workspace");
        assert_eq!(poll(&mut s, &[1]), ids(&[]), "a's first poll is its baseline");
        assert_eq!(poll(&mut s, &[1, 2]), ids(&[2]));

        s.switch("b").expect("switched");
        assert_eq!(poll(&mut s, &[10, 11]), ids(&[]), "b's first poll is its baseline, not a burst");

        s.switch("a").expect("switched");
        assert_eq!(listed(&s), ids(&[1, 2]), "a's list is back before it polls");
        assert_eq!(poll(&mut s, &[1, 2]), ids(&[]), "nothing new in a: no burst on the way back");
        assert_eq!(poll(&mut s, &[1, 2, 3]), ids(&[3]));

        s.switch("b").expect("switched");
        assert_eq!(poll(&mut s, &[10, 11, 12]), ids(&[12]), "new in b while it was in the background");
    }

    #[test]
    fn applyPoll_notifyFollowsTheSetting() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("notify");
        poll(&mut s, &[1]);
        let generation = s.generation;
        let outcome = s.apply_poll(generation, Poll::Off, github(&[1, 2]), Utc::now());
        assert_eq!((outcome.fresh.len(), outcome.notify), (1, true));

        s.settings.notify = false;
        let outcome = s.apply_poll(generation, Poll::Off, github(&[1, 2, 3]), Utc::now());
        assert_eq!((outcome.fresh.len(), outcome.notify), (1, false), "still fresh for the counter, but no notification");
    }

    #[test]
    fn applyPoll_gitHubOnly_prunesClosedPrs_andDoesntRewriteTheFileEachPoll() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("prune-github");
        s.ws.hidden = ids(&[7, 8]);
        let first = file(&mut s, Poll::Off, github(&[7])).hidden.expect("8 closed; 7's source learned");
        assert_eq!(first, store::Hidden::from([(7, Some(Provider::GitHub))]));
        assert!(file(&mut s, Poll::Off, github(&[7])).hidden.is_none(), "nothing changed");
    }

    #[test]
    fn applyPoll_azureDevOpsNeverAnswered_listsGitHubWithNoAge_andOffForgetsItsTime() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("never-answered");
        file(&mut s, Poll::Failed("Azure DevOps: HTTP 401".into()), github(&[7]));
        assert_eq!((listed(&s), s.ws.last_refreshed), (ids(&[7]), None));

        s.ws.source_refreshed.insert(Provider::Ado, Utc::now());
        file(&mut s, Poll::Off, github(&[7]));
        assert!(!s.ws.source_refreshed.contains_key(&Provider::Ado));
        assert!(s.ws.last_refreshed.is_some());
    }

    #[test]
    fn applyPoll_failingAzureDevOps_keepsTheListsAgeAtItsLastGoodPoll() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("failing-age");
        let ado_answered: DateTime<Utc> = "2026-11-18T08:00:00Z".parse().unwrap();
        s.ws.source_refreshed.insert(Provider::Ado, ado_answered);
        file(&mut s, Poll::Failed("Azure DevOps: HTTP 503".into()), github(&[7]));
        assert_eq!(s.ws.last_refreshed, Some(ado_answered), "GitHub's fresh answer must not make the list look current");
    }

    #[test]
    fn applyPoll_gitHubOnly_writesAQueueWithItsPrs() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("queue-github");
        let queue = file(&mut s, Poll::Off, github(&[7])).queue.expect("a queue");
        let pr = &queue.branches[0].prs[0];
        assert_eq!((pr.provider.as_str(), pr.pr_id, pr.repo_id.as_str()), (queue::GITHUB, 7, "o/repo"));
        assert!(queue.missing_sources.is_empty());
    }

    #[test]
    fn applyPoll_failedSource_isNamedInTheQueue() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("queue-failed");
        let queue = file(&mut s, Poll::Failed("Azure DevOps: HTTP 503".into()), github(&[7])).queue.expect("a queue");
        assert_eq!(queue.missing_sources, [queue::AZURE_DEV_OPS]);
    }

    #[test]
    fn applyPoll_everySourceFailed_writesNoQueue() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("queue-none");
        let applied = file(&mut s, Poll::Failed("Azure DevOps: x".into()), Poll::Failed("GitHub: y".into()));
        assert!(applied.queue.is_none(), "a stale queue must age out, not be re-stamped");
    }

    #[test]
    fn applyPoll_partialGitHubPoll_queuesWhatCameBackAndNamesGitHub() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("queue-partial");
        let queue = file(&mut s, Poll::Off, Poll::Fresh(github::tests::partial(&[7]))).queue.expect("a queue");
        assert_eq!((queue.branches[0].prs[0].pr_id, queue.missing_sources.as_slice()), (7, [queue::GITHUB.to_string()].as_slice()));
    }

    #[test]
    fn applyPoll_failedAzureDevOps_keepsItsRowsButLeavesThemOutOfTheQueue() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("queue-failed-rows");
        s.ws.items = vec![ReviewItem { provider: Provider::Ado, ..pr(1, ReviewState::NeedsReview) }];
        let applied = file(&mut s, Poll::Failed("Azure DevOps: HTTP 503".into()), github(&[7]));
        assert!(listed(&s).contains(&1));
        let queued: Vec<i64> = applied.queue.expect("a queue").branches.iter().flat_map(|b| &b.prs).map(|p| p.pr_id).collect();
        assert_eq!(queued, [7]);
    }

    #[test]
    fn load_readsWhichSecretsEachWorkspaceHas() {
        let Fixture { s, secrets, dir } = two_workspaces("load");
        s.store.save_settings(&s.settings).expect("saved");
        s.store.save_pat("b", "pat-b").expect("saved");
        let (loaded, warning) = Session::load(Store::new(&dir.0, Box::new(secrets)));
        assert_eq!(warning, None);
        assert_eq!(loaded.status().saved["b"], Saved { pat: true, github_token: true });
        assert_eq!(loaded.settings.active_workspace, "a");
    }

    // Filing an Azure DevOps poll.

    #[test]
    fn applyPoll_freshAzureDevOps_listsMine_relatesOthersOnTheBranch_andQueuesThem() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("ado-fresh");
        let prs = vec![ado_pr(101, "feature/x", Some(0)), ado_pr(102, "feature/x", None), ado_pr(103, "feature/y", None)];
        let applied = file(&mut s, ado(prs), Poll::Off);
        assert_eq!(listed(&s), ids(&[101]));
        assert_eq!(related_ids(&s), [102], "same branch, not mine: related; another branch: left out");
        let row = &s.rows()[0];
        assert_eq!(row.members.iter().map(|m| (m.id, m.state.is_some())).collect::<Vec<_>>(), [(101, true), (102, false)]);
        let queued = &applied.queue.expect("a queue").branches[0].prs[0];
        assert_eq!(
            (queued.provider.as_str(), queued.pr_id, queued.project.as_str(), queued.repo_id.as_str(), queued.source_commit.as_deref()),
            (queue::AZURE_DEV_OPS, 101, "Contoso", "repo-id", Some("sha1"))
        );
        assert_eq!((queued.url.as_str(), queued.last_voted_commit.as_deref()), ("https://ado/101", None));
    }

    #[test]
    fn applyPoll_freshAzureDevOps_pushSinceMyVote_isAReReviewAgainstTheVotedCommit_andSavesTheMemory() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("ado-rereview");
        s.ws.memory.insert(101, score::VotedAt { vote: 10, commit: "sha0".into() });
        let generation = s.generation;
        s.apply_poll(generation, ado(vec![ado_pr(101, "feature/x", Some(10))]), Poll::Off, Utc::now());
        assert_eq!(s.ws.items[0].state, score::ReviewState::ReReview);
        assert_eq!(s.store.memory("a").get(&101).map(|v| v.commit.as_str()), Some("sha0"), "the vote memory is saved");

        let applied = file(&mut s, ado(vec![ado_pr(101, "feature/x", Some(10))]), Poll::Off);
        let queued = &applied.queue.expect("a queue").branches[0].prs[0];
        assert_eq!(queued.last_voted_commit.as_deref(), Some("sha0"));
    }

    #[test]
    fn applyPoll_relatedPrs_surviveAFailedAzureDevOpsPoll_andGoWhenItIsOff() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("ado-related");
        file(&mut s, ado(vec![ado_pr(101, "feature/x", Some(0)), ado_pr(102, "feature/x", None)]), Poll::Off);
        file(&mut s, Poll::Failed("Azure DevOps: HTTP 503".into()), github(&[7]));
        assert_eq!(related_ids(&s), [102], "a fresh GitHub answer must not wipe them");
        assert_eq!(listed(&s), ids(&[101, 7]));
        file(&mut s, Poll::Off, github(&[7]));
        assert_eq!(related_ids(&s), Vec::<i64>::new());
    }

    #[test]
    fn applyPoll_bothSources_errorsAndMissingSourcesAreInAzureDevOpsThenGitHubOrder() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("both-order");
        let applied = file(&mut s, Poll::Failed("Azure DevOps: HTTP 503".into()), Poll::Fresh(github::tests::partial(&[7])));
        assert_eq!(s.ws.error.as_deref(), Some("Azure DevOps: HTTP 503 · GitHub: over 100 PRs in a search; only the first 100 are listed"));
        assert_eq!(applied.queue.expect("a queue").missing_sources, [queue::AZURE_DEV_OPS, queue::GITHUB]);
    }

    #[test]
    fn applyPoll_listsAndQueuesByScore_notInTheOrderTheSourceAnswered() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("sorted");
        let mut required = ado_pr(103, "feature/c", Some(0));
        required.reviewers[0].is_required = true;
        let answered = vec![ado_pr(101, "feature/a", Some(10)), ado_pr(102, "feature/b", Some(0)), required];
        let applied = file(&mut s, ado(answered), Poll::Off);
        assert_eq!(s.ws.items.iter().map(|i| i.id).collect::<Vec<_>>(), [103, 102, 101], "required, then to review, then approved");
        let queued: Vec<i64> = applied.queue.expect("a queue").branches.iter().flat_map(|b| &b.prs).map(|p| p.pr_id).collect();
        assert_eq!(queued, [103, 102]);
    }

    #[test]
    fn applyPoll_staleAzureDevOpsPoll_leavesTheVoteMemoryAlone() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("stale-memory");
        s.ws.memory.insert(101, score::VotedAt { vote: 10, commit: "sha0".into() });
        let stale = s.generation + 1;
        let outcome = s.apply_poll(stale, ado(vec![ado_pr(101, "feature/x", Some(0))]), Poll::Off, Utc::now());
        assert!(outcome.fresh.is_empty());
        assert!(s.ws.memory.contains_key(&101), "a vote reset to 0 in a stale answer must not forget the vote");
        assert!(s.store.memory("a").is_empty(), "nor save anything");
    }

    #[test]
    fn applyPoll_bothSourcesFresh_announcesAzureDevOpsNewPrsThenGitHubs() {
        let Fixture { mut s, dir: _dir, .. } = two_workspaces("both-fresh");
        s.store.save_pat("a", "pat").expect("saved");
        file(&mut s, ado(vec![ado_pr(101, "feature/x", Some(0))]), github(&[7]));
        let generation = s.generation;
        let outcome = s.apply_poll(generation, ado(vec![ado_pr(101, "feature/x", Some(0)), ado_pr(102, "feature/z", Some(0))]), github(&[7, 8]), Utc::now());
        assert_eq!(outcome.fresh.iter().map(|i| i.id).collect::<Vec<_>>(), [102, 8]);
        assert_eq!(listed(&s), ids(&[101, 102, 7, 8]));
    }

    // The pure rules.

    #[test]
    fn tokenSaved_readErrorCountsAsSaved() {
        assert!(token_saved(&Err("Credential Manager: locked".into())));
        assert!(token_saved(&Ok(Some("t".into()))));
        assert!(!token_saved(&Ok(None)));
    }

    #[test]
    fn pruneHidden_learningASource_isSaved() {
        let mut ws = hidden_state();
        ws.urls.insert(1, "https://x/1".into());
        assert!(prune_hidden(&mut ws, &complete(&[]), &complete(&[])), "1's source was learned, so the file changes");
        assert_eq!(hidden_file(&ws), store::Hidden::from([(1, Some(Provider::Ado)), (2, Some(Provider::GitHub)), (3, None)]));
        assert!(!prune_hidden(&mut ws, &complete(&[]), &complete(&[])));
    }

    #[test]
    fn pruneHidden_failedAzureDevOps_keepsItsHiddenPrs_andPrunesGitHubsClosedOnes() {
        let mut ws = hidden_state();
        assert!(prune_hidden(&mut ws, &complete(&[Provider::GitHub]), &complete(&[])));
        assert_eq!(ws.hidden, ids(&[1, 3]), "1 is Azure DevOps's, 3 has no known source while Azure DevOps failed");
        assert_eq!(ws.hidden_source.get(&1), Some(&Provider::Ado));
    }

    #[test]
    fn pruneHidden_partialGitHub_stillPrunesAzureDevOpsClosedOnes() {
        let mut ws = hidden_state();
        ws.items.clear();
        ws.hidden_source.insert(1, Provider::Ado);
        assert!(prune_hidden(&mut ws, &complete(&[Provider::Ado]), &complete(&[])));
        assert_eq!(ws.hidden, ids(&[2, 3]));
    }

    #[test]
    fn pruneHidden_everySourceComplete_prunesWhatClosed_andKeepsWhatIsOpen() {
        let mut ws = hidden_state();
        ws.urls.insert(1, "https://x/1".into());
        assert!(prune_hidden(&mut ws, &complete(&[Provider::Ado, Provider::GitHub]), &complete(&[])));
        assert_eq!(ws.hidden, ids(&[1]));
        assert_eq!(ws.hidden_source.keys().copied().collect::<HashSet<_>>(), ids(&[1]));
    }

    #[test]
    fn pruneHidden_sourceTurnedOff_keepsItsHiddenPrs_butAnUnknownOneGoes() {
        let mut ws = hidden_state();
        ws.items.clear();
        assert!(prune_hidden(&mut ws, &complete(&[Provider::Ado]), &complete(&[Provider::GitHub])));
        assert_eq!(ws.hidden, ids(&[2]), "2 is GitHub's, which is off; 1 closed on Azure DevOps; 3 has no source");
    }

    #[test]
    fn oldestRefresh_isTheOldestSourceWithAToken_orNoneWhileOneNeverAnswered() {
        let (early, late): (DateTime<Utc>, DateTime<Utc>) = ("2026-11-18T08:00:00Z".parse().unwrap(), "2026-11-18T09:00:00Z".parse().unwrap());
        let both = HashMap::from([(Provider::Ado, early), (Provider::GitHub, late)]);
        assert_eq!(oldest_refresh(&both, &complete(&[])), Some(early));
        assert_eq!(oldest_refresh(&both, &complete(&[Provider::Ado])), Some(late), "a source with no token doesn't count");
        let github_only = HashMap::from([(Provider::GitHub, late)]);
        assert_eq!(oldest_refresh(&github_only, &complete(&[])), None, "Azure DevOps has a token but never answered");
        assert_eq!(oldest_refresh(&HashMap::new(), &complete(&[Provider::Ado, Provider::GitHub])), None);
    }

    #[test]
    fn nextBaseline_firstPoll_recordsWithoutAnnouncing() {
        let (a, b) = (pr(1, ReviewState::NeedsReview), pr(2, ReviewState::Approved));
        assert_eq!(next_baseline(None, &[&a, &b], false), (ids(&[1]), ids(&[])));
    }

    #[test]
    fn nextBaseline_fullPoll_announcesNewAndForgetsGone() {
        let a = pr(3, ReviewState::NeedsReview);
        assert_eq!(next_baseline(Some(&ids(&[1])), &[&a], false), (ids(&[3]), ids(&[3])));
    }

    #[test]
    fn nextBaseline_partialPoll_keepsBaselinePrsItDidNotList() {
        let a = pr(3, ReviewState::NeedsReview);
        assert_eq!(next_baseline(Some(&ids(&[1])), &[&a], true), (ids(&[1, 3]), ids(&[3])));
    }

    #[test]
    fn nextBaseline_partialPoll_dropsAListedPrIApproved_soItsNextPushAnnounces() {
        let approved = pr(1, ReviewState::Approved);
        let (baseline, _) = next_baseline(Some(&ids(&[1])), &[&approved], true);
        assert_eq!(baseline, ids(&[]));
        let pushed = pr(1, ReviewState::ReReview);
        assert_eq!(next_baseline(Some(&baseline), &[&pushed], true).1, ids(&[1]));
    }
}
