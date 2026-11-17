//! Whether the Rado tab groups extension is loaded in Edge or Chrome, read from the browser's profile files.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::store::Browser;

/// The extension's name in `browser-extension/manifest.json`.
const NAME: &str = "Rado tab groups";
/// Chromium's `location` for an extension loaded with Load unpacked.
const UNPACKED: i64 = 4;
/// Folder names the extension has shipped under; the only clue left once its folder is gone.
const FOLDER_NAMES: [&str; 2] = ["browser-extension", "edge-extension"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// Loaded and switched on; `from` names the folder when it isn't the bundled one.
    Loaded { from: Option<String> },
    /// Loaded but switched off on the extensions page.
    Off,
    /// Loaded from a folder that no longer exists, such as an old repo checkout.
    Gone { path: String },
    NotLoaded,
}

/// The extension's status in one browser, and the folder the installer put it in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    folder: String,
    #[serde(flatten)]
    status: Status,
}

fn bundled(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app.path().resolve("browser-extension", BaseDirectory::Resource).map_err(|e| e.to_string())?;
    let text = path.to_string_lossy();
    Ok(text.strip_prefix(r"\\?\").map_or_else(|| path.clone(), PathBuf::from))
}

#[tauri::command]
pub async fn extension_status(app: AppHandle, browser: Browser) -> Result<Report, String> {
    let folder = bundled(&app)?;
    let user_data = user_data_dir(browser).ok_or("Choose Edge or Chrome first")?;
    let read = |path: PathBuf| std::fs::read_to_string(path).unwrap_or_default();
    let profile = user_data.join(last_used_profile(&read(user_data.join("Local State"))));
    let files = [read(profile.join("Preferences")), read(profile.join("Secure Preferences"))];
    let status = profile_status(&files, &folder, &|p| p.is_dir(), &manifest_name);
    Ok(Report { folder: folder.to_string_lossy().into_owned(), status })
}

/// Copies the browser's extensions page address, opens a tab to paste it in, and shows the bundled folder in Explorer.
#[tauri::command]
pub async fn set_up_extension(app: AppHandle, browser: Browser) -> Result<(), String> {
    let program = browser.program().ok_or("Choose Edge or Chrome first")?;
    let folder = bundled(&app)?;
    let page = if browser == Browser::Edge { "edge://extensions" } else { "chrome://extensions" };
    app.clipboard().write_text(page).map_err(|e| e.to_string())?;
    // The browser and Explorer take focus next; the list stays open so its steps stay readable.
    app.state::<crate::Shared>().inner.lock().expect("state lock").list_had_focus = false;
    // Chromium drops internal pages passed on its command line (Edge 157 opens a new tab), so the page is pasted.
    app.opener().open_url(page, Some(program)).map_err(|e| format!("Couldn't start {program}: {e}"))?;
    app.opener().reveal_item_in_dir(folder).map_err(|e| e.to_string())
}

fn user_data_dir(browser: Browser) -> Option<PathBuf> {
    let vendor = match browser {
        Browser::Edge => ["Microsoft", "Edge"],
        Browser::Chrome => ["Google", "Chrome"],
        Browser::System => return None,
    };
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(vendor[0]).join(vendor[1]).join("User Data"))
}

/// The profile a link opened from outside the browser lands in.
fn last_used_profile(local_state: &str) -> String {
    let parsed: Value = serde_json::from_str(local_state).unwrap_or_default();
    parsed.pointer("/profile/last_used").and_then(Value::as_str).filter(|p| !p.is_empty()).unwrap_or("Default").to_owned()
}

fn manifest_name(folder: &Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join("manifest.json")).ok()?;
    serde_json::from_str::<Value>(&text).ok()?.get("name")?.as_str().map(str::to_owned)
}

/// One profile's status from its `Preferences` and `Secure Preferences` text. Chromium keeps
/// `extensions.settings` in either file (Edge 157: `Secure Preferences` for Default, `Preferences` for
/// Profile 1) and stores no manifest for an unpacked extension, so the name is read from the folder itself.
fn profile_status(files: &[String], bundled: &Path, exists: &dyn Fn(&Path) -> bool, name_of: &dyn Fn(&Path) -> Option<String>) -> Status {
    let mut best = Status::NotLoaded;
    for text in files {
        let parsed: Value = serde_json::from_str(text).unwrap_or_default();
        let Some(settings) = parsed.pointer("/extensions/settings").and_then(Value::as_object) else { continue };
        for entry in settings.values() {
            if entry.get("location").and_then(Value::as_i64) != Some(UNPACKED) {
                continue;
            }
            let Some(path) = entry.get("path").and_then(Value::as_str) else { continue };
            let folder = Path::new(path);
            let found = if !exists(folder) {
                let last = folder.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
                if !FOLDER_NAMES.contains(&last.as_str()) {
                    continue;
                }
                Status::Gone { path: path.into() }
            } else if same_path(folder, bundled) || name_of(folder).as_deref() == Some(NAME) {
                let off = entry.get("disable_reasons").and_then(Value::as_array).is_some_and(|r| !r.is_empty());
                if off { Status::Off } else { Status::Loaded { from: (!same_path(folder, bundled)).then(|| path.into()) } }
            } else {
                continue;
            };
            if rank(&found) > rank(&best) {
                best = found;
            }
        }
    }
    best
}

fn rank(status: &Status) -> u8 {
    match status {
        Status::NotLoaded => 0,
        Status::Gone { .. } => 1,
        Status::Off => 2,
        Status::Loaded { .. } => 3,
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED: &str = r"C:\Users\me\AppData\Local\Rado\browser-extension";

    /// An unpacked entry in the shape Edge 157 writes: no manifest, an absolute path.
    fn prefs(path: &str, disable_reasons: Value) -> String {
        serde_json::json!({ "extensions": { "settings": {
            "aaaa": { "location": 5, "path": r"C:\Program Files\Edge\pdf", "manifest": { "name": "PDF Viewer" } },
            "clng": { "location": 4, "path": path, "disable_reasons": disable_reasons, "creation_flags": 38 }
        } } })
        .to_string()
    }

    fn check(files: &[String], existing: &[&str], named: &[(&str, &str)]) -> Status {
        let exists = |p: &Path| existing.iter().any(|e| same_path(p, Path::new(e)));
        let name_of = |p: &Path| named.iter().find(|(e, _)| same_path(p, Path::new(e))).map(|(_, n)| n.to_string());
        profile_status(files, Path::new(BUNDLED), &exists, &name_of)
    }

    #[test]
    fn the_bundled_folder_counts_as_loaded_whatever_its_case() {
        let files = [String::new(), prefs(&format!(r"{}\", BUNDLED.to_uppercase()), serde_json::json!([]))];
        assert_eq!(check(&files, &[BUNDLED], &[]), Status::Loaded { from: None });
    }

    #[test]
    fn another_folder_counts_by_its_manifest_name() {
        let repo = r"C:\dev\rado\browser-extension";
        let files = [prefs(repo, serde_json::json!([])), String::new()];
        assert_eq!(check(&files, &[repo], &[(repo, NAME)]), Status::Loaded { from: Some(repo.into()) });
        assert_eq!(check(&files, &[repo], &[(repo, "Something else")]), Status::NotLoaded);
    }

    #[test]
    fn a_switched_off_extension_is_off() {
        assert_eq!(check(&[prefs(BUNDLED, serde_json::json!([1]))], &[BUNDLED], &[]), Status::Off);
    }

    #[test]
    fn a_missing_folder_is_flagged_only_under_a_name_rado_used() {
        let old = r"C:\dev\rado\edge-extension";
        assert_eq!(check(&[prefs(old, serde_json::json!([]))], &[], &[]), Status::Gone { path: old.into() });
        assert_eq!(check(&[prefs(r"C:\dev\other\dist", serde_json::json!([]))], &[], &[]), Status::NotLoaded);
    }

    #[test]
    fn a_loaded_copy_wins_over_a_stale_one() {
        let files = serde_json::json!({ "extensions": { "settings": {
            "old": { "location": 4, "path": r"C:\gone\edge-extension" },
            "new": { "location": 4, "path": BUNDLED }
        } } })
        .to_string();
        assert_eq!(check(&[files], &[BUNDLED], &[]), Status::Loaded { from: None });
    }

    #[test]
    fn unreadable_files_mean_not_loaded() {
        assert_eq!(check(&["not json".into(), String::new()], &[], &[]), Status::NotLoaded);
    }

    #[test]
    fn the_last_used_profile_defaults_to_default() {
        assert_eq!(last_used_profile(""), "Default");
        assert_eq!(last_used_profile(r#"{"profile":{"last_used":"Profile 2"}}"#), "Profile 2");
    }

    #[test]
    fn the_installer_ships_every_extension_file_but_the_tests() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let conf: Value = serde_json::from_str(&std::fs::read_to_string(root.join("tauri.conf.json")).unwrap()).unwrap();
        let mut shipped: Vec<String> = conf.pointer("/bundle/resources").and_then(Value::as_object).unwrap().keys().cloned().collect();
        let mut files: Vec<String> = std::fs::read_dir(root.join("../browser-extension"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| !name.ends_with(".test.js"))
            .map(|name| format!("../browser-extension/{name}"))
            .collect();
        shipped.sort();
        files.sort();
        assert_eq!(shipped, files);
    }
}
