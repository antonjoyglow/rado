use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::queue::{self, Queue};
use crate::score::VoteMemory;
use crate::workspace::{self, Legacy, Workspace};

const KEYRING_SERVICE: &str = "review-bracket";
const MEMORY_FILE: &str = "votes.json";
const HIDDEN_FILE: &str = "hidden.json";
/// A copy of the settings file from before workspaces, written once and never overwritten.
const BACKUP_FILE: &str = "settings.pre-workspaces.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub poll_minutes: u32,
    pub notify: bool,
    /// Size of the `{ reviews | mine }` counter, in percent of the 16 px original.
    #[serde(deserialize_with = "lenient_number::<_, 100>")]
    pub pill_scale: u32,
    /// Size of the review list, in logical pixels; shrunk to fit the screen when shown.
    #[serde(deserialize_with = "lenient_number::<_, 640>")]
    pub list_width: u32,
    #[serde(deserialize_with = "lenient_number::<_, 860>")]
    pub list_height: u32,
    #[serde(deserialize_with = "lenient_browser")]
    pub browser: Browser,
    #[serde(deserialize_with = "workspace::lenient_workspaces")]
    pub workspaces: Vec<Workspace>,
    pub active_workspace: String,
    /// The workspace made from the pre-workspace settings, while their secrets and files still need moving into it.
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// Like `lenient_number`: a hand-edited `null` or unknown browser falls back to the default browser
/// instead of failing the whole file.
fn lenient_browser<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Browser, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Where Rado opens links.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Browser {
    Edge,
    Chrome,
    #[default]
    System,
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

/// Smallest and largest counter size, in percent. Below 75% the pixel font stops being readable;
/// above 200% the counter starts to cover what it floats over.
pub const PILL_SCALE_RANGE: (u32, u32) = (75, 200);
/// Smallest and largest review list, in logical pixels. Below these the rows stop fitting their buttons.
pub const LIST_WIDTH_RANGE: (u32, u32) = (360, 1600);
pub const LIST_HEIGHT_RANGE: (u32, u32) = (400, 1600);

impl Settings {
    /// Pulls values typed or edited by hand back into their allowed ranges.
    pub fn clamped(self) -> Self {
        Self {
            poll_minutes: self.poll_minutes.clamp(1, 120),
            pill_scale: self.pill_scale.clamp(PILL_SCALE_RANGE.0, PILL_SCALE_RANGE.1),
            list_width: self.list_width.clamp(LIST_WIDTH_RANGE.0, LIST_WIDTH_RANGE.1),
            list_height: self.list_height.clamp(LIST_HEIGHT_RANGE.0, LIST_HEIGHT_RANGE.1),
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
            list_width: 640,
            list_height: 860,
            browser: Browser::System,
            workspaces: Vec::new(),
            active_workspace: String::new(),
            migrating_to: None,
        }
    }
}

/// The OS credential store, behind a seam so migration can be tested without touching it.
pub trait Secrets: Send + Sync {
    fn get(&self, user: &str) -> Option<String>;
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
    fn get(&self, user: &str) -> Option<String> {
        keyring::Entry::new(&self.service, user).ok()?.get_password().ok()
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

pub struct Store {
    config_dir: PathBuf,
    data_dir: PathBuf,
    secrets: Box<dyn Secrets>,
}

impl Store {
    pub fn new(config_dir: &Path, data_dir: &Path, secrets: Box<dyn Secrets>) -> Self {
        let _ = std::fs::create_dir_all(config_dir);
        let _ = std::fs::create_dir_all(data_dir);
        Self { config_dir: config_dir.into(), data_dir: data_dir.into(), secrets }
    }

    fn settings_path(&self) -> PathBuf {
        self.config_dir.join(queue::SETTINGS_FILE)
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
        let copy = self.config_dir.join(format!("settings.unreadable-{secs}.json"));
        match std::fs::copy(path, &copy) {
            Ok(_) => format!("settings.json couldn't be read; kept it as {} and started with defaults", copy.display()),
            Err(e) => format!("settings.json couldn't be read or copied ({e}); started with defaults"),
        }
    }

    fn back_up(&self, path: &Path) -> Result<(), String> {
        let backup = self.config_dir.join(BACKUP_FILE);
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
            let Some(value) = self.secrets.get(legacy) else { continue };
            if self.secrets.get(&key).is_none() {
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

    pub fn hidden(&self, id: &str) -> HashSet<i64> {
        queue::read_json(&self.workspace_dir(id).join(HIDDEN_FILE)).unwrap_or_default()
    }

    pub fn save_hidden(&self, id: &str, hidden: &HashSet<i64>) -> Result<(), String> {
        self.write_in(id, HIDDEN_FILE, hidden)
    }

    /// For the MCP server (`rado-mcp`), which reads it and never touches the PAT.
    pub fn save_queue(&self, id: &str, queue: &Queue) -> Result<(), String> {
        self.write_in(id, queue::QUEUE_FILE, queue)
    }

    pub fn pat(&self, id: &str) -> Option<String> {
        self.secrets.get(&workspace::pat_key(id))
    }

    pub fn save_pat(&self, id: &str, pat: &str) -> Result<(), String> {
        self.secrets.set(&workspace::pat_key(id), pat)
    }

    pub fn github_token(&self, id: &str) -> Option<String> {
        self.secrets.get(&workspace::github_key(id))
    }

    pub fn save_github_token(&self, id: &str, token: &str) -> Result<(), String> {
        self.secrets.set(&workspace::github_key(id), token)
    }

    /// Removes a deleted workspace's secrets and data folder.
    pub fn forget(&self, id: &str) -> Result<(), String> {
        self.secrets.delete(&workspace::pat_key(id))?;
        self.secrets.delete(&workspace::github_key(id))?;
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

    #[derive(Clone, Default)]
    pub struct MemorySecrets {
        entries: Arc<Mutex<HashMap<String, String>>>,
        fail_writes: Arc<Mutex<bool>>,
    }

    impl MemorySecrets {
        pub fn with(entries: &[(&str, &str)]) -> Self {
            let secrets = Self::default();
            secrets.entries.lock().unwrap().extend(entries.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            secrets
        }

        fn keys(&self) -> Vec<String> {
            let mut keys: Vec<String> = self.entries.lock().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        }
    }

    impl Secrets for MemorySecrets {
        fn get(&self, user: &str) -> Option<String> {
            self.entries.lock().unwrap().get(user).cloned()
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
        Store::new(&dir.0, &dir.0, Box::new(secrets.clone()))
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
        assert_eq!(ws.tracker, Tracker::Jira { site: "acme.atlassian.net".into() });
        assert_eq!((s.active_workspace.as_str(), s.migrating_to.as_deref()), (ws.id.as_str(), None));
        assert_eq!(secrets.keys(), { let mut k = vec![github_key(&ws.id), pat_key(&ws.id)]; k.sort(); k });
        assert_eq!((secrets.get(&pat_key(&ws.id)).as_deref(), secrets.get(&github_key(&ws.id)).as_deref()), (Some("pat"), Some("gh")));
        assert_eq!(std::fs::read_to_string(dir.0.join(BACKUP_FILE)).unwrap(), OLD_FILE);
        assert_eq!(store(&dir, &secrets).hidden(&ws.id), HashSet::from([7]));
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
    fn load_newerSecretAlreadySaved_winsOverTheOldOne() {
        let dir = TempDir::new("newer");
        std::fs::write(dir.0.join("settings.json"), r#"{"workspaces":[{"id":"a1","name":"A"}],"activeWorkspace":"a1","migratingTo":"a1"}"#).unwrap();
        let secrets = MemorySecrets::with(&[(LEGACY_PAT_KEY, "old"), ("ado-pat:a1", "new")]);

        store(&dir, &secrets).load();

        assert_eq!(secrets.keys(), ["ado-pat:a1"]);
        assert_eq!(secrets.get("ado-pat:a1").as_deref(), Some("new"));
    }

    #[test]
    fn forget_removesTheWorkspacesSecretsAndFolder() {
        let dir = TempDir::new("forget");
        let secrets = MemorySecrets::with(&[("ado-pat:a1", "p"), ("github-token:a1", "g"), ("ado-pat:b2", "q")]);
        let store = store(&dir, &secrets);
        store.save_hidden("a1", &HashSet::from([1])).unwrap();

        store.forget("a1").unwrap();

        assert_eq!(secrets.keys(), ["ado-pat:b2"]);
        assert!(!queue::workspace_dir(&dir.0, "a1").exists());
    }

    #[test]
    fn clamped_outOfRangeValues_areBroughtIntoRange() {
        let low = Settings { poll_minutes: 0, pill_scale: 10, ..Settings::default() }.clamped();
        let high = Settings { poll_minutes: 999, pill_scale: 900, ..Settings::default() }.clamped();
        assert_eq!((low.poll_minutes, low.pill_scale), (1, 75));
        assert_eq!((high.poll_minutes, high.pill_scale), (120, 200));
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
    fn settings_browser_readsKnownNamesAndFallsBackToSystem() {
        for (raw, expected) in [("\"chrome\"", Browser::Chrome), ("\"edge\"", Browser::Edge), ("\"firefox\"", Browser::System), ("null", Browser::System), ("7", Browser::System)] {
            let json = format!(r#"{{"pollMinutes":7,"browser":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap();
            assert_eq!((settings.poll_minutes, settings.browser), (7, expected), "browser {raw}");
        }
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().browser, Browser::System);
    }

    #[test]
    fn settings_listSize_defaultsWhenMissingOrJunkAndIsClamped() {
        let old: Settings = serde_json::from_str(r#"{"pollMinutes":7}"#).unwrap();
        assert_eq!((old.list_width, old.list_height), (640, 860));
        for (raw_w, raw_h, expected) in [("\"wide\"", "null", (640, 860)), ("10", "99999", (360, 1600)), ("700.4", "900", (700, 900))] {
            let json = format!(r#"{{"pollMinutes":7,"listWidth":{raw_w},"listHeight":{raw_h}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap().clamped();
            assert_eq!((settings.poll_minutes, (settings.list_width, settings.list_height)), (7, expected), "{raw_w} x {raw_h}");
        }
    }

    #[test]
    fn clamped_inRangeScale_isKeptExactly() {
        assert_eq!(Settings { pill_scale: 110, ..Settings::default() }.clamped().pill_scale, 110);
    }
}
