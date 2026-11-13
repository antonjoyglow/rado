use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::queue::{self, Queue};
use crate::score::VoteMemory;

const KEYRING_SERVICE: &str = "review-bracket";
const KEYRING_USER: &str = "ado-pat";
const KEYRING_GITHUB_USER: &str = "github-token";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub organization: String,
    pub poll_minutes: u32,
    pub notify: bool,
    /// Team reviewers whose PRs count as mine, by name (`TeamAlpha` matches `[contoso]\TeamAlpha`).
    pub teams: Vec<String>,
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
    /// Host of the Jira site ticket chips link to; empty hides the chips.
    pub jira_site: String,
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
            jira_site: jira_host(&self.jira_site),
            ..self
        }
    }
}

/// The host of a typed Jira site: `HTTPS://acme.atlassian.net/jira/boards` gives `acme.atlassian.net`.
fn jira_host(site: &str) -> String {
    let site = site.trim();
    let without_scheme = site.find("://").map_or(site, |i| &site[i + 3..]);
    without_scheme.split(['/', '?', '#', '\\']).next().unwrap_or_default().to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            organization: String::new(),
            poll_minutes: 3,
            notify: true,
            teams: Vec::new(),
            pill_scale: 100,
            list_width: 640,
            list_height: 860,
            browser: Browser::System,
            jira_site: String::new(),
        }
    }
}

pub struct Store {
    settings_path: PathBuf,
    memory_path: PathBuf,
    hidden_path: PathBuf,
    queue_path: PathBuf,
}

impl Store {
    pub fn new(config_dir: &Path, data_dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(config_dir);
        let _ = std::fs::create_dir_all(data_dir);
        Self {
            settings_path: config_dir.join("settings.json"),
            memory_path: data_dir.join("votes.json"),
            hidden_path: data_dir.join("hidden.json"),
            queue_path: data_dir.join(queue::QUEUE_FILE),
        }
    }

    pub fn settings(&self) -> Settings {
        read_json::<Settings>(&self.settings_path).unwrap_or_default().clamped()
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        write_json(&self.settings_path, settings)
    }

    pub fn memory(&self) -> VoteMemory {
        read_json(&self.memory_path).unwrap_or_default()
    }

    pub fn save_memory(&self, memory: &VoteMemory) -> Result<(), String> {
        write_json(&self.memory_path, memory)
    }

    pub fn hidden(&self) -> HashSet<i64> {
        read_json(&self.hidden_path).unwrap_or_default()
    }

    pub fn save_hidden(&self, hidden: &HashSet<i64>) -> Result<(), String> {
        write_json(&self.hidden_path, hidden)
    }

    /// For the MCP server (`rado-mcp`), which reads it and never touches the PAT.
    pub fn save_queue(&self, queue: &Queue) -> Result<(), String> {
        queue::write_json_atomic(&self.queue_path, queue)
    }

    pub fn pat(&self) -> Option<String> {
        secret(KEYRING_USER)
    }

    pub fn save_pat(&self, pat: &str) -> Result<(), String> {
        save_secret(KEYRING_USER, pat)
    }

    pub fn github_token(&self) -> Option<String> {
        secret(KEYRING_GITHUB_USER)
    }

    pub fn save_github_token(&self, token: &str) -> Result<(), String> {
        save_secret(KEYRING_GITHUB_USER, token)
    }
}

fn secret(user: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, user).ok()?.get_password().ok()
}

fn save_secret(user: &str, value: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, user)
        .and_then(|e| e.set_password(value))
        .map_err(|e| format!("Credential Manager: {e}"))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn settings_fileFromBeforePillScale_defaultsTo100() {
        let old = r#"{"organization":"org","pollMinutes":5,"notify":false,"teams":["T"]}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert_eq!((settings.poll_minutes, settings.pill_scale), (5, 100));
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
            let json = format!(r#"{{"organization":"org","teams":["T"],"pillScale":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap().clamped();
            assert_eq!((settings.organization.as_str(), settings.pill_scale), ("org", expected), "pillScale {raw}");
        }
    }

    #[test]
    fn settings_browser_readsKnownNamesAndFallsBackToSystem() {
        for (raw, expected) in [("\"chrome\"", Browser::Chrome), ("\"edge\"", Browser::Edge), ("\"firefox\"", Browser::System), ("null", Browser::System), ("7", Browser::System)] {
            let json = format!(r#"{{"organization":"org","browser":{raw}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap();
            assert_eq!((settings.organization.as_str(), settings.browser), ("org", expected), "browser {raw}");
        }
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().browser, Browser::System);
    }

    #[test]
    fn clamped_jiraSite_keepsOnlyTheHost() {
        for (typed, expected) in [
            ("acme.atlassian.net", "acme.atlassian.net"),
            (" HTTPS://acme.atlassian.net/ ", "acme.atlassian.net"),
            ("https://acme.atlassian.net/jira/software/projects/X", "acme.atlassian.net"),
            ("https://acme.atlassian.net?atlOrigin=x#top", "acme.atlassian.net"),
            (r"acme.atlassian.net\", "acme.atlassian.net"),
            ("https://", ""),
            ("  ", ""),
        ] {
            let settings = Settings { jira_site: typed.into(), ..Settings::default() }.clamped();
            assert_eq!(settings.jira_site, expected, "{typed:?}");
        }
    }

    #[test]
    fn settings_listSize_defaultsWhenMissingOrJunkAndIsClamped() {
        let old: Settings = serde_json::from_str(r#"{"organization":"org"}"#).unwrap();
        assert_eq!((old.list_width, old.list_height), (640, 860));
        for (raw_w, raw_h, expected) in [("\"wide\"", "null", (640, 860)), ("10", "99999", (360, 1600)), ("700.4", "900", (700, 900))] {
            let json = format!(r#"{{"organization":"org","listWidth":{raw_w},"listHeight":{raw_h}}}"#);
            let settings = serde_json::from_str::<Settings>(&json).unwrap().clamped();
            assert_eq!((settings.organization.as_str(), (settings.list_width, settings.list_height)), ("org", expected), "{raw_w} x {raw_h}");
        }
    }

    #[test]
    fn clamped_inRangeScale_isKeptExactly() {
        assert_eq!(Settings { pill_scale: 110, ..Settings::default() }.clamped().pill_scale, 110);
    }
}
