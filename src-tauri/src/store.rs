use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::queue::{self, Queue};
use crate::score::{Provider, VoteMemory};
use crate::workspace::{self, Legacy, Workspace};

const KEYRING_SERVICE: &str = "review-bracket";
const MEMORY_FILE: &str = "votes.json";
const HIDDEN_FILE: &str = "hidden.json";
const HIDDEN_SOURCES_FILE: &str = "hidden-sources.json";
/// A copy of the settings file from before workspaces, written once and never overwritten.
const BACKUP_FILE: &str = "settings.pre-workspaces.json";

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub poll_minutes: u32,
    pub notify: bool,
    /// Size of the `reviews <bird> mine` counter, in percent of the 16 px original.
    #[serde(deserialize_with = "lenient_number::<_, 100>")]
    pub pill_scale: u32,
    /// Text size of the review list, in percent.
    #[serde(deserialize_with = "lenient_number::<_, 100>")]
    pub text_scale: u32,
    /// Size of the review list, in percent of the work area of the counter's screen, never under 360 px wide. Files from 0.9.0 and
    /// earlier hold `listWidth` / `listHeight` in pixels instead; those are ignored.
    #[serde(deserialize_with = "lenient_number::<_, 33>")]
    pub list_width_percent: u32,
    #[serde(deserialize_with = "lenient_number::<_, 50>")]
    pub list_height_percent: u32,
    #[serde(deserialize_with = "lenient_choice")]
    pub browser: Browser,
    /// Colours of the review list; the counter has its own, in `counter`.
    #[serde(deserialize_with = "lenient_choice")]
    pub theme: Theme,
    /// Colours of the `Custom` theme; kept when another theme is chosen.
    #[serde(deserialize_with = "lenient_choice")]
    pub custom_theme: CustomTheme,
    /// Colours and opacity of the `reviews <bird> mine` counter; its size is `pill_scale`.
    #[serde(deserialize_with = "lenient_choice")]
    pub counter: CounterStyle,
    #[serde(deserialize_with = "workspace::lenient_workspaces")]
    pub workspaces: Vec<Workspace>,
    pub active_workspace: String,
    /// The workspace made from the pre-workspace settings, while their secrets and files still need moving into it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub migrating_to: Option<String>,
}

impl Settings {
    pub fn active(&self) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == self.active_workspace)
    }

    /// A file with a junk global value still keeps its workspaces; losing them would strand their secrets.
    fn lenient(file: &Value) -> Self {
        serde_json::from_value(file.clone()).unwrap_or_else(|_| Self {
            workspaces: workspace::lenient_workspaces(file.get("workspaces").cloned().unwrap_or(Value::Null)).unwrap_or_default(),
            active_workspace: file["activeWorkspace"].as_str().unwrap_or_default().to_string(),
            migrating_to: file["migratingTo"].as_str().map(str::to_string),
            ..Self::default()
        })
    }
}

/// Like `lenient_number`: a hand-edited `null`, unknown or misshapen value (browser, theme, custom colours)
/// falls back to its default instead of failing the whole file.
fn lenient_choice<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned + Default>(deserializer: D) -> Result<T, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Where Rado opens links.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum Browser {
    Edge,
    Chrome,
    #[default]
    System,
}

/// Colours of the review list. `System` follows Windows' light or dark app mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
    HighContrast,
    Custom,
}

/// The three colours a custom theme is picked from, as `#rrggbb`; the list derives the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomTheme {
    pub background: String,
    pub text: String,
    pub accent: String,
}

/// Starts from the dark palette.
impl Default for CustomTheme {
    fn default() -> Self {
        Self { background: "#1f2125".into(), text: "#e8eaee".into(), accent: "#6d9ef6".into() }
    }
}

impl CustomTheme {
    /// Each colour as lowercase `#rrggbb`; a colour that isn't one falls back to its default.
    fn clamped(self) -> Self {
        let default = Self::default();
        Self {
            background: hex_color(&self.background).unwrap_or(default.background),
            text: hex_color(&self.text).unwrap_or(default.text),
            accent: hex_color(&self.accent).unwrap_or(default.accent),
        }
    }
}

/// The counter's look. A number colour of `None` is heat: blue at 0 through to red at 10 or more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default)]
pub struct CounterStyle {
    #[serde(deserialize_with = "lenient_choice")]
    pub reviews: Option<String>,
    #[serde(deserialize_with = "lenient_choice")]
    pub mine: Option<String>,
    /// The bird between the numbers, and the `!`. Files from 0.9.0 call it `braces`, after the `{ | }` it replaced.
    #[serde(alias = "braces", deserialize_with = "lenient_choice")]
    pub logo: String,
    /// Opacity in percent, away from the cursor and near it.
    #[serde(deserialize_with = "lenient_number::<_, 55>")]
    pub rest_opacity: u32,
    #[serde(deserialize_with = "lenient_number::<_, 8>")]
    pub hover_opacity: u32,
}

/// The look before the counter could be customised.
impl Default for CounterStyle {
    fn default() -> Self {
        Self { reviews: None, mine: None, logo: "#ffffff".into(), rest_opacity: 55, hover_opacity: 8 }
    }
}

/// Lowest opacity at rest, in percent; below it the counter is too faint to find.
pub const COUNTER_REST_RANGE: (u32, u32) = (20, 100);
pub const COUNTER_HOVER_RANGE: (u32, u32) = (0, 100);

impl CounterStyle {
    /// Colours as lowercase `#rrggbb` (a junk number colour becomes heat), opacities inside their ranges.
    fn clamped(self) -> Self {
        Self {
            reviews: self.reviews.as_deref().and_then(hex_color),
            mine: self.mine.as_deref().and_then(hex_color),
            logo: hex_color(&self.logo).unwrap_or_else(|| Self::default().logo),
            rest_opacity: self.rest_opacity.clamp(COUNTER_REST_RANGE.0, COUNTER_REST_RANGE.1),
            hover_opacity: self.hover_opacity.clamp(COUNTER_HOVER_RANGE.0, COUNTER_HOVER_RANGE.1),
        }
    }
}

/// `#RRGGBB` or `rrggbb`, trimmed, as lowercase `#rrggbb`.
fn hex_color(typed: &str) -> Option<String> {
    let typed = typed.trim();
    let digits = typed.strip_prefix('#').unwrap_or(typed);
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", digits.to_ascii_lowercase()))
}

impl Browser {
    /// The program Windows resolves through its App Paths registry, or `None` for the default browser.
    pub fn program(self) -> Option<&'static str> {
        match self {
            Browser::System => None,
            Browser::Edge => Some("msedge.exe"),
            Browser::Chrome => Some("chrome.exe"),
        }
    }
}

/// A hand-edited `87.5` or `-1` must not fail the whole file and reset the other settings:
/// any number is rounded (then `clamped`), and anything else falls back to `DEFAULT`.
fn lenient_number<'de, D: serde::Deserializer<'de>, const DEFAULT: u32>(deserializer: D) -> Result<u32, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    // `as` saturates, so a negative number becomes 0 and `clamped` lifts it to the minimum.
    Ok(value.as_f64().map_or(DEFAULT, |v| v.round() as u32))
}

/// Smallest and largest counter size, in percent. Below 75% the numbers stop being readable;
/// above 200% the counter starts to cover what it floats over.
pub const PILL_SCALE_RANGE: (u32, u32) = (75, 200);
/// Smallest and largest list text size, in percent.
pub const TEXT_SCALE_RANGE: (u32, u32) = (80, 150);
/// Smallest and largest review list, in percent of its screen.
pub const LIST_WIDTH_PERCENT_RANGE: (u32, u32) = (20, 100);
pub const LIST_HEIGHT_PERCENT_RANGE: (u32, u32) = (25, 100);
/// Shortest and longest time between polls, in minutes.
pub const POLL_MINUTES_RANGE: (u32, u32) = (1, 120);

impl Settings {
    /// Pulls values typed or edited by hand back into their allowed ranges.
    pub fn clamped(self) -> Self {
        Self {
            poll_minutes: self.poll_minutes.clamp(POLL_MINUTES_RANGE.0, POLL_MINUTES_RANGE.1),
            pill_scale: self.pill_scale.clamp(PILL_SCALE_RANGE.0, PILL_SCALE_RANGE.1),
            text_scale: self.text_scale.clamp(TEXT_SCALE_RANGE.0, TEXT_SCALE_RANGE.1),
            list_width_percent: self.list_width_percent.clamp(LIST_WIDTH_PERCENT_RANGE.0, LIST_WIDTH_PERCENT_RANGE.1),
            list_height_percent: self.list_height_percent.clamp(LIST_HEIGHT_PERCENT_RANGE.0, LIST_HEIGHT_PERCENT_RANGE.1),
            custom_theme: self.custom_theme.clamped(),
            counter: self.counter.clamped(),
            workspaces: self.workspaces.into_iter().map(Workspace::clamped).collect(),
            ..self
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_minutes: 3,
            notify: true,
            pill_scale: 100,
            text_scale: 100,
            list_width_percent: 33,
            list_height_percent: 50,
            browser: Browser::System,
            theme: Theme::System,
            custom_theme: CustomTheme::default(),
            counter: CounterStyle::default(),
            workspaces: Vec::new(),
            active_workspace: String::new(),
            migrating_to: None,
        }
    }
}

/// The OS credential store, behind a seam so migration can be tested without touching it.
pub trait Secrets: Send + Sync {
    /// `Ok(None)` when there is no such entry; `Err` when the store couldn't be read.
    fn read(&self, user: &str) -> Result<Option<String>, String>;

    fn get(&self, user: &str) -> Option<String> {
        self.read(user).ok().flatten()
    }

    fn set(&self, user: &str, value: &str) -> Result<(), String>;
    /// Deleting an entry that isn't there succeeds.
    fn delete(&self, user: &str) -> Result<(), String>;
}

/// Windows Credential Manager under one service name; `RADO_KEYRING_SERVICE` overrides it for a dev build.
pub struct Keyring {
    service: String,
}

impl Keyring {
    pub fn from_env() -> Self {
        let service = std::env::var("RADO_KEYRING_SERVICE").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| KEYRING_SERVICE.into());
        Self { service }
    }
}

impl Secrets for Keyring {
    fn read(&self, user: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(&self.service, user).and_then(|e| e.get_password()) {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Credential Manager: {e}")),
        }
    }

    fn set(&self, user: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(&self.service, user)
            .and_then(|e| e.set_password(value))
            .map_err(|e| format!("Credential Manager: {e}"))
    }

    fn delete(&self, user: &str) -> Result<(), String> {
        match keyring::Entry::new(&self.service, user).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Credential Manager: {e}")),
        }
    }
}

/// The settings as loaded, and anything about the load the status line should say.
pub struct Loaded {
    pub settings: Settings,
    pub warning: Option<String>,
}

pub type Hidden = HashMap<i64, Option<Provider>>;

pub struct Store {
    /// Settings, workspaces and the queue `rado-mcp` reads: one folder, so both binaries agree on the layout.
    data_dir: PathBuf,
    secrets: Box<dyn Secrets>,
    /// Set when the settings file couldn't be copied aside, so that saving would destroy the only copy.
    frozen: AtomicBool,
}

impl Store {
    pub fn new(data_dir: &Path, secrets: Box<dyn Secrets>) -> Self {
        let _ = std::fs::create_dir_all(data_dir);
        Self { data_dir: data_dir.into(), secrets, frozen: AtomicBool::new(false) }
    }

    fn settings_path(&self) -> PathBuf {
        queue::settings_path(&self.data_dir)
    }

    /// Reads the settings, turning a file from before workspaces into one workspace. Every step can be re-run: the
    /// new file names the workspace before any secret or file moves, and the old secrets go only once the new ones read back.
    pub fn load(&self) -> Loaded {
        let mut warnings = Vec::new();
        let path = self.settings_path();
        let file = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value @ Value::Object(_)) => Some(value),
                _ => {
                    warnings.push(self.keep_unreadable(&path));
                    None
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                warnings.push(self.keep_unreadable(&path));
                None
            }
        };
        let mut settings = file.as_ref().map(Settings::lenient).unwrap_or_default();
        let mut dirty = false;
        if settings.workspaces.is_empty() {
            let legacy = Legacy::read(file.as_ref().unwrap_or(&Value::Null));
            let id = workspace::new_id();
            let has_github = self.secrets.get(workspace::LEGACY_GITHUB_KEY).is_some();
            settings.workspaces.push(legacy.into_workspace(id.clone(), has_github));
            settings.active_workspace.clone_from(&id);
            settings.migrating_to = Some(id);
            if file.is_some() && let Err(e) = self.back_up(&path) {
                self.frozen.store(true, Ordering::Relaxed);
                warnings.push(e);
                return Loaded { settings: settings.clamped(), warning: Some(warnings.join(" · ")) };
            }
            dirty = true;
        }
        dirty |= workspace::normalize(&mut settings.workspaces, &mut settings.active_workspace, workspace::new_id);
        let mut settings = settings.clamped();
        if dirty && let Err(e) = self.save_settings(&settings) {
            warnings.push(e);
            return Loaded { settings, warning: Some(warnings.join(" · ")) };
        }
        if let Some(id) = settings.migrating_to.clone() {
            let exists = settings.workspaces.iter().any(|w| w.id == id);
            match if exists { self.move_legacy_into(&id) } else { Ok(()) } {
                Ok(()) => {
                    settings.migrating_to = None;
                    if let Err(e) = self.save_settings(&settings) {
                        warnings.push(e);
                    }
                }
                Err(e) => warnings.push(format!("Couldn't finish moving your saved tokens into the workspace ({e}); restart Rado to retry")),
            }
        }
        Loaded { settings, warning: (!warnings.is_empty()).then(|| warnings.join(" · ")) }
    }

    fn keep_unreadable(&self, path: &Path) -> String {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let copy = self.data_dir.join(format!("settings.unreadable-{secs}.json"));
        match std::fs::copy(path, &copy) {
            Ok(_) => format!("settings.json couldn't be read; kept it as {} and started with defaults", copy.display()),
            Err(e) => {
                self.frozen.store(true, Ordering::Relaxed);
                format!("settings.json couldn't be read or copied ({e}); running on defaults without saving over it")
            }
        }
    }

    fn back_up(&self, path: &Path) -> Result<(), String> {
        let backup = self.data_dir.join(BACKUP_FILE);
        if backup.exists() {
            return Ok(());
        }
        std::fs::copy(path, &backup).map(|_| ()).map_err(|e| format!("Couldn't back up settings.json, so it was left as it was: {e}"))
    }

    /// Moves the pre-workspace data files and secrets into workspace `id`; a secret saved there since wins.
    fn move_legacy_into(&self, id: &str) -> Result<(), String> {
        let dir = self.workspace_dir(id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for name in [MEMORY_FILE, HIDDEN_FILE, queue::QUEUE_FILE, queue::DRAFTS_FILE] {
            let (from, to) = (self.data_dir.join(name), dir.join(name));
            if from.exists() && !to.exists() {
                std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
            }
        }
        for (legacy, key) in [(workspace::LEGACY_PAT_KEY, workspace::pat_key(id)), (workspace::LEGACY_GITHUB_KEY, workspace::github_key(id))] {
            let Some(value) = self.secrets.read(legacy)? else { continue };
            if self.secrets.read(&key)?.is_none() {
                self.secrets.set(&key, &value)?;
                if self.secrets.get(&key).as_deref() != Some(value.as_str()) {
                    return Err(format!("{key} didn't read back"));
                }
            }
            self.secrets.delete(legacy)?;
        }
        Ok(())
    }

    /// Written whole and renamed into place: `rado-mcp` reads the active workspace from it.
    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        if self.frozen.load(Ordering::Relaxed) {
            return Err("settings.json couldn't be copied aside, so Rado won't save over it this run".into());
        }
        queue::write_json_atomic(&self.settings_path(), settings)
    }

    fn workspace_dir(&self, id: &str) -> PathBuf {
        queue::workspace_dir(&self.data_dir, id)
    }

    fn write_in(&self, id: &str, name: &str, bytes: &impl Serialize) -> Result<(), String> {
        let dir = self.workspace_dir(id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        queue::write_json_atomic(&dir.join(name), bytes)
    }

    pub fn memory(&self, id: &str) -> VoteMemory {
        queue::read_json(&self.workspace_dir(id).join(MEMORY_FILE)).unwrap_or_default()
    }

    pub fn save_memory(&self, id: &str, memory: &VoteMemory) -> Result<(), String> {
        self.write_in(id, MEMORY_FILE, memory)
    }

    /// Hidden PR ids, each with the source it came from; `None` when none is recorded or the entry is unreadable.
    pub fn hidden(&self, id: &str) -> Hidden {
        let dir = self.workspace_dir(id);
        let ids: HashSet<i64> = queue::read_json(&dir.join(HIDDEN_FILE)).unwrap_or_default();
        let sources: HashMap<String, Value> = queue::read_json(&dir.join(HIDDEN_SOURCES_FILE)).unwrap_or_default();
        let source = |id: i64| sources.get(&id.to_string()).and_then(|v| serde_json::from_value(v.clone()).ok());
        ids.into_iter().map(|id| (id, source(id))).collect()
    }

    /// The ids stay a plain array in `hidden.json`, which versions before 0.10.0 read; sources go beside it.
    pub fn save_hidden(&self, id: &str, hidden: &Hidden) -> Result<(), String> {
        let ids: HashSet<i64> = hidden.keys().copied().collect();
        let sources: HashMap<i64, Provider> = hidden.iter().filter_map(|(id, source)| Some((*id, (*source)?))).collect();
        self.write_in(id, HIDDEN_SOURCES_FILE, &sources)?;
        self.write_in(id, HIDDEN_FILE, &ids)
    }

    /// For the MCP server (`rado-mcp`), which reads it and never touches the PAT.
    pub fn save_queue(&self, id: &str, queue: &Queue) -> Result<(), String> {
        self.write_in(id, queue::QUEUE_FILE, queue)
    }

    /// The PRs `rado-mcp` marked drafted at the commit `commits` gives them, with the draft's link. Read only: a
    /// missing or damaged file means no marker, and the server reports the damage to the agent.
    pub fn agent_drafts(&self, id: &str, commits: &HashMap<i64, String>) -> HashMap<i64, Option<String>> {
        let dir = self.workspace_dir(id);
        // Marks before links: rado-mcp writes links first, so a new mark is never read with an old link.
        let drafts = queue::read_json(&dir.join(queue::DRAFTS_FILE)).unwrap_or_default();
        let links = queue::read_json(&dir.join(queue::DRAFT_LINKS_FILE)).unwrap_or_default();
        queue::agent_drafts(commits, &drafts, &links)
    }

    /// The PAT, `None` when none is saved, or the error when Credential Manager couldn't be read.
    pub fn read_pat(&self, id: &str) -> Result<Option<String>, String> {
        self.secrets.read(&workspace::pat_key(id))
    }

    pub fn read_github_token(&self, id: &str) -> Result<Option<String>, String> {
        self.secrets.read(&workspace::github_key(id))
    }

    pub fn save_pat(&self, id: &str, pat: &str) -> Result<(), String> {
        self.secrets.set(&workspace::pat_key(id), pat)
    }

    pub fn save_github_token(&self, id: &str, token: &str) -> Result<(), String> {
        self.secrets.set(&workspace::github_key(id), token)
    }

    pub fn remove_queue(&self, id: &str) -> Result<(), String> {
        let path = self.workspace_dir(id).join(queue::QUEUE_FILE);
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
            _ => Ok(()),
        }
    }

    pub fn remove_pat(&self, id: &str) -> Result<(), String> {
        self.secrets.delete(&workspace::pat_key(id))
    }

    pub fn remove_github_token(&self, id: &str) -> Result<(), String> {
        self.secrets.delete(&workspace::github_key(id))
    }

    /// Removes a deleted workspace's secrets and data folder.
    pub fn forget(&self, id: &str) -> Result<(), String> {
        self.remove_pat(id)?;
        self.remove_github_token(id)?;
        let dir = self.workspace_dir(id);
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", dir.display())),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::workspace::{LEGACY_GITHUB_KEY, LEGACY_PAT_KEY, Tracker, github_key, pat_key};

    /// Writes the limits and defaults the frontend imports next to the ts-rs types; CI fails when the
    /// committed file differs, so the two sides can't drift apart.
    #[test]
    fn export_limits() {
        let range = |name: &str, (min, max): (u32, u32)| format!("export const {name}: readonly [number, number] = [{min}, {max}];\n");
        let defaults = Settings::default();
        let text = [
            "// Generated from the Rust constants by `cargo test` (store.rs, `export_limits`). Do not edit this file manually.\n".to_string(),
            "import type { CounterStyle } from \"./CounterStyle\";\n\n".to_string(),
            range("POLL_MINUTES", POLL_MINUTES_RANGE),
            range("PILL_SCALE", PILL_SCALE_RANGE),
            range("TEXT_SCALE", TEXT_SCALE_RANGE),
            range("LIST_WIDTH_PERCENT", LIST_WIDTH_PERCENT_RANGE),
            range("LIST_HEIGHT_PERCENT", LIST_HEIGHT_PERCENT_RANGE),
            range("COUNTER_REST_OPACITY", COUNTER_REST_RANGE),
            range("COUNTER_HOVER_OPACITY", COUNTER_HOVER_RANGE),
            format!("export const DEFAULT_PILL_SCALE = {};\n", defaults.pill_scale),
            format!("export const COUNTER_DEFAULT: Readonly<CounterStyle> = {};\n", serde_json::to_string(&defaults.counter).unwrap()),
        ]
        .concat();
        let dir = std::env::var("TS_RS_EXPORT_DIR").expect("TS_RS_EXPORT_DIR, set in .cargo/config.toml");
        std::fs::write(std::path::Path::new(&dir).join("limits.ts"), text).unwrap();
    }

    #[derive(Clone, Default)]
    pub struct MemorySecrets {
        entries: Arc<Mutex<HashMap<String, String>>>,
        fail_writes: Arc<Mutex<bool>>,
        fail_reads: Arc<Mutex<bool>>,
    }

    impl MemorySecrets {
        pub fn with(entries: &[(&str, &str)]) -> Self {
            let secrets = Self::default();
            secrets.entries.lock().unwrap().extend(entries.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            secrets
        }

        /// Makes every read fail, as a locked Credential Manager does.
        pub fn fail_reads(&self, fail: bool) {
            *self.fail_reads.lock().unwrap() = fail;
        }

        fn keys(&self) -> Vec<String> {
            let mut keys: Vec<String> = self.entries.lock().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        }
    }

    impl Secrets for MemorySecrets {
        fn read(&self, user: &str) -> Result<Option<String>, String> {
            if *self.fail_reads.lock().unwrap() {
                return Err("unavailable".into());
            }
            Ok(self.entries.lock().unwrap().get(user).cloned())
        }

        fn set(&self, user: &str, value: &str) -> Result<(), String> {
            if *self.fail_writes.lock().unwrap() {
                return Err("locked".into());
            }
            self.entries.lock().unwrap().insert(user.into(), value.into());
            Ok(())
        }

        fn delete(&self, user: &str) -> Result<(), String> {
            self.entries.lock().unwrap().remove(user);
            Ok(())
        }
    }

    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rado-store-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(dir: &TempDir, secrets: &MemorySecrets) -> Store {
        Store::new(&dir.0, Box::new(secrets.clone()))
    }

    #[test]
    fn saveSettings_landsWhereRadoMcpReadsTheWorkspaces() {
        let dir = TempDir::new("settings-path");
        let settings = Settings { active_workspace: "w1".into(), ..Settings::default() };
        store(&dir, &MemorySecrets::default()).save_settings(&settings).unwrap();
        // Spelled out, not through settings_path: installed rado-mcp builds and the README rely on this exact place.
        let read: serde_json::Value = queue::read_json_file(&dir.0.join("settings.json")).unwrap().expect("written");
        assert_eq!(read["activeWorkspace"], "w1");
    }

    const OLD_FILE: &str = r#"{"organization":"contoso","pollMinutes":5,"notify":false,"teams":["TeamAlpha"],"pillScale":120,"jiraSite":"acme.atlassian.net"}"#;

    #[test]
    fn load_oldFileWithBothSecrets_becomesOneWorkspaceAndMovesEverything() {
        let dir = TempDir::new("migrate");
        std::fs::write(dir.0.join("settings.json"), OLD_FILE).unwrap();
        std::fs::write(dir.0.join("hidden.json"), "[7]").unwrap();
        let secrets = MemorySecrets::with(&[(LEGACY_PAT_KEY, "pat"), (LEGACY_GITHUB_KEY, "gh")]);

        let loaded = store(&dir, &secrets).load();

        assert_eq!(loaded.warning, None);
        let s = &loaded.settings;
        assert_eq!((s.poll_minutes, s.notify, s.pill_scale), (5, false, 120));
        let [ws] = s.workspaces.as_slice() else { panic!("one workspace: {:?}", s.workspaces) };
        assert_eq!((ws.name.as_str(), ws.azure_dev_ops.organization.as_str(), ws.azure_dev_ops.teams.as_slice()), ("contoso", "contoso", ["TeamAlpha".to_string()].as_slice()));
        assert_eq!(ws.tracker, Tracker::Jira { site: "acme.atlassian.net".into(), keys: Vec::new() });
        assert_eq!((s.active_workspace.as_str(), s.migrating_to.as_deref()), (ws.id.as_str(), None));
        assert_eq!(secrets.keys(), { let mut k = vec![github_key(&ws.id), pat_key(&ws.id)]; k.sort(); k });
        assert_eq!((secrets.get(&pat_key(&ws.id)).as_deref(), secrets.get(&github_key(&ws.id)).as_deref()), (Some("pat"), Some("gh")));
        assert_eq!(std::fs::read_to_string(dir.0.join(BACKUP_FILE)).unwrap(), OLD_FILE);
        assert_eq!(store(&dir, &secrets).hidden(&ws.id), Hidden::from([(7, None)]));
        assert!(!dir.0.join("hidden.json").exists());

        let again = store(&dir, &secrets).load();
        assert_eq!((again.settings.workspaces.len(), again.settings.active_workspace.as_str(), again.warning), (1, ws.id.as_str(), None));
    }

    #[test]
    fn load_oldFileWithNeitherSource_isOneDefaultWorkspace() {
        let dir = TempDir::new("neither");
        std::fs::write(dir.0.join("settings.json"), r#"{"pollMinutes":10}"#).unwrap();
        let secrets = MemorySecrets::default();

        let loaded = store(&dir, &secrets).load();

        let [ws] = loaded.settings.workspaces.as_slice() else { panic!() };
        assert_eq!((ws.name.as_str(), &ws.tracker, loaded.settings.poll_minutes), ("Default", &Tracker::None, 10));
        assert!(secrets.keys().is_empty());
    }

    #[test]
    fn load_githubTokenOnly_isNamedGitHub() {
        let dir = TempDir::new("github");
        let secrets = MemorySecrets::with(&[(LEGACY_GITHUB_KEY, "gh")]);

        let loaded = store(&dir, &secrets).load();

        assert_eq!(loaded.settings.workspaces[0].name, "GitHub");
        assert_eq!(secrets.keys(), [github_key(&loaded.settings.active_workspace)]);
    }

    #[test]
    fn load_corruptFile_isKeptAsACopyAndTheSecretsStillMove() {
        let dir = TempDir::new("corrupt");
        std::fs::write(dir.0.join("settings.json"), "{ not json").unwrap();
        let secrets = MemorySecrets::with(&[(LEGACY_PAT_KEY, "pat")]);

        let loaded = store(&dir, &secrets).load();

        assert!(loaded.warning.unwrap().contains("couldn't be read"));
        let copies: Vec<String> = std::fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("settings.unreadable-"))
            .collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(std::fs::read_to_string(dir.0.join(&copies[0])).unwrap(), "{ not json");
        assert_eq!(secrets.get(&pat_key(&loaded.settings.active_workspace)).as_deref(), Some("pat"));
    }

    #[test]
    fn load_junkGlobalValue_keepsTheWorkspaces() {
        let dir = TempDir::new("junk");
        std::fs::write(dir.0.join("settings.json"), r#"{"pollMinutes":"often","workspaces":[{"id":"a1","name":"Acme"}],"activeWorkspace":"a1"}"#).unwrap();

        let loaded = store(&dir, &MemorySecrets::default()).load();

        assert_eq!((loaded.settings.active_workspace.as_str(), loaded.settings.workspaces[0].name.as_str(), loaded.settings.poll_minutes), ("a1", "Acme", 3));
    }

    #[test]
    fn load_secretWriteFails_keepsTheOldEntryAndFinishesOnTheNextStartWithTheSameId() {
        let dir = TempDir::new("retry");
        std::fs::write(dir.0.join("settings.json"), OLD_FILE).unwrap();
        let secrets = MemorySecrets::with(&[(LEGACY_PAT_KEY, "pat")]);
        *secrets.fail_writes.lock().unwrap() = true;

        let first = store(&dir, &secrets).load();

        assert!(first.warning.unwrap().contains("restart Rado"));
        assert_eq!(secrets.keys(), [LEGACY_PAT_KEY]);
        let id = first.settings.active_workspace;

        *secrets.fail_writes.lock().unwrap() = false;
        let second = store(&dir, &secrets).load();

        assert_eq!((second.settings.active_workspace.as_str(), second.settings.migrating_to, second.warning), (id.as_str(), None, None));
        assert_eq!(secrets.keys(), [pat_key(&id)]);
    }

    #[test]
    fn load_credentialStoreUnreadable_keepsTheOldEntryAndRetriesNextStart() {
        let dir = TempDir::new("unreadable");
        std::fs::write(dir.0.join("settings.json"), OLD_FILE).unwrap();
        let secrets = MemorySecrets::with(&[(LEGACY_PAT_KEY, "pat")]);
        *secrets.fail_reads.lock().unwrap() = true;

        let first = store(&dir, &secrets).load();

        assert!(first.settings.migrating_to.is_some());
        *secrets.fail_reads.lock().unwrap() = false;
        let id = first.settings.active_workspace;
        store(&dir, &secrets).load();
        assert_eq!(secrets.keys(), [pat_key(&id)]);
    }

    #[test]
    fn load_fileThatCannotBeReadOrCopied_isNeverSavedOver() {
        let dir = TempDir::new("uncopyable");
        std::fs::create_dir(dir.0.join("settings.json")).unwrap();
        std::fs::write(dir.0.join("settings.json").join("inside"), "keep").unwrap();
        let store = store(&dir, &MemorySecrets::default());

        let loaded = store.load();

        assert!(loaded.warning.unwrap().contains("without saving over it"));
        assert_eq!(loaded.settings.workspaces.len(), 1);
        // With the obstacle gone, only the guard stops the save.
        std::fs::remove_dir_all(dir.0.join("settings.json")).unwrap();
        assert!(store.save_settings(&loaded.settings).is_err());
        assert!(!dir.0.join("settings.json").exists());
    }

    #[test]
    fn load_newerSecretAlreadySaved_winsOverTheOldOne() {
        let dir = TempDir::new("newer");
        std::fs::write(dir.0.join("settings.json"), r#"{"workspaces":[{"id":"a1","name":"A"}],"activeWorkspace":"a1","migratingTo":"a1"}"#).unwrap();
        let secrets = MemorySecrets::with(&[(LEGACY_PAT_KEY, "old"), ("ado-pat:a1", "new")]);

        store(&dir, &secrets).load();

        assert_eq!(secrets.keys(), ["ado-pat:a1"]);
        assert_eq!(secrets.get("ado-pat:a1").as_deref(), Some("new"));
    }

    #[test]
    fn hidden_roundTripsSources_andReadsTheOldIdOnlyFile() {
        let dir = TempDir::new("hidden");
        let secrets = MemorySecrets::with(&[]);
        let store = store(&dir, &secrets);
        let hidden = Hidden::from([(1, Some(Provider::Ado)), (2, Some(Provider::GitHub)), (3, None)]);
        store.save_hidden("a1", &hidden).unwrap();
        assert_eq!(store.hidden("a1"), hidden);

        let folder = queue::workspace_dir(&dir.0, "a1");
        let ids: Vec<i64> = serde_json::from_slice(&std::fs::read(folder.join(HIDDEN_FILE)).unwrap()).unwrap();
        assert_eq!(ids.len(), 3, "hidden.json stays the id array that older versions read");

        std::fs::write(folder.join(HIDDEN_SOURCES_FILE), r#"{"4":"ado","5":"someday","x":"gitHub"}"#).unwrap();
        std::fs::write(folder.join(HIDDEN_FILE), "[4, 5, 6]").unwrap();
        assert_eq!(store.hidden("a1"), Hidden::from([(4, Some(Provider::Ado)), (5, None), (6, None)]));
    }

    #[test]
    fn removeQueue_deletesTheFile_andAMissingOneIsFine() {
        let dir = TempDir::new("remove-queue");
        let secrets = MemorySecrets::with(&[]);
        let store = store(&dir, &secrets);
        store.save_queue("a1", &queue::tests::queue(vec![])).unwrap();

        store.remove_queue("a1").unwrap();
        store.remove_queue("a1").unwrap();

        assert!(!queue::workspace_dir(&dir.0, "a1").join(queue::QUEUE_FILE).exists());
    }

    #[test]
    fn forget_removesTheWorkspacesSecretsAndFolder() {
        let dir = TempDir::new("forget");
        let secrets = MemorySecrets::with(&[("ado-pat:a1", "p"), ("github-token:a1", "g"), ("ado-pat:b2", "q")]);
        let store = store(&dir, &secrets);
        store.save_hidden("a1", &Hidden::from([(1, None)])).unwrap();

        store.forget("a1").unwrap();

        assert_eq!(secrets.keys(), ["ado-pat:b2"]);
        assert!(!queue::workspace_dir(&dir.0, "a1").exists());
    }

    #[test]
    fn clamped_outOfRangeValues_areBroughtIntoRange() {
        let low = Settings { poll_minutes: 0, pill_scale: 10, text_scale: 10, ..Settings::default() }.clamped();
        let high = Settings { poll_minutes: 999, pill_scale: 900, text_scale: 900, ..Settings::default() }.clamped();
        assert_eq!((low.poll_minutes, low.pill_scale, low.text_scale), (1, 75, 80));
        assert_eq!((high.poll_minutes, high.pill_scale, high.text_scale), (120, 200, 150));
    }

    #[test]
    fn settings_handEditedOddScale_keepsTheOtherSettings() {
        for (raw, expected) in [("87.4", 87), ("-1", 75), ("\"big\"", 100), ("null", 100)] {
            let json = format!(r#"{{"pollMinutes":7,"pillScale":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap().clamped();
            assert_eq!((settings.poll_minutes, settings.pill_scale), (7, expected), "pillScale {raw}");
        }
    }

    #[test]
    fn settings_textScale_defaultsTo100AndToleratesHandEdits() {
        assert_eq!(serde_json::from_str::<Settings>(r#"{"pollMinutes":7}"#).unwrap().text_scale, 100);
        for (raw, expected) in [("117", 117), ("99.6", 100), ("-1", 80), ("400", 150), ("\"big\"", 100), ("null", 100)] {
            let json = format!(r#"{{"pollMinutes":7,"textScale":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap().clamped();
            assert_eq!((settings.poll_minutes, settings.text_scale), (7, expected), "textScale {raw}");
        }
    }

    #[test]
    fn settings_browser_readsKnownNamesAndFallsBackToSystem() {
        for (raw, expected) in [("\"chrome\"", Browser::Chrome), ("\"edge\"", Browser::Edge), ("\"firefox\"", Browser::System), ("null", Browser::System), ("7", Browser::System)] {
            let json = format!(r#"{{"pollMinutes":7,"browser":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap();
            assert_eq!((settings.poll_minutes, settings.browser), (7, expected), "browser {raw}");
        }
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().browser, Browser::System);
    }

    #[test]
    fn settings_theme_readsKnownNamesAndFallsBackToSystem() {
        for (raw, expected) in [
            ("\"dark\"", Theme::Dark),
            ("\"light\"", Theme::Light),
            ("\"highContrast\"", Theme::HighContrast),
            ("\"custom\"", Theme::Custom),
            ("\"system\"", Theme::System),
            ("\"sepia\"", Theme::System),
            ("null", Theme::System),
            ("1", Theme::System),
        ] {
            let json = format!(r#"{{"pollMinutes":7,"theme":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap();
            assert_eq!((settings.poll_minutes, settings.theme), (7, expected), "theme {raw}");
        }
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().theme, Theme::System);
        assert_eq!(serde_json::to_value(Theme::HighContrast).unwrap(), "highContrast");
    }

    #[test]
    fn clamped_customTheme_normalisesHexAndReplacesJunkPerColour() {
        let json = r##"{"pollMinutes":7,"customTheme":{"background":" FFAA00 ","text":"#12345G","accent":"#ABCDEF"}}"##;
        let settings = serde_json::from_str::<Settings>(json).unwrap().clamped();
        let expected = CustomTheme { background: "#ffaa00".into(), text: CustomTheme::default().text, accent: "#abcdef".into() };
        assert_eq!((settings.poll_minutes, settings.custom_theme), (7, expected));
    }

    #[test]
    fn settings_customThemeOfWrongShape_fallsBackToDefaultColours() {
        for raw in ["null", "\"red\"", "[1,2]", r#"{"background":7}"#, r###"{"background":"##1f2125"}"###] {
            let json = format!(r#"{{"pollMinutes":7,"customTheme":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap().clamped();
            assert_eq!((settings.poll_minutes, settings.custom_theme), (7, CustomTheme::default()), "customTheme {raw}");
        }
    }

    #[test]
    fn settings_listSize_defaultsWhenMissingOrJunkAndIsClamped() {
        for (raw_w, raw_h, expected) in [("\"wide\"", "null", (33, 50)), ("10", "140", (20, 100)), ("40.4", "75", (40, 75))] {
            let json = format!(r#"{{"pollMinutes":7,"listWidthPercent":{raw_w},"listHeightPercent":{raw_h}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap().clamped();
            assert_eq!((settings.poll_minutes, (settings.list_width_percent, settings.list_height_percent)), (7, expected), "{raw_w} x {raw_h}");
        }
    }

    #[test]
    fn settings_090PixelListSize_isIgnoredForTheDefaults() {
        let old = serde_json::from_str::<Settings>(r#"{"pollMinutes":7,"listWidth":820,"listHeight":1040}"#).unwrap().clamped();
        assert_eq!((old.poll_minutes, old.list_width_percent, old.list_height_percent), (7, 33, 50));
        let saved = serde_json::to_value(&old).unwrap();
        assert!(saved.get("listWidth").is_none() && saved.get("listHeight").is_none());
    }

    #[test]
    fn settings_withoutCounter_loadsTheOriginalLook() {
        let old: Settings = serde_json::from_str(r#"{"pollMinutes":7,"pillScale":125}"#).unwrap();
        let old = old.clamped();
        assert_eq!((old.poll_minutes, old.pill_scale, old.counter), (7, 125, CounterStyle::default()));
        assert_eq!(CounterStyle::default(), CounterStyle { reviews: None, mine: None, logo: "#ffffff".into(), rest_opacity: 55, hover_opacity: 8 });
    }

    #[test]
    fn clamped_counter_normalisesColoursAndClampsOpacity() {
        let json = r##"{"pollMinutes":7,"counter":{"reviews":" FFAA00 ","mine":"#12345G","logo":"pink","restOpacity":5,"hoverOpacity":140.4}}"##;
        let settings = serde_json::from_str::<Settings>(json).unwrap().clamped();
        let expected = CounterStyle { reviews: Some("#ffaa00".into()), mine: None, logo: "#ffffff".into(), rest_opacity: 20, hover_opacity: 100 };
        assert_eq!((settings.poll_minutes, settings.counter), (7, expected));
    }

    #[test]
    fn settings_090CounterWithBraces_loadsItAsTheLogoColour() {
        let json = r##"{"pollMinutes":7,"counter":{"reviews":"#f26b6b","mine":null,"braces":"#C792EA","restOpacity":80,"hoverOpacity":25}}"##;
        let settings = serde_json::from_str::<Settings>(json).unwrap().clamped();
        let expected = CounterStyle { reviews: Some("#f26b6b".into()), mine: None, logo: "#c792ea".into(), rest_opacity: 80, hover_opacity: 25 };
        assert!(serde_json::to_value(&settings).unwrap()["counter"].get("braces").is_none());
        assert_eq!((settings.poll_minutes, settings.counter), (7, expected));
    }

    #[test]
    fn settings_counterFieldsOfWrongShape_fallBackPerField() {
        let json = r##"{"pollMinutes":7,"counter":{"reviews":7,"mine":"#00ff00","logo":null,"restOpacity":"high","hoverOpacity":[1]}}"##;
        let settings = serde_json::from_str::<Settings>(json).unwrap().clamped();
        let expected = CounterStyle { mine: Some("#00ff00".into()), ..CounterStyle::default() };
        assert_eq!((settings.poll_minutes, settings.counter), (7, expected));
        for raw in ["null", "\"red\"", "[1,2]"] {
            let json = format!(r#"{{"pollMinutes":7,"counter":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap().clamped();
            assert_eq!((settings.poll_minutes, settings.counter), (7, CounterStyle::default()), "counter {raw}");
        }
    }

    #[test]
    fn counter_serialisesHeatAsNullInCamelCase() {
        let value = serde_json::to_value(CounterStyle::default()).unwrap();
        assert_eq!(value, serde_json::json!({"reviews":null,"mine":null,"logo":"#ffffff","restOpacity":55,"hoverOpacity":8}));
    }

    #[test]
    fn clamped_inRangeScale_isKeptExactly() {
        assert_eq!(Settings { pill_scale: 110, ..Settings::default() }.clamped().pill_scale, 110);
    }
}
