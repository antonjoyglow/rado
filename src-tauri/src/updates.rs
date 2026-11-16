//! Checks GitHub Releases for a newer Rado and installs it: by itself while the list is closed when
//! `auto_update` is on, or from Settings → General. Installing quits Rado; the installer restarts it.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The first check waits for start-up and the first poll to settle.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// How often a found update looks again whether the list has closed.
const LIST_OPEN_RETRY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    /// The running version.
    pub current: String,
    pub available: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
    /// Download progress in percent while an update installs.
    pub installing: Option<u8>,
    pub error: Option<String>,
}

fn list_open(app: &AppHandle) -> bool {
    app.get_webview_window(crate::LIST).is_some_and(|w| w.is_visible().unwrap_or(false))
}

fn auto_update(app: &AppHandle) -> bool {
    app.state::<crate::Shared>().inner.lock().expect("state lock").settings.auto_update
}

/// Asks the release feed and records the answer; a newer version comes back to be installed.
async fn check(app: &AppHandle) -> Option<Update> {
    let result = if cfg!(debug_assertions) {
        Err("a dev build doesn't check".to_string())
    } else {
        match app.updater() {
            Ok(updater) => updater.check().await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    };
    crate::update(app, |s| {
        s.update.checked_at = Some(Utc::now());
        match &result {
            Ok(found) => {
                s.update.available = found.as_ref().map(|u| u.version.clone());
                s.update.error = None;
            }
            Err(e) => s.update.error = Some(format!("Couldn't check for updates: {e}")),
        }
    });
    result.ok().flatten()
}

/// Downloads and starts the installer. On Windows the process exits once the installer runs, so a return is a failure.
async fn install(app: &AppHandle, update: Update) -> Result<(), String> {
    // Taken under the state lock, so the loop and the Install button can't both start one.
    let mut started = false;
    crate::update(app, |s| {
        if s.update.installing.is_none() {
            s.update.installing = Some(0);
            started = true;
        }
    });
    if !started {
        return Err("An update is already installing.".into());
    }
    let (mut received, mut shown) = (0u64, 0u8);
    let result = update
        .download_and_install(
            |chunk, total| {
                received += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let percent = (received * 100 / total).min(100) as u8;
                    if percent != shown {
                        shown = percent;
                        crate::update(app, |s| s.update.installing = Some(percent));
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("Couldn't install {}: {e}", update.version));
    crate::update(app, |s| {
        s.update.installing = None;
        s.update.error = result.as_ref().err().cloned();
    });
    result
}

/// Checks a minute after start and every 6 hours; with `auto_update` on, installs what it finds once the list is closed.
pub async fn update_loop(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    tokio::time::sleep(FIRST_CHECK).await;
    loop {
        if let Some(found) = check(&app).await {
            while auto_update(&app) && list_open(&app) {
                tokio::time::sleep(LIST_OPEN_RETRY).await;
            }
            if auto_update(&app) {
                let _ = install(&app, found).await;
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

#[tauri::command]
pub async fn check_update(app: AppHandle) {
    check(&app).await;
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let found = check(&app).await.ok_or("There is no newer version to install.")?;
    install(&app, found).await
}
