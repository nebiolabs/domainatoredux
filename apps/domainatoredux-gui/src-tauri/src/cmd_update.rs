//! "Is there a newer version?" — the network half.
//!
//! All the logic that can be wrong lives in
//! `domainator_core::app_support::version_check` and is unit-tested without a
//! network. This is only the fetch.
//!
//! Deliberately *not* `tauri-plugin-updater`: that needs a signing keypair, a
//! published `latest.json` manifest, per-artifact signatures and a release
//! pipeline, none of which this project has. It would also replace the running
//! binary in place, which for a tool that stamps `engine_version` into every
//! job's provenance is a change you opt into rather than inherit.

use domainator_core::app_support::about;
use domainator_core::app_support::prefs::update_prefs;
use domainator_core::app_support::version_check::{is_newer, pick_latest_release, UpdateCheck};
use tauri::AppHandle;

use crate::cmd_prefs::prefs_path;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Ask GitHub for the latest release and compare it to this build.
///
/// Never an error the caller should shout about: being offline, rate-limited or
/// pointed at a repository with no public releases all come back as a `note`, and
/// the UI shows it inline. The last-checked stamp is written even on failure, so
/// a broken endpoint backs off instead of being retried every launch.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateCheck, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    // ureq is blocking; same pattern as `download_external_tool`.
    let fetched = tauri::async_runtime::spawn_blocking(fetch_latest)
        .await
        .map_err(|e| format!("update check failed to start: {e}"))?;

    // Best-effort: a failure to record the stamp must not fail the check.
    if let Ok(path) = prefs_path(&app) {
        let _ = update_prefs(&path, |p| p.last_update_check_ms = Some(now_ms()));
    }

    Ok(match fetched {
        Ok(Some(rel)) => {
            let newer = is_newer(&rel.tag, &current);
            UpdateCheck {
                current,
                latest: Some(rel),
                newer,
                note: None,
            }
        }
        Ok(None) => UpdateCheck {
            current,
            latest: None,
            newer: false,
            note: Some("No public releases are available yet.".into()),
        },
        Err(note) => UpdateCheck {
            current,
            latest: None,
            newer: false,
            note: Some(note),
        },
    })
}

/// One GET against the releases API. `Ok(None)` means "reachable, nothing
/// published"; `Err` carries a message fit to show the user.
fn fetch_latest() -> Result<Option<domainator_core::app_support::version_check::ReleaseInfo>, String>
{
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build();

    let resp = agent
        .get(about::RELEASES_API)
        // GitHub 403s a request with no User-Agent, so this line is load-bearing.
        .set(
            "User-Agent",
            &format!("domainatoredux-gui/{}", env!("CARGO_PKG_VERSION")),
        )
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call();

    match resp {
        Ok(r) => {
            // `into_string` rather than `into_json`: the latter needs ureq's
            // `json` feature, and serde_json is already a dependency here.
            let text = r
                .into_string()
                .map_err(|e| format!("could not read the release list: {e}"))?;
            let body: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| format!("could not parse the release list: {e}"))?;
            Ok(pick_latest_release(&body))
        }
        // A repository with no releases (or a private one) answers 404. That is
        // not an error worth alarming anyone about.
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(ureq::Error::Status(403, r)) => {
            let limited = r.header("x-ratelimit-remaining").is_some_and(|v| v == "0");
            Err(if limited {
                "GitHub's rate limit is exhausted for this network; try again later.".into()
            } else {
                "GitHub refused the request (403).".into()
            })
        }
        Err(ureq::Error::Status(code, _)) => Err(format!("GitHub returned {code}.")),
        Err(_) => Err("Could not reach GitHub — check your network connection.".into()),
    }
}
