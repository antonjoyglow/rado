mod ado;
mod export;
mod extension;
mod github;
mod group;
// Shared with the MCP server binary, which uses the parts the app doesn't.
#[allow(dead_code)]
mod queue;
mod score;
mod releases;
mod session;
mod source;
mod store;
mod workspace;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use futures_util::StreamExt;
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

use score::{PrThreads, Provider};
use session::{Outcome, Poll, Session, Ticket};
use store::{Keyring, Settings, Store};
use workspace::Workspace;

const PILL: &str = "pill";
const LIST: &str = "list";
/// The pill's size at 100%, matching `tauri.conf.json`.
const PILL_SIZE: (f64, f64) = (112.0, 36.0);

struct Shared {
    wake: Notify,
    /// Mirrors the session's generation; a change cancels the poll in flight.
    generation: watch::Sender<u64>,
    inner: Mutex<Inner>,
}

struct Inner {
    session: Session,
    /// What went wrong loading the settings; kept apart from the session's error, which every poll replaces.
    load_warning: Option<String>,
    list_had_focus: bool,
    newer_release: Option<releases::NewerRelease>,
}

#[derive(Clone, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    #[serde(flatten)]
    status: session::Status,
    /// What went wrong loading the settings, for the whole run.
    warning: Option<String>,
    /// A GitHub release newer than this Rado, for the list's Download link.
    newer_release: Option<releases::NewerRelease>,
}

impl Inner {
    fn snapshot(&self) -> Snapshot {
        Snapshot { status: self.session.status(), warning: self.load_warning.clone(), newer_release: self.newer_release.clone() }
    }
}

/// Applies `change` under the lock, then keeps the generation watch in step, wakes the poller when the session asks,
/// and sends every window the new snapshot.
fn update<T>(app: &AppHandle, change: impl FnOnce(&mut Inner) -> T) -> T {
    let shared = app.state::<Shared>();
    let (value, snapshot, wake) = {
        let mut inner = shared.inner.lock().expect("state lock");
        let value = change(&mut inner);
        // Sent under the lock, so the watch can't go back to an older generation and cancel a newer poll.
        let generation = inner.session.generation();
        shared.generation.send_if_modified(|g| std::mem::replace(g, generation) != generation);
        (value, inner.snapshot(), inner.session.take_wake())
    };
    let _ = app.emit("snapshot", snapshot);
    if wake {
        shared.wake.notify_one();
    }
    value
}

/// Runs one session method through `update`.
fn session<T>(app: &AppHandle, change: impl FnOnce(&mut Session) -> T) -> T {
    update(app, |s| change(&mut s.session))
}

async fn poll_loop(app: AppHandle) {
    let mut me: Option<(u64, String)> = None;
    loop {
        let shared = app.state::<Shared>();
        // Subscribed before the ticket reads the generation: a switch after this point cancels the poll below.
        let mut changed = shared.generation.subscribe();
        if let Some(ticket) = session(&app, |s| s.begin_poll(Utc::now())) {
            poll_workspace(&app, ticket, &mut me, &mut changed).await;
        }
        let wait = {
            let inner = shared.inner.lock().expect("state lock");
            let interval = Duration::from_secs(u64::from(inner.session.settings().poll_interval()) * 60);
            // A failing source's next try can fall between polls (a rate limit's end); wake for it.
            let now = Utc::now();
            let next_try = inner.session.next_try(now).and_then(|at| (at - now).to_std().ok());
            next_try.map_or(interval, |d| d.min(interval))
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = shared.wake.notified() => {}
        }
    }
}

/// Polls one workspace for its ticket: a switch while it runs drops the result rather than filing it under the
/// workspace switched to.
async fn poll_workspace(app: &AppHandle, ticket: Ticket, me: &mut Option<(u64, String)>, changed: &mut watch::Receiver<u64>) {
    let Ticket { workspace, generation, pat, github_token, settled, held } = ticket;
    let ado = async {
        match pat.as_ref().map(Option::as_deref) {
            Ok(None) => Poll::Off,
            _ if held.contains(&Provider::Ado) => Poll::Held,
            Err(error) => Poll::Failed(format!("Azure DevOps: can't read the PAT ({error})").into()),
            Ok(Some(pat)) => match poll_ado(&workspace.azure_dev_ops, pat, generation, me, &settled).await {
                Ok(poll) => Poll::Fresh(poll),
                Err(error) => Poll::Failed(error.named("Azure DevOps")),
            },
        }
    };
    let github = async {
        match github_token.as_ref().map(Option::as_deref) {
            Ok(None) => Poll::Off,
            _ if held.contains(&Provider::GitHub) => Poll::Held,
            Err(error) => Poll::Failed(format!("GitHub: can't read the token ({error})").into()),
            Ok(Some(token)) => match github::fetch(token, &workspace.github.repos).await {
                Ok(fetched) => Poll::Fresh(fetched),
                Err(error) => Poll::Failed(error.named("GitHub")),
            },
        }
    };
    tokio::select! {
        (ado, github) = async { tokio::join!(ado, github) } => apply(app, generation, ado, github),
        // Switched, or a token or organization changed: the answer is stale, and the wake that came with it polls again.
        _ = changed.changed() => {}
    }
}

/// Thread requests in flight at once, so a long list doesn't burst at Azure DevOps.
const THREAD_REQUESTS: usize = 6;

/// Comment counts and last activity for the PRs in my list that need them; a PR whose threads fail to load keeps
/// what the last poll knew. A throttled threads request fails the whole poll, so the source backs off.
async fn fetch_threads(
    client: &ado::Client,
    prs: &[ado::PullRequest],
    me: &ado::Me<'_>,
    settled: &HashMap<i64, String>,
) -> Result<HashMap<i64, PrThreads>, source::SourceError> {
    // Collected first: a lazy map of async closures trips the compiler's `Send` check on the poll loop.
    let requests: Vec<_> = ado::needing_threads(prs, me, settled)
        .into_iter()
        .map(|pr| async move {
            let stats = client.threads(&pr.repository.project.id, &pr.repository.id, pr.pull_request_id).await;
            (pr.pull_request_id, stats)
        })
        .collect();
    let answers: Vec<_> = futures_util::stream::iter(requests).buffer_unordered(THREAD_REQUESTS).collect().await;
    let mut threads = HashMap::new();
    for (id, stats) in answers {
        match stats {
            Ok(stats) => {
                threads.insert(id, stats);
            }
            Err(error) if matches!(error.kind, source::FailureKind::RateLimited { .. }) => return Err(error),
            Err(_) => {}
        }
    }
    Ok(threads)
}

async fn poll_ado(
    source: &workspace::AdoSource,
    pat: &str,
    generation: u64,
    me: &mut Option<(u64, String)>,
    settled: &HashMap<i64, String>,
) -> Result<ado::Fetched, source::SourceError> {
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
    let threads = fetch_threads(&client, &prs, &ado::Me { id: &my_id, teams: &teams }, settled).await?;
    let urls = prs.iter().map(|pr| (pr.pull_request_id, client.pr_url(pr))).collect();
    Ok(ado::Fetched { prs, my_id, teams, threads, urls })
}

fn apply(app: &AppHandle, generation: u64, ado: Poll<ado::Fetched>, github: Poll<github::Fetched>) {
    let Outcome { fresh, notify } = session(app, |s| s.apply_poll(generation, ado, github, Utc::now()));
    if fresh.is_empty() {
        return;
    }
    let _ = app.emit("fresh", fresh.len());
    if notify {
        let title = match fresh.len() {
            1 => "A PR needs you".to_string(),
            n => format!("{n} PRs need you"),
        };
        let body = fresh.iter().take(3).map(|i| format!("{} · {}", i.repo, i.title)).collect::<Vec<_>>().join("\n");
        let _ = app.notification().builder().title(title).body(body).show();
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
        // Alt+F4 closes like ✕, keeping the window and bringing the counter back.
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            hide_list(handle.clone());
            return;
        }
        let WindowEvent::Focused(focused) = event else { return };
        // Windows may refuse focus to a newly shown window and report a blur at once;
        // only a blur after the list really had focus means the user clicked away.
        let clicked_away = {
            let shared = handle.state::<Shared>();
            let mut inner = shared.inner.lock().expect("state lock");
            if *focused {
                inner.list_had_focus = true;
                false
            } else {
                inner.list_had_focus
            }
        };
        // Hiding can raise another focus event synchronously, so the lock must be released first.
        if clicked_away {
            hide_list(handle.clone());
        }
    });
    Ok(window)
}

/// A screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// Narrowest list, in logical pixels; below it the rows stop fitting their buttons.
const LIST_MIN_WIDTH: f64 = 360.0;

/// The list on `area` (the work area of the counter's screen): `percent` of its width and height, against
/// the screen edge on the counter's side, its middle level with the counter's, and kept on screen.
fn list_rect(area: Rect, counter: Rect, percent: (u32, u32), min_width: i32) -> Rect {
    let w = (area.w * percent.0 as i32 / 100).max(min_width).min(area.w);
    let h = area.h * percent.1 as i32 / 100;
    let (mid_x, mid_y) = (counter.x + counter.w / 2, counter.y + counter.h / 2);
    let x = if mid_x > area.x + area.w / 2 { area.x + area.w - w } else { area.x };
    let y = (mid_y - h / 2).clamp(area.y, area.y + area.h - h);
    Rect { x, y, w, h }
}

fn place_list(app: &AppHandle, list: &tauri::WebviewWindow) {
    let percent = {
        let shared = app.state::<Shared>();
        let session = &shared.inner.lock().expect("state lock").session;
        (session.settings().list_width_percent, session.settings().list_height_percent)
    };
    let Some(pill) = app.get_webview_window(PILL) else { return };
    let (Ok(pos), Ok(size), Ok(Some(monitor))) = (pill.outer_position(), pill.outer_size(), pill.current_monitor()) else { return };
    let area = monitor.work_area();
    let area = Rect { x: area.position.x, y: area.position.y, w: area.size.width as i32, h: area.size.height as i32 };
    let counter = Rect { x: pos.x, y: pos.y, w: size.width as i32, h: size.height as i32 };
    let r = list_rect(area, counter, percent, (LIST_MIN_WIDTH * monitor.scale_factor()) as i32);
    let at = PhysicalPosition::new(r.x, r.y);
    // Moving onto a monitor with another DPI rescales the window, so it moves first, is sized there, and is placed again.
    let _ = list.set_position(at);
    let _ = list.set_size(PhysicalSize::new(r.w as u32, r.h as u32));
    let _ = list.set_position(at);
}

/// Opens the list and takes the counter off the screen until `hide_list`.
fn show_list(app: &AppHandle) -> tauri::Result<()> {
    let list = list_window(app)?;
    place_list(app, &list);
    app.state::<Shared>().inner.lock().expect("state lock").list_had_focus = false;
    list.show()?;
    let _ = list.set_focus();
    if let Some(pill) = app.get_webview_window(PILL) {
        let _ = pill.hide();
    }
    Ok(())
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, Shared>) -> Snapshot {
    state.inner.lock().expect("state lock").snapshot()
}

#[tauri::command]
fn refresh_now(app: AppHandle) {
    session(&app, |s| s.refresh(Utc::now()));
}

// Async: building a window inside a synchronous command can deadlock on Windows.
#[tauri::command]
async fn open_list(app: AppHandle) -> Result<(), String> {
    show_list(&app).map_err(|e| e.to_string())
}

/// Every way the list closes (✕, Escape, a click elsewhere) ends here, so the counter always comes back.
#[tauri::command]
fn hide_list(app: AppHandle) {
    if let Some(w) = app.get_webview_window(LIST) {
        let _ = w.hide();
    }
    if let Some(pill) = app.get_webview_window(PILL) {
        let _ = pill.show();
    }
}

/// Opens links in the browser chosen in settings. When that browser can't be started, the links open in the
/// default one, with the reason in the status line until the next refresh; Windows is asked only once per call.
fn open_links(app: &AppHandle, urls: Vec<String>) -> Result<(), String> {
    let mut program = app.state::<Shared>().inner.lock().expect("state lock").session.settings().browser.program();
    for url in urls {
        if let Some(name) = program {
            match app.opener().open_url(&url, Some(name)) {
                Ok(()) => continue,
                Err(e) => {
                    session(app, |s| s.report(format!("Couldn't start {name} ({e}); opened in the default browser")));
                    program = None;
                }
            }
        }
        app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Reads the session under the lock, without a snapshot.
fn read<T>(app: &AppHandle, f: impl FnOnce(&Session) -> T) -> T {
    f(&app.state::<Shared>().inner.lock().expect("state lock").session)
}

#[tauri::command]
fn open_pr(app: AppHandle, id: i64) -> Result<(), String> {
    let url = read(&app, |s| s.url(id));
    open_links(&app, vec![url.ok_or("Unknown pull request")?])
}

/// Opens the agent's draft for a PR. The link comes from the session, never from the window.
#[tauri::command]
fn open_draft(app: AppHandle, id: i64) -> Result<(), String> {
    let url = read(&app, |s| s.draft_url(id))?;
    open_links(&app, vec![url])
}

#[tauri::command]
fn open_row(app: AppHandle, key: String) -> Result<(), String> {
    let row = read(&app, |s| s.row(&key))?;
    open_links(&app, group::open_urls(&row))
}

#[tauri::command]
fn open_ticket(app: AppHandle, key: String) -> Result<(), String> {
    let url = read(&app, |s| s.ticket_url(&key))?;
    open_links(&app, vec![url])
}

#[tauri::command]
fn copy_row(app: AppHandle, key: String) -> Result<(), String> {
    let row = read(&app, |s| s.row(&key))?;
    app.clipboard().write_text(group::share_text(&row.members)).map_err(|e| e.to_string())
}

/// Hides or shows every PR of mine in the row.
#[tauri::command]
fn set_hidden(app: AppHandle, key: String, hidden: bool) -> Result<(), String> {
    session(&app, |s| s.set_hidden(&key, hidden))
}

#[tauri::command]
fn save_pat(app: AppHandle, workspace: String, pat: String) -> Result<(), String> {
    session(&app, |s| s.save_secret(&workspace, Provider::Ado, &pat))
}

/// Turns the workspace's Azure DevOps source off; its rows leave at the next poll.
#[tauri::command]
fn remove_pat(app: AppHandle, workspace: String) -> Result<(), String> {
    session(&app, |s| s.remove_secret(&workspace, Provider::Ado))
}

#[tauri::command]
fn remove_github_token(app: AppHandle, workspace: String) -> Result<(), String> {
    session(&app, |s| s.remove_secret(&workspace, Provider::GitHub))
}

#[tauri::command]
fn save_github_token(app: AppHandle, workspace: String, token: String) -> Result<(), String> {
    session(&app, |s| s.save_secret(&workspace, Provider::GitHub, &token))
}

/// The general settings; the workspaces change only through their own commands.
#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let change = session(&app, |s| s.save_settings(settings))?;
    if let Some(scale) = change.pill_scale {
        size_pill(&app, scale, true);
    }
    if change.list_resized && let Some(list) = app.get_webview_window(LIST) {
        place_list(&app, &list);
    }
    Ok(())
}

#[tauri::command]
fn add_workspace(app: AppHandle, name: String) -> Result<String, String> {
    session(&app, |s| s.add_workspace(&name))
}

/// Saves a workspace's name, Azure DevOps organization and teams, watched GitHub repos, and tracker.
#[tauri::command]
fn save_workspace(app: AppHandle, workspace: Workspace) -> Result<(), String> {
    session(&app, |s| s.save_workspace(workspace))
}

#[tauri::command]
fn switch_workspace(app: AppHandle, id: String) -> Result<(), String> {
    session(&app, |s| s.switch(&id))
}

/// Deletes a workspace with its secrets and data. The last one stays; deleting the active one switches to its neighbour.
#[tauri::command]
fn delete_workspace(app: AppHandle, id: String) -> Result<(), String> {
    session(&app, |s| s.delete(&id))
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
            "refresh" => session(app, |s| s.refresh(Utc::now())),
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
            let data = app.path().app_data_dir()?;
            let store = Store::new(&data, Box::new(Keyring::from_env()));
            let (session, load_warning) = Session::load(store);
            let pill_scale = session.settings().pill_scale;
            let inner = Inner { session, load_warning, list_had_focus: false, newer_release: None };
            app.manage(Shared { wake: Notify::new(), generation: watch::Sender::new(0), inner: Mutex::new(inner) });
            // The saved position is the top-left of the already-scaled pill, so size it without moving it.
            size_pill(app.handle(), pill_scale, false);
            place_pill(app.handle());
            // Only the first show honours `focus: false`; without this, showing the counter again after Esc
            // or ✕ makes it the active window, and the next keystrokes go nowhere.
            if let Some(pill) = app.get_webview_window(PILL) {
                let _ = pill.set_focusable(false);
            }
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
            tauri::async_runtime::spawn(releases::release_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_now,
            open_list,
            hide_list,
            open_pr,
            open_draft,
            open_row,
            open_ticket,
            copy_row,
            set_hidden,
            save_pat,
            save_github_token,
            remove_pat,
            remove_github_token,
            save_settings,
            add_workspace,
            save_workspace,
            switch_workspace,
            delete_workspace,
            get_autostart,
            set_autostart,
            releases::open_newer_release,
            extension::extension_status,
            extension::set_up_extension
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    /// A 1920 × 1040 work area (a 40 px taskbar below) and a 112 × 36 counter at `(x, y)`.
    const AREA: Rect = Rect { x: 0, y: 0, w: 1920, h: 1040 };
    fn counter_at(x: i32, y: i32) -> Rect {
        Rect { x, y, w: 112, h: 36 }
    }

    #[test]
    fn listRect_counterBottomRight_isTheRightEdgesBottomHalf() {
        let r = list_rect(AREA, counter_at(1796, 992), (33, 50), 360);
        assert_eq!(r, Rect { x: 1920 - 633, y: 520, w: 633, h: 520 });
    }

    #[test]
    fn listRect_counterLeftAndMidway_isTheLeftEdgeCentredOnIt() {
        let r = list_rect(AREA, counter_at(40, 400), (33, 50), 360);
        assert_eq!(r, Rect { x: 0, y: 418 - 260, w: 633, h: 520 });
    }

    #[test]
    fn listRect_counterAtTheTop_staysOnScreen() {
        assert_eq!(list_rect(AREA, counter_at(1700, 0), (33, 50), 360).y, 0);
    }

    #[test]
    fn listRect_fullSize_coversTheWholeWorkArea() {
        assert_eq!(list_rect(AREA, counter_at(1796, 992), (100, 100), 360), AREA);
    }

    #[test]
    fn listRect_narrowPercent_keepsTheMinimumWidthAgainstTheEdge() {
        let r = list_rect(AREA, counter_at(1796, 992), (20, 50), 720);
        assert_eq!((r.x, r.w), (1920 - 720, 720));
    }

    #[test]
    fn listRect_secondScreenLeftOfThePrimary_usesItsOwnEdges() {
        let area = Rect { x: -1280, y: 0, w: 1280, h: 984 };
        let r = list_rect(area, counter_at(-1200, 900), (50, 100), 360);
        assert_eq!(r, Rect { x: -1280, y: 0, w: 640, h: 984 });
    }
}
