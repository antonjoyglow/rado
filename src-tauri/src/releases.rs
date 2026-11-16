//! Watches GitHub for a newer Rado release, so the list can link to its installer. Rado never downloads it.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

const LATEST: &str = "https://api.github.com/repos/antonjoyglow/rado/releases/latest";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

/// A release newer than the running Rado, and where to get it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewerRelease {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// `v1.2.3` or `1.2.3`; anything else, a pre-release suffix included, is `None`.
fn version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.').map(|p| p.parse::<u32>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// The release when it is newer than `current`, linking to its setup exe, or to its page when it has none.
fn newer(release: &Release, current: &str) -> Option<NewerRelease> {
    let latest = version(&release.tag_name)?;
    if latest <= version(current)? {
        return None;
    }
    let setup = release.assets.iter().find(|a| a.name.ends_with("-setup.exe"));
    Some(NewerRelease {
        version: format!("{}.{}.{}", latest.0, latest.1, latest.2),
        url: setup.map_or(&release.html_url, |a| &a.browser_download_url).clone(),
    })
}

async fn latest() -> Result<Release, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?
        .get(LATEST)
        .header("User-Agent", "rado")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

/// Checks at start and every 6 hours. An offline or rate-limited check keeps what the last one found.
pub async fn release_loop(app: AppHandle) {
    let current = app.package_info().version.to_string();
    loop {
        if let Ok(release) = latest().await {
            let found = newer(&release, &current);
            crate::update(&app, |s| s.newer_release = found);
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

/// Opens the newer release's installer (or page) in the default browser.
#[tauri::command]
pub fn open_newer_release(app: AppHandle) -> Result<(), String> {
    let url = app.state::<crate::Shared>().inner.lock().expect("state lock").newer_release.as_ref().map(|r| r.url.clone());
    let url = url.ok_or("There is no newer release.")?;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: format!("https://github.com/antonjoyglow/rado/releases/tag/{tag}"),
            assets: assets
                .iter()
                .map(|name| Asset { name: (*name).into(), browser_download_url: format!("https://github.com/dl/{name}") })
                .collect(),
        }
    }

    #[test]
    fn version_readsTagsAndRejectsTheRest() {
        assert_eq!(version("v0.9.1"), Some((0, 9, 1)));
        assert_eq!(version("0.10.0"), Some((0, 10, 0)));
        for junk in ["", "v1.2", "v1.2.3.4", "v1.2.3-beta", "vx.y.z", "1..2"] {
            assert_eq!(version(junk), None, "{junk}");
        }
    }

    #[test]
    fn newer_comparesAsNumbersNotText() {
        assert!(newer(&release("v0.10.0", &[]), "0.9.1").is_some());
        assert!(newer(&release("v0.9.1", &[]), "0.9.1").is_none());
        assert!(newer(&release("v0.9.0", &[]), "0.9.1").is_none());
        assert!(newer(&release("v1.0.0-rc1", &[]), "0.9.1").is_none());
    }

    #[test]
    fn newer_linksTheSetupExe() {
        let found = newer(&release("v0.9.2", &["Rado_0.9.2_x64-setup.exe.sig", "Rado_0.9.2_x64-setup.exe", "notes.txt"]), "0.9.1");
        assert_eq!(found, Some(NewerRelease { version: "0.9.2".into(), url: "https://github.com/dl/Rado_0.9.2_x64-setup.exe".into() }));
    }

    #[test]
    fn newer_withoutAnInstaller_linksTheReleasePage() {
        let found = newer(&release("v0.9.2", &["notes.txt"]), "0.9.1").unwrap();
        assert_eq!(found.url, "https://github.com/antonjoyglow/rado/releases/tag/v0.9.2");
    }
}
