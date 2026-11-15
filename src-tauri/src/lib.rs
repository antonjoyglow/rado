mod ado;
mod export;
mod github;
mod group;
// Shared with the MCP server binary, which uses the parts the app doesn't.
#[allow(dead_code)]
mod queue;
mod score;
mod store;
mod workspace;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use tokio::sync::{Notify, watch};

use score::{ReviewItem, VoteMemory};
use store::{Keyring, Settings, Store};
use workspace::{Tracker, Workspace};

const PILL: &str = "pill";
const LIST: &str = "list";
/// The pill's size at 100%, matching `tauri.conf.json`.
const PILL_SIZE: (f64, f64) = (112.0, 36.0);

struct Shared {
    store: Store,
    wake: Notify,
    /// Mirrors `Inner::credentials_generation`; a change cancels the poll in flight.
    generation: watch::Sender<u64>,
    inner: Mutex<Inner>,
}

/// What one workspace has seen. The active workspace's lives in `Inner::ws`; the others wait in `Inner::parked`
/// until switched back to, so their lists, vote memory and notification baselines survive a switch.
#[derive(Default)]
struct WorkspaceState {
    items: Vec<ReviewItem>,
    urls: HashMap<i64, String>,
    memory: VoteMemory,
    hidden: HashSet<i64>,
    related: Vec<group::Related>,
    last_refreshed: Option<DateTime<Utc>>,
    error: Option<String>,
    /// Per source, the PRs already waiting on me at its last good poll. A source with no entry yet records
    /// its PRs without notifying, so a new token, a recovered source or a first visit doesn't announce everything at once.
    known_actionable: HashMap<score::Provider, HashSet<i64>>,
}

/// Which secrets a workspace has saved; the values never leave the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    pat: bool,
    github_token: bool,
}

#[derive(Default)]
struct Inner {
    settings: Settings,
    ws: WorkspaceState,
    parked: HashMap<String, WorkspaceState>,
    saved: HashMap<String, Saved>,
    /// What went wrong loading the settings; kept apart from `ws.error`, which every poll replaces.
    load_warning: Option<String>,
    polling: bool,
    credentials_generation: u64,
    list_hidden_at: Option<Instant>,
    list_had_focus: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    items: Vec<group::ReviewRow>,
    last_refreshed: Option<DateTime<Utc>>,
    polling: bool,
    error: Option<String>,
    has_pat: bool,
    has_github_token: bool,
    /// What went wrong loading the settings, for the whole run.
    warning: Option<String>,
    settings: Settings,
    /// By workspace id.
    saved: HashMap<String, Saved>,
}

impl Inner {
    fn rows(&self) -> Vec<group::ReviewRow> {
        let tracker = self.settings.active().map(|w| &w.tracker).unwrap_or(&Tracker::None);
        group::group(&self.ws.items, &self.ws.related, &self.ws.urls, tracker)
    }

    fn row(&self, key: &str) -> Result<group::ReviewRow, String> {
        self.rows().into_iter().find(|r| r.key == key).ok_or_else(|| "That row is gone; refresh the list".to_string())
    }

    fn active_saved(&self) -> Saved {
        self.saved.get(&self.settings.active_workspace).copied().unwrap_or_default()
    }

    fn snapshot(&self) -> Snapshot {
        let saved = self.active_saved();
        Snapshot {
            items: self.rows(),
            last_refreshed: self.ws.last_refreshed,
            polling: self.polling,
            error: self.ws.error.clone(),
            warning: self.load_warning.clone(),
            has_pat: saved.pat,
            has_github_token: saved.github_token,
            settings: self.settings.clone(),
            saved: self.saved.clone(),
        }
    }

    /// A workspace's state, active or parked.
    fn state_of(&mut self, id: &str) -> Option<&mut WorkspaceState> {
        if id == self.settings.active_workspace { Some(&mut self.ws) } else { self.parked.get_mut(id) }
    }

    /// Makes `id` the active workspace, parking the current one. `load` reads a workspace never visited since start.
    /// Bumps the generation, so a poll still running for the old workspace is cancelled and its result discarded.
    fn switch_to(&mut self, id: &str, load: impl FnOnce() -> WorkspaceState) -> bool {
        if id == self.settings.active_workspace || !self.settings.workspaces.iter().any(|w| w.id == id) {
            return false;
        }
        let target = self.parked.remove(id).unwrap_or_else(load);
        let previous = std::mem::replace(&mut self.ws, target);
        let previous_id = std::mem::replace(&mut self.settings.active_workspace, id.to_string());
        self.parked.insert(previous_id, previous);
        self.credentials_generation += 1;
        let saved = self.active_saved();
        self.polling = saved.pat || saved.github_token;
        true
    }
}

fn load_state(store: &Store, id: &str) -> WorkspaceState {
    WorkspaceState { memory: store.memory(id), hidden: store.hidden(id), ..WorkspaceState::default() }
}

fn update(app: &AppHandle, change: impl FnOnce(&mut Inner)) {
    let shared = app.state::<Shared>();
    let snapshot = {
        let mut inner = shared.inner.lock().expect("state lock");
        change(&mut inner);
        // Sent under the lock, so the watch can't go back to an older generation and cancel a newer poll.
        let generation = inner.credentials_generation;
        shared.generation.send_if_modified(|g| std::mem::replace(g, generation) != generation);
        inner.snapshot()
    };
    let _ = app.emit("snapshot", snapshot);
}

/// One source's result for a poll.
enum Poll<T> {
    /// No token for this source.
    Off,
    /// Its PRs from the last good poll stay listed.
    Failed(String),
    Fresh(T),
}

struct AdoPoll {
    prs: Vec<ado::PullRequest>,
    my_id: String,
    teams: HashSet<String>,
    threads: HashMap<i64, ado::PrThreads>,
    urls: HashMap<i64, String>,
}

async fn poll_loop(app: AppHandle) {
    let mut me: Option<(u64, String)> = None;
    loop {
        let shared = app.state::<Shared>();
        // Subscribed before reading the generation: a switch after this point cancels the poll below.
        let mut changed = shared.generation.subscribe();
        let (workspace, generation) = {
            let inner = shared.inner.lock().expect("state lock");
            (inner.settings.active().cloned(), inner.credentials_generation)
        };
        if let Some(workspace) = workspace {
            poll_workspace(&app, &workspace, generation, &mut me, &mut changed).await;
        }
        let minutes = shared.inner.lock().expect("state lock").settings.poll_minutes.clamp(1, 120);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(u64::from(minutes) * 60)) => {}
            _ = shared.wake.notified() => {}
        }
    }
}

/// Polls one workspace, by the id read with `generation`: a switch while it runs drops the result rather than filing it
/// under the workspace switched to.
async fn poll_workspace(app: &AppHandle, workspace: &Workspace, generation: u64, me: &mut Option<(u64, String)>, changed: &mut watch::Receiver<u64>) {
    let shared = app.state::<Shared>();
    let (pat, github_token) = (shared.store.pat(&workspace.id), shared.store.github_token(&workspace.id));
    let saved = Saved { pat: pat.is_some(), github_token: github_token.is_some() };
    let current = |s: &Inner| s.credentials_generation == generation;
    if pat.is_none() && github_token.is_none() {
        update(app, |s| {
            if current(s) {
                s.saved.insert(workspace.id.clone(), saved);
                s.polling = false;
                s.ws.error = Some("Add an Azure DevOps PAT or a GitHub token to this workspace in settings".into());
            }
        });
        return;
    }
    update(app, |s| {
        if current(s) {
            s.saved.insert(workspace.id.clone(), saved);
            s.polling = true;
        }
    });
    let ado = async {
        match pat.as_deref() {
            None => Poll::Off,
            Some(pat) => match poll_ado(app, &workspace.azure_dev_ops, pat, generation, me).await {
                Ok(poll) => Poll::Fresh(poll),
                Err(error) => Poll::Failed(format!("Azure DevOps: {error}")),
            },
        }
    };
    let github = async {
        match github_token.as_deref() {
            None => Poll::Off,
            Some(token) => match github::fetch(token, &workspace.github.repos).await {
                Ok(fetched) => Poll::Fresh(fetched),
                Err(error) => Poll::Failed(format!("GitHub: {error}")),
            },
        }
    };
    tokio::select! {
        (ado, github) = async { tokio::join!(ado, github) } => apply(app, &workspace.id, generation, ado, github),
        // Switched, or a token or organization changed: the answer is stale, and the wake that came with it polls again.
        _ = changed.changed() => {}
    }
}

/// Comment counts and last activity for the PRs in my list only; a PR whose threads fail to load just shows neither.
async fn fetch_threads(
    app: &AppHandle,
    client: &ado::Client,
    prs: &[ado::PullRequest],
    me: &score::Me<'_>,
) -> HashMap<i64, ado::PrThreads> {
    let mut scratch = app.state::<Shared>().inner.lock().expect("state lock").ws.memory.clone();
    let mine: HashSet<i64> = score::rank(prs, me, &mut scratch, Utc::now()).iter().map(|i| i.id).collect();
    let requests = prs.iter().filter(|pr| mine.contains(&pr.pull_request_id)).map(|pr| async move {
        let stats = client.threads(&pr.repository.project.id, &pr.repository.id, pr.pull_request_id).await;
        (pr.pull_request_id, stats)
    });
    futures_util::future::join_all(requests)
        .await
        .into_iter()
        .filter_map(|(id, stats)| stats.ok().map(|s| (id, s)))
        .collect()
}

async fn poll_ado(
    app: &AppHandle,
    source: &workspace::AdoSource,
    pat: &str,
    generation: u64,
    me: &mut Option<(u64, String)>,
) -> Result<AdoPoll, String> {
    if source.organization.is_empty() {
        return Err("set the organization for this workspace in settings".into());
    }
    let client = ado::Client::new(&source.organization, pat);
    if me.as_ref().is_none_or(|(g, _)| *g != generation) {
        *me = Some((generation, client.my_id().await?.to_lowercase()));
    }
    let my_id = me.as_ref().map(|(_, id)| id.clone()).unwrap_or_default();
    let prs = client.active_pull_requests().await?;
    let teams: HashSet<String> = source.teams.iter().map(|t| t.trim().to_lowercase()).collect();
    let threads = fetch_threads(app, &client, &prs, &score::Me { id: &my_id, teams: &teams }).await;
    let urls = prs.iter().map(|pr| (pr.pull_request_id, client.pr_url(pr))).collect();
    Ok(AdoPoll { prs, my_id, teams, threads, urls })
}

/// What a poll's result asks for: files to save, and who to notify.
#[derive(Default)]
struct Applied {
    /// Newly waiting on me since the workspace's last poll.
    fresh: Vec<ReviewItem>,
    notify: bool,
    memory: Option<VoteMemory>,
    hidden: Option<HashSet<i64>>,
    queue: Option<queue::Queue>,
}

fn apply(app: &AppHandle, workspace_id: &str, generation: u64, ado: Poll<AdoPoll>, github: Poll<github::Fetched>) {
    let shared = app.state::<Shared>();
    let mut applied = None;
    // Saved under the lock, so a hide or a delete made after this poll can't be overwritten by its older copy.
    update(app, |s| {
        applied = apply_poll(s, generation, ado, github, Utc::now());
        let Some(applied) = &applied else { return };
        if let Some(memory) = &applied.memory {
            let _ = shared.store.save_memory(workspace_id, memory);
        }
        if let Some(hidden) = &applied.hidden {
            let _ = shared.store.save_hidden(workspace_id, hidden);
        }
        if let Some(queue) = &applied.queue {
            let _ = shared.store.save_queue(workspace_id, queue);
        }
    });
    let Some(applied) = applied else { return };
    if applied.notify && !applied.fresh.is_empty() {
        let title = match applied.fresh.len() {
            1 => "A PR needs you".to_string(),
            n => format!("{n} PRs need you"),
        };
        let body = applied.fresh.iter().take(3).map(|i| format!("{} · {}", i.repo, i.title)).collect::<Vec<_>>().join("\n");
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

/// Files a poll's result under the active workspace. None when the poll is stale: a switch, a new token or a changed
/// organization since it started.
fn apply_poll(s: &mut Inner, generation: u64, ado: Poll<AdoPoll>, github: Poll<github::Fetched>, now: DateTime<Utc>) -> Option<Applied> {
    if s.credentials_generation != generation {
        return None;
    }
    let mut applied = Applied::default();
    let organization = s.settings.active().map(|w| w.azure_dev_ops.organization.clone()).unwrap_or_default();
    let ws = &mut s.ws;
    let was_stale: HashSet<i64> = ws.items.iter().filter(|i| i.state == score::ReviewState::Stale).map(|i| i.id).collect();
    let (old_items, old_urls) = (std::mem::take(&mut ws.items), std::mem::take(&mut ws.urls));
    let mut items = Vec::new();
    let mut urls = HashMap::new();
    let mut errors = Vec::new();
    let mut fresh_sources = Vec::new();
    // Pruning hidden PRs needs every source's full list; a failed or partial poll would drop its hidden ones.
    let mut complete = true;
    let mut ado_prs = None;
    let keep = |provider: score::Provider, items: &mut Vec<ReviewItem>, urls: &mut HashMap<i64, String>| {
        for item in old_items.iter().filter(|i| i.provider == provider) {
            if let Some(url) = old_urls.get(&item.id) {
                urls.insert(item.id, url.clone());
            }
            items.push(item.clone());
        }
    };
    match ado {
        Poll::Off => {
            ws.related.clear();
            ws.known_actionable.remove(&score::Provider::Ado);
        }
        Poll::Failed(error) => {
            errors.push(error);
            complete = false;
            keep(score::Provider::Ado, &mut items, &mut urls);
        }
        Poll::Fresh(poll) => {
            fresh_sources.push((score::Provider::Ado, false));
            let me = score::Me { id: &poll.my_id, teams: &poll.teams };
            let mut ranked = score::rank(&poll.prs, &me, &mut ws.memory, now);
            score::attach_threads(&mut ranked, &poll.threads, &was_stale, now);
            let mine: HashSet<i64> = ranked.iter().map(|i| i.id).collect();
            let branches: HashSet<&str> = ranked.iter().map(|i| i.branch.as_str()).collect();
            ws.related = poll
                .prs
                .iter()
                .filter(|pr| !mine.contains(&pr.pull_request_id) && branches.contains(pr.branch()))
                .map(|pr| group::Related {
                    id: pr.pull_request_id,
                    repo: pr.repository.name.clone(),
                    title: pr.title.clone(),
                    branch: pr.branch().to_string(),
                    url: poll.urls.get(&pr.pull_request_id).cloned().unwrap_or_default(),
                    created_at: pr.creation_date,
                })
                .collect();
            applied.memory = Some(ws.memory.clone());
            urls.extend(poll.urls);
            items.extend(ranked);
            ado_prs = Some(poll.prs);
        }
    }
    match github {
        Poll::Off => {
            ws.known_actionable.remove(&score::Provider::GitHub);
        }
        Poll::Failed(error) => {
            errors.push(error);
            complete = false;
            keep(score::Provider::GitHub, &mut items, &mut urls);
        }
        Poll::Fresh(fetched) => {
            fresh_sources.push((score::Provider::GitHub, fetched.warning.is_some()));
            if let Some(warning) = &fetched.warning {
                errors.push(format!("GitHub: {warning}"));
                complete = false;
            }
            let (mut ranked, gh_urls, threads) = github::items(&fetched, now);
            score::attach_threads(&mut ranked, &threads, &was_stale, now);
            urls.extend(gh_urls);
            items.extend(ranked);
        }
    }
    items.sort_by(|a, b| b.score.cmp(&a.score).then(b.age_hours.cmp(&a.age_hours)));
    ws.items = items;
    ws.urls = urls;
    s.polling = false;
    ws.error = (!errors.is_empty()).then(|| errors.join(" · "));
    if fresh_sources.is_empty() {
        return Some(applied);
    }
    if complete {
        let open = ws.urls.keys().copied().collect::<HashSet<_>>();
        let before = ws.hidden.len();
        ws.hidden.retain(|id| open.contains(id));
        if ws.hidden.len() != before {
            applied.hidden = Some(ws.hidden.clone());
        }
    }
    mark_hidden(ws);
    for (source, partial) in fresh_sources {
        let listed: Vec<&ReviewItem> = ws.items.iter().filter(|i| i.provider == source).collect();
        let (known, new) = next_known(ws.known_actionable.get(&source), &listed, partial);
        applied.fresh.extend(listed.into_iter().filter(|i| new.contains(&i.id)).cloned());
        ws.known_actionable.insert(source, known);
    }
    ws.last_refreshed = Some(now);
    applied.notify = s.settings.notify;
    if let Some(prs) = &ado_prs {
        let ctx = export::Context { organization: &organization, poll_minutes: s.settings.poll_minutes, now };
        applied.queue = Some(export::queue(&ws.items, prs, &ws.memory, &ws.urls, &ctx));
    }
    Some(applied)
}

/// One source's PRs waiting on me after a poll, and which of them are new since `known` (none when the
/// source has no `known` yet). A partial answer keeps known PRs it didn't list, so they aren't announced
/// again when they reappear; a PR it did list is judged on what it says now.
fn next_known(known: Option<&HashSet<i64>>, listed: &[&ReviewItem], partial: bool) -> (HashSet<i64>, HashSet<i64>) {
    let mut waiting: HashSet<i64> = listed.iter().filter(|i| i.actionable && !i.hidden).map(|i| i.id).collect();
    let Some(known) = known else { return (waiting, HashSet::new()) };
    let new = waiting.difference(known).copied().collect();
    if partial {
        let present: HashSet<i64> = listed.iter().map(|i| i.id).collect();
        waiting.extend(known.iter().filter(|id| !present.contains(id)));
    }
    (waiting, new)
}

fn mark_hidden(ws: &mut WorkspaceState) {
    for item in &mut ws.items {
        item.hidden = ws.hidden.contains(&item.id);
    }
}

fn list_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(LIST) {
        return Ok(window);
    }
    let window = WebviewWindowBuilder::new(app, LIST, WebviewUrl::App("index.html".into()))
        .title("Pending reviews")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        let WindowEvent::Focused(focused) = event else { return };
        // Windows may refuse focus to a newly shown window and report a blur at once;
        // only a blur after the list really had focus means the user clicked away.
        let clicked_away = {
            let shared = handle.state::<Shared>();
            let mut inner = shared.inner.lock().expect("state lock");
            if *focused {
                inner.list_had_focus = true;
                false
            } else if inner.list_had_focus {
                inner.list_hidden_at = Some(Instant::now());
                true
            } else {
                false
            }
        };
        // Hiding can raise another focus event synchronously, so the lock must be released first.
        if clicked_away && let Some(w) = handle.get_webview_window(LIST) {
            let _ = w.hide();
        }
    });
    Ok(window)
}

/// Sizes the list from settings, shrunk to fit the counter's screen, and puts it beside the counter.
fn place_list(app: &AppHandle, list: &tauri::WebviewWindow) {
    let (width, height) = {
        let shared = app.state::<Shared>();
        let inner = shared.inner.lock().expect("state lock");
        (inner.settings.list_width, inner.settings.list_height)
    };
    let Some(pill) = app.get_webview_window(PILL) else { return };
    let (Ok(pos), Ok(size), Ok(Some(monitor))) = (pill.outer_position(), pill.outer_size(), pill.current_monitor()) else { return };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let (left, top) = (area.position.x, area.position.y);
    let (right, bottom) = (left + area.size.width as i32, top + area.size.height as i32);
    let w = ((f64::from(width) * scale) as i32).min(area.size.width as i32);
    let h = ((f64::from(height) * scale) as i32).min(area.size.height as i32);
    let x = if pos.x + size.width as i32 / 2 > (left + right) / 2 { pos.x - w - 8 } else { pos.x + size.width as i32 + 8 };
    let at = PhysicalPosition::new(x.clamp(left, (right - w).max(left)), pos.y.clamp(top, (bottom - h).max(top)));
    // Moving onto a monitor with another DPI rescales the window, so it moves first, is sized there, and is placed again.
    let _ = list.set_position(at);
    let _ = list.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = list.set_position(at);
}

fn show_list(app: &AppHandle) -> tauri::Result<()> {
    let list = list_window(app)?;
    place_list(app, &list);
    app.state::<Shared>().inner.lock().expect("state lock").list_had_focus = false;
    list.show()?;
    let _ = list.set_focus();
    Ok(())
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, Shared>) -> Snapshot {
    state.inner.lock().expect("state lock").snapshot()
}

#[tauri::command]
fn refresh_now(state: tauri::State<'_, Shared>) {
    state.wake.notify_one();
}

// Async: building a window inside a synchronous command can deadlock on Windows.
#[tauri::command]
async fn toggle_list(app: AppHandle) -> Result<(), String> {
    let visible = app.get_webview_window(LIST).is_some_and(|w| w.is_visible().unwrap_or(false));
    // Clicking the pill blurs the list first, which hides it; that click means "close", not "reopen".
    let just_hidden = app
        .state::<Shared>()
        .inner
        .lock()
        .expect("state lock")
        .list_hidden_at
        .is_some_and(|t| t.elapsed() < Duration::from_millis(400));
    if visible {
        hide_list(app);
        Ok(())
    } else if just_hidden {
        Ok(())
    } else {
        show_list(&app).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn hide_list(app: AppHandle) {
    if let Some(w) = app.get_webview_window(LIST) {
        let _ = w.hide();
    }
}

/// Opens links in the browser chosen in settings. When that browser can't be started, the links open in the
/// default one, with the reason in the status line until the next refresh; Windows is asked only once per call.
fn open_links(app: &AppHandle, urls: Vec<String>) -> Result<(), String> {
    let mut program = app.state::<Shared>().inner.lock().expect("state lock").settings.browser.program();
    for url in urls {
        if let Some(name) = program {
            match app.opener().open_url(&url, Some(name)) {
                Ok(()) => continue,
                Err(e) => {
                    update(app, |s| s.ws.error = Some(format!("Couldn't start {name} ({e}); opened in the default browser")));
                    program = None;
                }
            }
        }
        app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn open_pr(app: AppHandle, id: i64) -> Result<(), String> {
    let url = app.state::<Shared>().inner.lock().expect("state lock").ws.urls.get(&id).cloned();
    open_links(&app, vec![url.ok_or("Unknown pull request")?])
}

#[tauri::command]
fn open_row(app: AppHandle, key: String) -> Result<(), String> {
    let row = app.state::<Shared>().inner.lock().expect("state lock").row(&key)?;
    open_links(&app, group::open_urls(&row))
}

#[tauri::command]
fn open_ticket(app: AppHandle, key: String) -> Result<(), String> {
    let shared = app.state::<Shared>();
    let (row, tracker) = {
        let inner = shared.inner.lock().expect("state lock");
        (inner.row(&key)?, inner.settings.active().map(|w| w.tracker.clone()).unwrap_or_default())
    };
    let ticket = row.ticket.ok_or("That row's branch names no ticket")?;
    open_links(&app, vec![tracker.ticket_url(&ticket).ok_or("Set this workspace's tracker in settings")?])
}

#[tauri::command]
fn copy_row(app: AppHandle, key: String) -> Result<(), String> {
    let row = app.state::<Shared>().inner.lock().expect("state lock").row(&key)?;
    app.clipboard().write_text(group::share_text(&row.members)).map_err(|e| e.to_string())
}

/// Hides or shows every PR of mine in the row.
#[tauri::command]
fn set_hidden(app: AppHandle, key: String, hidden: bool) -> Result<(), String> {
    let shared = app.state::<Shared>();
    let mut result = Ok(());
    update(&app, |s| {
        let ids: Vec<(i64, score::Provider)> = match s.row(&key) {
            Ok(row) => row.members.iter().filter(|m| m.state.is_some()).map(|m| (m.id, m.provider)).collect(),
            Err(e) => {
                result = Err(e);
                return;
            }
        };
        for (id, provider) in ids {
            if hidden {
                s.ws.hidden.insert(id);
            } else {
                s.ws.hidden.remove(&id);
                // Unhiding is not news: keep it out of the next poll's notification.
                if let Some(known) = s.ws.known_actionable.get_mut(&provider) {
                    known.insert(id);
                }
            }
        }
        mark_hidden(&mut s.ws);
        result = shared.store.save_hidden(&s.settings.active_workspace, &s.ws.hidden);
    });
    result
}

/// After a new secret for `id`: a poll for it running now is stale, and its next good poll is a baseline.
fn secret_saved(app: &AppHandle, id: &str, provider: score::Provider) {
    let shared = app.state::<Shared>();
    let mut active = false;
    update(app, |s| {
        let saved = s.saved.entry(id.to_string()).or_default();
        match provider {
            score::Provider::Ado => saved.pat = true,
            score::Provider::GitHub => saved.github_token = true,
        }
        active = id == s.settings.active_workspace;
        if active {
            s.ws.error = None;
            s.credentials_generation += 1;
        }
        if let Some(state) = s.state_of(id) {
            state.known_actionable.remove(&provider);
        }
    });
    if active {
        shared.wake.notify_one();
    }
}

fn known_workspace(app: &AppHandle, id: &str) -> Result<(), String> {
    let inner = app.state::<Shared>();
    let inner = inner.inner.lock().expect("state lock");
    inner.settings.workspaces.iter().any(|w| w.id == id).then_some(()).ok_or_else(|| "That workspace is gone".to_string())
}

#[tauri::command]
fn save_pat(app: AppHandle, workspace: String, pat: String) -> Result<(), String> {
    let pat = pat.trim();
    if pat.is_empty() {
        return Err("The PAT is empty".into());
    }
    known_workspace(&app, &workspace)?;
    app.state::<Shared>().store.save_pat(&workspace, pat)?;
    secret_saved(&app, &workspace, score::Provider::Ado);
    Ok(())
}

#[tauri::command]
fn save_github_token(app: AppHandle, workspace: String, token: String) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("The GitHub token is empty".into());
    }
    known_workspace(&app, &workspace)?;
    app.state::<Shared>().store.save_github_token(&workspace, token)?;
    secret_saved(&app, &workspace, score::Provider::GitHub);
    Ok(())
}

/// The general settings; the workspaces change only through their own commands.
#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let shared = app.state::<Shared>();
    let mut result = Ok(());
    let (mut resize, mut resize_list, mut refetch, mut pill_scale) = (false, false, false, 0);
    update(&app, |s| {
        let settings = Settings {
            workspaces: s.settings.workspaces.clone(),
            active_workspace: s.settings.active_workspace.clone(),
            migrating_to: s.settings.migrating_to.clone(),
            ..settings
        }
        .clamped();
        result = shared.store.save_settings(&settings);
        if result.is_err() {
            return;
        }
        let old = &s.settings;
        resize = old.pill_scale != settings.pill_scale;
        resize_list = (old.list_width, old.list_height) != (settings.list_width, settings.list_height);
        refetch = old.poll_minutes != settings.poll_minutes;
        pill_scale = settings.pill_scale;
        s.settings = settings;
    });
    result?;
    if resize {
        size_pill(&app, pill_scale, true);
    }
    if resize_list && let Some(list) = app.get_webview_window(LIST) {
        place_list(&app, &list);
    }
    // A size or notification change needs no new data; refetching on every − / + click would hit ADO each time.
    if refetch {
        shared.wake.notify_one();
    }
    Ok(())
}

/// Applies `change` and saves the settings, all under the lock so two edits can't save out of order.
/// A failed save leaves the change in place for this run and says so.
fn edit_workspaces<T>(app: &AppHandle, change: impl FnOnce(&mut Inner) -> Result<T, String>) -> Result<T, String> {
    let shared = app.state::<Shared>();
    let mut result = Err(String::new());
    update(app, |s| {
        result = change(s).and_then(|value| {
            shared.store.save_settings(&s.settings).map(|()| value).map_err(|e| format!("Changed until Rado quits, but not saved: {e}"))
        });
    });
    result
}

#[tauri::command]
fn add_workspace(app: AppHandle, name: String) -> Result<String, String> {
    edit_workspaces(&app, |s| {
        let id = workspace::new_id();
        let name = match name.trim() {
            "" => format!("Workspace {}", s.settings.workspaces.len() + 1),
            name => name.to_string(),
        };
        s.settings.workspaces.push(Workspace { id: id.clone(), name, ..Workspace::default() });
        Ok(id)
    })
}

/// Saves a workspace's name, Azure DevOps organization and teams, watched GitHub repos, and tracker.
#[tauri::command]
fn save_workspace(app: AppHandle, workspace: Workspace) -> Result<(), String> {
    let mut refetch = false;
    let typed_tracker = workspace.tracker != Tracker::None;
    let result = edit_workspaces(&app, |s| {
        if let Some(repo) = workspace.bad_repo() {
            return Err(format!("Not saved: “{repo}” isn't a GitHub repo (owner/name)"));
        }
        let mut workspace = workspace.clamped();
        if typed_tracker && workspace.tracker == Tracker::None {
            return Err("Not saved: that isn't a Jira site or a Linear workspace".into());
        }
        let active = workspace.id == s.settings.active_workspace;
        let slot = s.settings.workspaces.iter_mut().find(|w| w.id == workspace.id).ok_or("That workspace is gone")?;
        if workspace.name.is_empty() {
            workspace.name.clone_from(&slot.name);
        }
        // A changed source is re-polled, and its next good poll is a baseline, so new matches don't each notify.
        let changed: Vec<score::Provider> = [
            (score::Provider::Ado, slot.azure_dev_ops != workspace.azure_dev_ops),
            (score::Provider::GitHub, slot.github != workspace.github),
        ]
        .into_iter()
        .filter_map(|(provider, changed)| changed.then_some(provider))
        .collect();
        *slot = workspace;
        let id = slot.id.clone();
        if let Some(state) = s.state_of(&id) {
            for provider in &changed {
                state.known_actionable.remove(provider);
            }
        }
        if active && !changed.is_empty() {
            s.credentials_generation += 1;
            refetch = true;
        }
        Ok(())
    });
    // The bump cancelled the poll in flight; even when the save failed, the change stands for this run and needs a poll.
    if refetch {
        app.state::<Shared>().wake.notify_one();
    }
    result
}

#[tauri::command]
fn switch_workspace(app: AppHandle, id: String) -> Result<(), String> {
    let shared = app.state::<Shared>();
    let mut switched = false;
    let result = edit_workspaces(&app, |s| {
        switched = s.switch_to(&id, || load_state(&shared.store, &id));
        if id == s.settings.active_workspace { Ok(()) } else { Err("That workspace is gone".to_string()) }
    });
    if switched {
        shared.wake.notify_one();
    }
    result
}

/// Deletes a workspace with its secrets and data. The last one stays; deleting the active one switches to its neighbour.
#[tauri::command]
fn delete_workspace(app: AppHandle, id: String) -> Result<(), String> {
    let shared = app.state::<Shared>();
    let (mut switched, mut removed) = (false, false);
    let result = edit_workspaces(&app, |s| {
        let list = &s.settings.workspaces;
        let index = list.iter().position(|w| w.id == id).ok_or("That workspace is gone")?;
        if list.len() == 1 {
            return Err("Keep at least one workspace".into());
        }
        if id == s.settings.active_workspace {
            let neighbour = list[if index + 1 < list.len() { index + 1 } else { index - 1 }].id.clone();
            switched = s.switch_to(&neighbour, || load_state(&shared.store, &neighbour));
        }
        s.settings.workspaces.remove(index);
        s.parked.remove(&id);
        s.saved.remove(&id);
        removed = true;
        Ok(())
    });
    if switched {
        shared.wake.notify_one();
    }
    // Its secrets go even when the save failed: a PAT left with nothing pointing at it would never be removed.
    if removed {
        shared.store.forget(&id).map_err(|e| format!("Deleted the workspace, but its saved data stayed behind: {e}"))?;
    }
    result
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    if enabled { launcher.enable() } else { launcher.disable() }.map_err(|e| e.to_string())
}

#[tauri::command]
fn quit(app: AppHandle) {
    let _ = app.save_window_state(StateFlags::POSITION);
    app.exit(0);
}

/// Glance's hover fade: the cursor inside the window bounds widened by 15% fades the count.
/// Checks every 50 ms near the window and every 250 ms when the cursor is far away.
async fn hover_loop(app: AppHandle) {
    const ZONE: f64 = 0.15;
    const NEARBY_MARGIN: f64 = 150.0;
    let mut hovering = false;
    loop {
        let mut nearby = false;
        if let (Some(pill), Ok(cursor)) = (app.get_webview_window(PILL), app.cursor_position())
            && let (Ok(pos), Ok(size)) = (pill.outer_position(), pill.outer_size())
        {
            let (w, h) = (f64::from(size.width), f64::from(size.height));
            let inside = |margin_x: f64, margin_y: f64| {
                cursor.x >= f64::from(pos.x) - margin_x
                    && cursor.x <= f64::from(pos.x) + w + margin_x
                    && cursor.y >= f64::from(pos.y) - margin_y
                    && cursor.y <= f64::from(pos.y) + h + margin_y
            };
            let now_hovering = inside(w * ZONE, h * ZONE);
            nearby = inside(w * ZONE + NEARBY_MARGIN, h * ZONE + NEARBY_MARGIN);
            if now_hovering != hovering {
                hovering = now_hovering;
                let _ = pill.emit("hover", hovering);
            }
        }
        tokio::time::sleep(Duration::from_millis(if nearby { 50 } else { 250 })).await;
    }
}

/// Sizes the pill window for `percent` of its 112 × 36 original. When `keep_anchor` is set, the right
/// edge (where the text sits) and the vertical centre stay put, so the counter grows leftwards in place.
fn size_pill(app: &AppHandle, percent: u32, keep_anchor: bool) {
    let Some(pill) = app.get_webview_window(PILL) else { return };
    let before = (pill.outer_position(), pill.outer_size());
    let factor = f64::from(percent) / 100.0;
    if pill.set_size(LogicalSize::new(PILL_SIZE.0 * factor, PILL_SIZE.1 * factor)).is_err() {
        return;
    }
    if keep_anchor
        && let (Ok(pos), Ok(old)) = before
        && let Ok(new) = pill.outer_size()
    {
        let x = pos.x + old.width as i32 - new.width as i32;
        let y = pos.y + (old.height as i32 - new.height as i32) / 2;
        let _ = pill.set_position(PhysicalPosition::new(x, y));
    }
    keep_on_screen(&pill);
}

/// Moves the window back inside its screen's work area if any part of it sticks out.
fn keep_on_screen(window: &tauri::WebviewWindow) {
    if let (Ok(Some(monitor)), Ok(pos), Ok(size)) = (window.current_monitor(), window.outer_position(), window.outer_size()) {
        let area = monitor.work_area();
        let (left, top) = (area.position.x, area.position.y);
        let right = (left + area.size.width as i32 - size.width as i32).max(left);
        let bottom = (top + area.size.height as i32 - size.height as i32).max(top);
        let clamped = PhysicalPosition::new(pos.x.clamp(left, right), pos.y.clamp(top, bottom));
        if clamped != pos {
            let _ = window.set_position(clamped);
        }
    }
}

fn place_pill(app: &AppHandle) {
    let saved = app.path().app_config_dir().map(|d| d.join(".window-state.json").exists()).unwrap_or(false);
    let Some(pill) = app.get_webview_window(PILL) else { return };
    if saved {
        // A saved position may come from a wider pill; keep the whole window on screen.
        keep_on_screen(&pill);
        return;
    }
    if let (Ok(Some(monitor)), Ok(size)) = (pill.primary_monitor(), pill.outer_size()) {
        let area = monitor.work_area();
        // Glance's default edge margin.
        let margin = (12.0 * monitor.scale_factor()) as i32;
        let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
        let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
        let _ = pill.set_position(PhysicalPosition::new(x, y));
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show reviews", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &refresh, &quit_item])?;
    TrayIconBuilder::with_id("tray")
        .icon(app.default_window_icon().cloned().expect("bundled icon"))
        .tooltip("Rado")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                let _ = show_list(app);
            }
            "refresh" => app.state::<Shared>().wake.notify_one(),
            "quit" => quit(app.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let _ = show_list(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let _ = show_list(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().with_state_flags(StateFlags::POSITION).with_denylist(&[LIST]).build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .setup(|app| {
            let config = app.path().app_config_dir()?;
            let data = app.path().app_data_dir()?;
            let store = Store::new(&config, &data, Box::new(Keyring::from_env()));
            let loaded = store.load();
            let saved = loaded
                .settings
                .workspaces
                .iter()
                .map(|w| (w.id.clone(), Saved { pat: store.pat(&w.id).is_some(), github_token: store.github_token(&w.id).is_some() }))
                .collect();
            let ws = load_state(&store, &loaded.settings.active_workspace);
            let inner = Inner { settings: loaded.settings, ws, saved, load_warning: loaded.warning, ..Default::default() };
            let pill_scale = inner.settings.pill_scale;
            app.manage(Shared { store, wake: Notify::new(), generation: watch::Sender::new(0), inner: Mutex::new(inner) });
            // The saved position is the top-left of the already-scaled pill, so size it without moving it.
            size_pill(app.handle(), pill_scale, false);
            place_pill(app.handle());
            // The plugin saves only on a clean exit; save on every move so a drag survives a kill or a reboot.
            if let Some(pill) = app.get_webview_window(PILL) {
                let handle = app.handle().clone();
                pill.on_window_event(move |event| {
                    if let WindowEvent::Moved(_) = event {
                        let _ = handle.save_window_state(StateFlags::POSITION);
                    }
                });
            }
            build_tray(app.handle())?;
            tauri::async_runtime::spawn(poll_loop(app.handle().clone()));
            tauri::async_runtime::spawn(hover_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_now,
            toggle_list,
            hide_list,
            open_pr,
            open_row,
            open_ticket,
            copy_row,
            set_hidden,
            save_pat,
            save_github_token,
            save_settings,
            add_workspace,
            save_workspace,
            switch_workspace,
            delete_workspace,
            get_autostart,
            set_autostart,
            quit
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use score::{PrFacts, Provider, ReviewState};

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

    /// Two workspaces, `a` (active) and `b`, both with a GitHub token.
    fn two_workspaces() -> Inner {
        let ws = |id: &str| Workspace { id: id.into(), name: id.to_uppercase(), ..Workspace::default() };
        let saved = Saved { pat: false, github_token: true };
        Inner {
            settings: Settings { workspaces: vec![ws("a"), ws("b")], active_workspace: "a".into(), ..Settings::default() },
            saved: HashMap::from([("a".into(), saved), ("b".into(), saved)]),
            ..Inner::default()
        }
    }

    /// Applies a GitHub poll listing `prs` at the current generation and returns the ids it would announce.
    fn poll(s: &mut Inner, prs: &[i64]) -> HashSet<i64> {
        let applied = apply_poll(s, s.credentials_generation, Poll::Off, Poll::Fresh(github::tests::requested(prs)), Utc::now()).expect("current poll");
        applied.fresh.iter().map(|i| i.id).collect()
    }

    fn listed(s: &Inner) -> HashSet<i64> {
        s.ws.items.iter().map(|i| i.id).collect()
    }

    #[test]
    fn applyPoll_startedBeforeASwitch_isDiscardedAndLeavesTheNewWorkspaceAlone() {
        let mut s = two_workspaces();
        poll(&mut s, &[1]);
        let started = s.credentials_generation;

        assert!(s.switch_to("b", WorkspaceState::default));
        let late = apply_poll(&mut s, started, Poll::Off, Poll::Fresh(github::tests::requested(&[1, 2])), Utc::now());

        assert!(late.is_none());
        assert_eq!((s.settings.active_workspace.as_str(), listed(&s), s.ws.last_refreshed, s.polling), ("b", ids(&[]), None, true));
        assert_eq!(s.parked["a"].items.len(), 1);
    }

    #[test]
    fn applyPoll_baselineIsPerWorkspace_soSwitchingAnnouncesOnlyWhatIsNewSinceThatWorkspacesLastPoll() {
        let mut s = two_workspaces();
        assert_eq!(poll(&mut s, &[1]), ids(&[]), "a's first poll is its baseline");
        assert_eq!(poll(&mut s, &[1, 2]), ids(&[2]));

        s.switch_to("b", WorkspaceState::default);
        assert_eq!(poll(&mut s, &[10, 11]), ids(&[]), "b's first poll is its baseline, not a burst");

        s.switch_to("a", || unreachable!("a is parked"));
        assert_eq!(listed(&s), ids(&[1, 2]), "a's list is back before it polls");
        assert_eq!(poll(&mut s, &[1, 2]), ids(&[]), "nothing new in a: no burst on the way back");
        assert_eq!(poll(&mut s, &[1, 2, 3]), ids(&[3]));

        s.switch_to("b", || unreachable!("b is parked"));
        assert_eq!(poll(&mut s, &[10, 11, 12]), ids(&[12]), "new in b while it was in the background");
    }

    #[test]
    fn switchTo_sameOrUnknownWorkspace_changesNothing() {
        let mut s = two_workspaces();
        let generation = s.credentials_generation;
        assert!(!s.switch_to("a", || unreachable!()));
        assert!(!s.switch_to("zz", || unreachable!()));
        assert_eq!((s.settings.active_workspace.as_str(), s.credentials_generation), ("a", generation));
    }

    #[test]
    fn switchTo_keepsEachWorkspacesHiddenPrsAndVoteMemoryApart() {
        let mut s = two_workspaces();
        s.ws.hidden.insert(1);
        s.switch_to("b", || WorkspaceState { hidden: ids(&[9]), ..WorkspaceState::default() });
        assert_eq!(s.ws.hidden, ids(&[9]));
        s.switch_to("a", || unreachable!());
        assert_eq!(s.ws.hidden, ids(&[1]));
    }

    #[test]
    fn switchTo_workspaceWithNoSecrets_isNotShownAsPolling() {
        let mut s = two_workspaces();
        s.saved.insert("b".into(), Saved::default());
        s.switch_to("b", WorkspaceState::default);
        assert!(!s.polling);
    }

    #[test]
    fn nextKnown_firstPoll_recordsWithoutAnnouncing() {
        let (a, b) = (pr(1, ReviewState::NeedsReview), pr(2, ReviewState::Approved));
        assert_eq!(next_known(None, &[&a, &b], false), (ids(&[1]), ids(&[])));
    }

    #[test]
    fn nextKnown_fullPoll_announcesNewAndForgetsGone() {
        let a = pr(3, ReviewState::NeedsReview);
        assert_eq!(next_known(Some(&ids(&[1])), &[&a], false), (ids(&[3]), ids(&[3])));
    }

    #[test]
    fn nextKnown_partialPoll_keepsKnownPrsItDidNotList() {
        let a = pr(3, ReviewState::NeedsReview);
        assert_eq!(next_known(Some(&ids(&[1])), &[&a], true), (ids(&[1, 3]), ids(&[3])));
    }

    #[test]
    fn nextKnown_partialPoll_dropsAListedPrIApproved_soItsNextPushAnnounces() {
        let approved = pr(1, ReviewState::Approved);
        let (known, _) = next_known(Some(&ids(&[1])), &[&approved], true);
        assert_eq!(known, ids(&[]));
        let pushed = pr(1, ReviewState::ReReview);
        assert_eq!(next_known(Some(&known), &[&pushed], true).1, ids(&[1]));
    }
}
