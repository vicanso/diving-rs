//! GitHub Releases self-updater for Diving Desktop (`vicanso/diving`).
//!
//! Checks `https://api.github.com/repos/vicanso/diving/releases/latest` at most once
//! every 48 hours in `~/.diving/update-check.json`, downloads the matching platform
//! asset with progress updates and retries, verifies SHA-256 against `SHA256SUMS.txt`
//! (falling back to `SHA256SUMS`), installs in place, and relaunches.

use std::cmp::Ordering as CmpOrdering;
use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
#[cfg(any(target_os = "windows", test))]
use std::io;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diving_app::logging::install_crypto_provider;
#[cfg(any(target_os = "linux", test))]
use flate2::read::GzDecoder;
use home::home_dir;
use reqwest::{Client, Proxy};
use ring::digest::{Context as Sha256Context, SHA256};
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "linux", test))]
use tar::Archive;
use tokio::runtime::Builder;
use tokio::time::sleep;
#[cfg(any(target_os = "windows", test))]
use zip::ZipArchive;

/// Running Diving Desktop semantic version compiled into this binary (preferring `DIVING_APP_VERSION` on nightly/CI builds).
pub const CURRENT_VERSION: &str = match option_env!("DIVING_APP_VERSION") {
    Some(ver) if !ver.is_empty() => ver,
    _ => env!("CARGO_PKG_VERSION"),
};

/// GitHub repository slug hosting Diving Desktop releases.
pub const REPO_SLUG: &str = "vicanso/diving";

/// Latest release endpoint on the GitHub REST API.
pub const RELEASE_API_URL: &str = "https://api.github.com/repos/vicanso/diving/releases/latest";

/// Releases list endpoint used to stitch intermediate release notes.
pub const RELEASE_LIST_URL: &str = "https://api.github.com/repos/vicanso/diving/releases";

/// Base URL prefix for release artifact downloads.
pub const RELEASES_BASE_URL: &str = "https://github.com/vicanso/diving/releases/download";

/// Web releases page fallback when manual download is needed.
pub const RELEASES_PAGE_URL: &str = "https://github.com/vicanso/diving/releases";

/// macOS bundle identifier verified against mounted `.dmg` payloads before swap.
pub const BUNDLE_ID: &str = "io.github.vicanso.diving";

/// macOS application bundle directory name inside the DMG and install destination.
pub const APP_BUNDLE_NAME: &str = "Diving.app";

/// macOS installer volume prefix swept on startup and during DMG mounts.
pub const INSTALLER_VOLUME_PREFIX: &str = "Diving Installer";

/// Cadence between silent background release checks (48 hours).
pub const AUTO_CHECK_EVERY: Duration = Duration::from_secs(48 * 60 * 60);

const STATE_FILE_NAME: &str = "update-check.json";
const CHECKSUM_ASSETS: [&str; 2] = ["SHA256SUMS.txt", "SHA256SUMS"];
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const DOWNLOAD_ATTEMPTS: usize = 3;
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
const HISTORY_PAGE_SIZE: usize = 30;
const USER_AGENT: &str = concat!("Diving-Desktop/", env!("CARGO_PKG_VERSION"));
#[cfg(target_os = "macos")]
const LSREGISTER_PATH: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
#[cfg(target_os = "macos")]
const PREVIOUS_BUNDLE_PREFIX: &str = "diving-previous-";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const UNPACK_PREFIX: &str = "diving-update-";

/// Result of checking GitHub Releases against the running version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    UpToDate,
    Newer { version: String, notes: String },
    Failed(String),
}

/// Outcome of installing a verified release asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    /// Updated the running install in place; relaunch completes the swap.
    Relaunch { target: PathBuf },
    /// Could not swap automatically; opened the installer artifact in the OS file manager.
    OpenedInstaller { path: PathBuf },
    /// Binary staged beside the running executable; quit completes the replacement.
    QuitToInstall { target: PathBuf },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct CheckFile {
    #[serde(default)]
    checked_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ignored: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
}

/// Returns the expected release asset filename for the host OS and CPU architecture.
pub fn asset_name() -> Option<&'static str> {
    asset_name_for(env::consts::OS, env::consts::ARCH)
}

/// Maps an `(os, arch)` tuple matching `.github/workflows/publish.yml` to its asset name.
pub fn asset_name_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64" | "x86_64") => Some("Diving-macos-universal.dmg"),
        ("linux", "x86_64") => Some("Diving-linux-x86_64.tar.gz"),
        ("linux", "aarch64") => Some("Diving-linux-aarch64.tar.gz"),
        ("windows", "x86_64") => Some("Diving-windows-x86_64.zip"),
        ("windows", "aarch64") => Some("Diving-windows-aarch64.zip"),
        _ => None,
    }
}

/// Returns the canonical GitHub release asset download URL for `tag` and `asset`.
pub fn release_download_url(tag: &str, asset: &str) -> String {
    format!("{RELEASES_BASE_URL}/{tag}/{asset}")
}

/// Returns `true` when background auto-update checks should stay silent (tests, CI, headless).
pub fn auto_check_disabled_by_env() -> bool {
    if cfg!(test) {
        return true;
    }
    let exe = env::current_exe().ok();
    let args: Vec<String> = env::args().collect();
    should_skip_auto_check(
        exe.as_deref(),
        &args,
        env_truthy("DIVING_SKIP_UPDATE_CHECK"),
        env_truthy("CI"),
        env::var_os("RUST_TEST_THREADS").is_some(),
        env::var_os("NEXTEST").is_some(),
    )
}

fn should_skip_auto_check(
    exe: Option<&Path>,
    args: &[String],
    skip_var: bool,
    ci_var: bool,
    has_test_threads: bool,
    has_nextest: bool,
) -> bool {
    if skip_var || ci_var || has_test_threads || has_nextest {
        return true;
    }
    if let Some(exe) = exe {
        let in_cargo_deps = exe.components().any(|part| part.as_os_str() == "deps");
        let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let in_cargo_build_out = exe.components().any(|part| part.as_os_str() == "build")
            && exe
                .parent()
                .and_then(|parent| parent.file_name())
                .is_some_and(|name| name == "out");
        let has_cargo_hash = stem.rsplit_once('-').is_some_and(|(_, hash)| {
            hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit())
        });
        let is_nsh_cli = stem.eq_ignore_ascii_case("nsh");
        let is_nsh_test_runner = stem.starts_with("nsh-app-") && stem.ends_with("-test");
        if in_cargo_deps
            || (in_cargo_build_out && has_cargo_hash)
            || is_nsh_cli
            || is_nsh_test_runner
        {
            return true;
        }
        let is_nsh_dev_host =
            stem.ends_with("-host") && (stem.starts_with("nsh-app-") || stem.starts_with("nsh-"));
        if is_nsh_dev_host
            && args.iter().any(|arg| {
                arg == "--data-dir"
                    || arg.starts_with("--data-dir=")
                    || arg == "--keep-window-size"
                    || arg == "--background"
            })
        {
            return true;
        }
    }
    false
}

fn env_truthy(name: &str) -> bool {
    let Some(val) = env::var_os(name) else {
        return false;
    };
    let text = val.to_string_lossy();
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Returns Diving's state directory (`~/.diving`), falling back to system temp when home is unset.
pub fn data_dir() -> PathBuf {
    home_dir().unwrap_or_else(env::temp_dir).join(".diving")
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join(STATE_FILE_NAME)
}

fn read_check_in(dir: &Path) -> CheckFile {
    fs::read_to_string(state_path(dir))
        .ok()
        .and_then(|raw| serde_json::from_str::<CheckFile>(&raw).ok())
        .unwrap_or_default()
}

fn write_check_in(dir: &Path, file: &CheckFile) {
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(payload) = serde_json::to_string_pretty(file) else {
        return;
    };
    let path = state_path(dir);
    let tmp = dir.join(format!("{STATE_FILE_NAME}.{}.tmp", process::id()));
    if fs::write(&tmp, payload).is_ok() {
        if fs::rename(&tmp, &path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

/// Returns whether the 48h background check cadence has elapsed.
pub fn auto_check_due(now: SystemTime) -> bool {
    auto_check_due_in(&data_dir(), now)
}

fn auto_check_due_in(dir: &Path, now: SystemTime) -> bool {
    let file = read_check_in(dir);
    if file.latest.is_some() && file.notes.is_none() {
        return true;
    }
    let checked = UNIX_EPOCH + Duration::from_secs(file.checked_unix);
    now.duration_since(checked)
        .is_ok_and(|age| age >= AUTO_CHECK_EVERY)
        || now.duration_since(checked).is_err()
}

/// Persists a completed check outcome into `~/.diving/update-check.json`.
pub fn record_outcome(now: SystemTime, outcome: &UpdateCheck) {
    record_outcome_in(&data_dir(), now, outcome);
}

fn record_outcome_in(dir: &Path, now: SystemTime, outcome: &UpdateCheck) {
    let mut file = read_check_in(dir);
    file.checked_unix = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match outcome {
        UpdateCheck::Newer { version, notes } => {
            file.latest = Some(version.clone());
            file.notes = Some(notes.trim().to_string());
        }
        UpdateCheck::UpToDate => {
            file.latest = None;
            file.notes = None;
        }
        UpdateCheck::Failed(_) => {}
    }
    write_check_in(dir, &file);
}

/// Records `version` as skipped in `~/.diving/update-check.json`.
pub fn skip_version(version: &str) {
    skip_version_in(&data_dir(), version);
}

fn skip_version_in(dir: &Path, version: &str) {
    let mut file = read_check_in(dir);
    file.ignored = Some(version.to_string());
    write_check_in(dir, &file);
}

/// Returns the persisted skipped version if it is still newer than `CURRENT_VERSION`.
pub fn skipped_version() -> Option<String> {
    skipped_version_in(&data_dir(), CURRENT_VERSION)
}

fn skipped_version_in(dir: &Path, current: &str) -> Option<String> {
    let marked = read_check_in(dir).ignored?;
    is_newer(&marked, current).then_some(marked)
}

/// Clears any skipped version marker in `~/.diving/update-check.json`.
pub fn clear_skipped_version() {
    clear_skipped_version_in(&data_dir());
}

fn clear_skipped_version_in(dir: &Path) {
    let mut file = read_check_in(dir);
    file.ignored = None;
    write_check_in(dir, &file);
}

/// Returns a persisted unskipped newer version waiting to be surfaced on launch.
pub fn nudge_version() -> Option<String> {
    nudge_version_in(&data_dir(), CURRENT_VERSION)
}

fn nudge_version_in(dir: &Path, current: &str) -> Option<String> {
    let file = read_check_in(dir);
    let latest = file.latest?;
    if file.ignored.as_deref() == Some(latest.as_str()) {
        return None;
    }
    is_newer(&latest, current).then_some(latest)
}

/// Returns persisted release notes matching [`nudge_version`], if non-empty.
pub fn nudge_notes() -> Option<String> {
    nudge_notes_in(&data_dir(), CURRENT_VERSION)
}

fn nudge_notes_in(dir: &Path, current: &str) -> Option<String> {
    let file = read_check_in(dir);
    let latest = file.latest?;
    if file.ignored.as_deref() == Some(latest.as_str()) || !is_newer(&latest, current) {
        return None;
    }
    file.notes.filter(|n| !n.trim().is_empty())
}

/// Compares `remote_tag` (e.g. `"v0.12.4"`) against `current` (e.g. `"0.12.3"`).
///
/// Numeric core segments (`major.minor.patch`) take precedence; for equal cores,
/// stable versions beat pre-release versions (`0.12.4` > `0.12.4-rc.1`).
pub fn is_newer(remote_tag: &str, current: &str) -> bool {
    let Some(remote) = ParsedSemver::parse(remote_tag) else {
        return false;
    };
    let Some(local) = ParsedSemver::parse(current) else {
        return false;
    };
    remote > local
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedSemver {
    core: [u64; 3],
    pre: Vec<PreIdent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PreIdent {
    Num(u64),
    Text(String),
}

impl Ord for PreIdent {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        match (self, other) {
            (Self::Num(a), Self::Num(b)) => a.cmp(b),
            (Self::Num(_), Self::Text(_)) => CmpOrdering::Less,
            (Self::Text(_), Self::Num(_)) => CmpOrdering::Greater,
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PreIdent {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for ParsedSemver {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        match self.core.cmp(&other.core) {
            CmpOrdering::Equal => match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => CmpOrdering::Equal,
                (true, false) => CmpOrdering::Greater,
                (false, true) => CmpOrdering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            },
            ord => ord,
        }
    }
}

impl PartialOrd for ParsedSemver {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl ParsedSemver {
    fn parse(raw: &str) -> Option<Self> {
        let cleaned = raw.trim().trim_start_matches(['v', 'V']);
        let without_build = cleaned.split('+').next()?;
        let (core_raw, pre_raw) = match without_build.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (without_build, None),
        };
        let mut parts = core_raw.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
        let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre_raw {
            Some(s) if !s.trim().is_empty() => s
                .split('.')
                .map(|part| match part.parse::<u64>() {
                    Ok(n) => PreIdent::Num(n),
                    Err(_) => PreIdent::Text(part.to_owned()),
                })
                .collect(),
            _ => Vec::new(),
        };
        Some(Self {
            core: [major, minor, patch],
            pre,
        })
    }
}

/// Checks GitHub Releases (latest + intermediate release notes) on a blocking runtime.
pub fn check_latest_blocking() -> UpdateCheck {
    let rt = match Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(err) => return UpdateCheck::Failed(err.to_string()),
    };
    rt.block_on(check_from_endpoints(
        RELEASE_API_URL,
        Some(RELEASE_LIST_URL),
        CURRENT_VERSION,
    ))
}

async fn check_from_endpoints(
    latest_url: &str,
    list_url: Option<&str>,
    current: &str,
) -> UpdateCheck {
    let client = match build_update_client(REQUEST_TIMEOUT) {
        Ok(client) => client,
        Err(err) => return UpdateCheck::Failed(err),
    };
    let latest_text =
        match fetch_text(&client, latest_url, Some("application/vnd.github+json")).await {
            Ok(body) => body,
            Err(err) => return UpdateCheck::Failed(err),
        };
    let release: GithubRelease = match serde_json::from_str(&latest_text) {
        Ok(rel) => rel,
        Err(err) => return UpdateCheck::Failed(format!("invalid release JSON: {err}")),
    };
    let tag = release.tag_name.trim().to_owned();
    let latest_notes = normalize_newlines(release.body.as_deref().unwrap_or_default());
    if !is_newer(&tag, current) {
        return UpdateCheck::UpToDate;
    }

    let notes = if let Some(url) = list_url {
        let paginated = format!("{url}?per_page={HISTORY_PAGE_SIZE}");
        match fetch_text(&client, &paginated, Some("application/vnd.github+json")).await {
            Ok(list_json) => stitch_release_notes(&list_json, current, &tag, &latest_notes),
            Err(err) => {
                log::warn!("release list unavailable ({err}); using latest notes only");
                latest_notes
            }
        }
    } else {
        latest_notes
    };

    UpdateCheck::Newer {
        version: tag,
        notes,
    }
}

fn normalize_newlines(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

fn stitch_release_notes(
    list_json: &str,
    current: &str,
    latest: &str,
    latest_notes: &str,
) -> String {
    let Ok(releases) = serde_json::from_str::<Vec<GithubRelease>>(list_json) else {
        return latest_notes.to_owned();
    };
    let mut intermediate: Vec<(String, String)> = releases
        .into_iter()
        .filter_map(|rel| {
            let tag = rel.tag_name.trim().to_owned();
            let notes = normalize_newlines(rel.body.as_deref().unwrap_or_default());
            (is_newer(&tag, current) && is_newer(latest, &tag) && !notes.trim().is_empty())
                .then_some((tag, notes))
        })
        .collect();
    if intermediate.is_empty() {
        return latest_notes.to_owned();
    }
    intermediate.sort_by(|(a, _), (b, _)| is_newer(b, a).cmp(&is_newer(a, b)));
    let mut seen = HashSet::new();
    intermediate.retain(|(tag, _)| seen.insert(tag.clone()));

    let headed = |tag: &str, body: &str| {
        let body = body.trim();
        let bare = tag.trim_start_matches(['v', 'V']);
        let starts_with_version = body
            .lines()
            .next()
            .is_some_and(|line| line.starts_with('#') && line.contains(bare));
        if starts_with_version {
            body.to_owned()
        } else {
            format!("## {tag}\n\n{body}")
        }
    };

    let mut sections = Vec::with_capacity(intermediate.len() + 1);
    if !latest_notes.trim().is_empty() {
        sections.push(headed(latest, latest_notes));
    }
    for (tag, body) in &intermediate {
        sections.push(headed(tag, body));
    }
    sections.join("\n\n")
}

fn build_update_client(timeout: Duration) -> Result<Client, String> {
    install_crypto_provider();
    let mut builder = Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(REQUEST_TIMEOUT)
        .timeout(timeout);
    if let Some(proxy_url) = discover_proxy_url()
        && let Ok(proxy) = Proxy::all(&proxy_url)
    {
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|err| format!("failed to create updater HTTP client: {err}"))
}

fn discover_proxy_url() -> Option<String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(val) = env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(proxy) = read_macos_scutil_proxy() {
            return Some(proxy);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn read_macos_scutil_proxy() -> Option<String> {
    let out = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_scutil_proxy(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(any(target_os = "macos", test))]
fn parse_scutil_proxy(output: &str) -> Option<String> {
    let find_val = |key: &str| -> Option<String> {
        output.lines().find_map(|line| {
            let (lhs, rhs) = line.split_once(':')?;
            (lhs.trim() == key).then(|| rhs.trim().to_owned())
        })
    };
    if find_val("HTTPSEnable").as_deref() == Some("1")
        && let (Some(host), Some(port)) = (find_val("HTTPSProxy"), find_val("HTTPSPort"))
    {
        return Some(format!("http://{host}:{port}"));
    }
    if find_val("HTTPEnable").as_deref() == Some("1")
        && let (Some(host), Some(port)) = (find_val("HTTPProxy"), find_val("HTTPPort"))
    {
        return Some(format!("http://{host}:{port}"));
    }
    None
}

async fn fetch_text(client: &Client, url: &str, accept: Option<&str>) -> Result<String, String> {
    let mut req = client.get(url);
    if let Some(mime) = accept {
        req = req.header("Accept", mime);
    }
    let resp = req.send().await.map_err(|err| err.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    resp.text().await.map_err(|err| err.to_string())
}

/// Downloads `tag`'s release asset for this platform and verifies its SHA-256 against
/// `SHA256SUMS.txt` (falling back to `SHA256SUMS`).
pub fn download_and_verify_blocking(
    tag: &str,
    on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
    let Some(asset) = asset_name() else {
        return Err(format!(
            "no release build for {}/{}",
            env::consts::OS,
            env::consts::ARCH
        ));
    };
    let asset_url = release_download_url(tag, asset);
    let checksum_urls: Vec<String> = CHECKSUM_ASSETS
        .iter()
        .map(|name| release_download_url(tag, name))
        .collect();
    let checksum_refs: Vec<&str> = checksum_urls.iter().map(String::as_str).collect();
    download_and_verify_from_urls(
        &asset_url,
        &checksum_refs,
        asset,
        &env::temp_dir(),
        on_progress,
    )
}

fn download_and_verify_from_urls(
    asset_url: &str,
    checksum_urls: &[&str],
    asset: &str,
    dest_dir: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
    let rt = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    rt.block_on(async {
        let client = build_update_client(DOWNLOAD_TIMEOUT)?;
        let safe_name = asset.replace(['/', '\\'], "_");
        let dest = dest_dir.join(format!("diving-dl-{}-{safe_name}", process::id()));
        download_with_retries(&client, asset_url, &dest, &mut on_progress).await?;

        let checksum_text = fetch_checksum_manifest(&client, checksum_urls).await;
        if let Err(err) = verify_downloaded_asset(&dest, asset, checksum_text.as_deref()) {
            let _ = fs::remove_file(&dest);
            return Err(err);
        }
        Ok(dest)
    })
}

async fn download_with_retries(
    client: &Client,
    url: &str,
    dest: &Path,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<(), String> {
    let mut last_err = String::new();
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        match stream_to_file(client, url, dest, on_progress).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                let _ = fs::remove_file(dest);
                last_err = err;
                if attempt < DOWNLOAD_ATTEMPTS {
                    sleep(Duration::from_millis(400 * attempt as u64)).await;
                }
            }
        }
    }
    Err(last_err)
}

async fn stream_to_file(
    client: &Client,
    url: &str,
    dest: &Path,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let total = resp.content_length().unwrap_or(0);
    if total > MAX_DOWNLOAD_BYTES {
        return Err("download exceeded maximum allowed size".into());
    }
    let tmp = dest.with_extension("part");
    let mut file = File::create(&tmp).map_err(|err| err.to_string())?;
    let mut received: u64 = 0;
    on_progress(0, total);
    while let Some(chunk) = resp.chunk().await.map_err(|err| err.to_string())? {
        received = received.saturating_add(chunk.len() as u64);
        if received > MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = fs::remove_file(&tmp);
            return Err("download exceeded maximum allowed size".into());
        }
        file.write_all(&chunk).map_err(|err| err.to_string())?;
        on_progress(received, total);
    }
    file.flush().map_err(|err| err.to_string())?;
    drop(file);
    if total > 0 && received < total {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "truncated download: received {received} of {total} bytes"
        ));
    }
    fs::rename(&tmp, dest).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        err.to_string()
    })
}

async fn fetch_checksum_manifest(client: &Client, urls: &[&str]) -> Option<String> {
    for url in urls {
        if let Ok(text) = fetch_text(client, url, None).await {
            return Some(text);
        }
    }
    None
}

/// Verifies `path`'s SHA-256 against an entry matching `asset` inside `checksums_text`.
pub fn verify_downloaded_asset(
    path: &Path,
    asset: &str,
    checksums_text: Option<&str>,
) -> Result<(), String> {
    let Some(text) = checksums_text else {
        return Err("release has no SHA256SUMS manifest; refusing unverified download".into());
    };
    let Some(expected) = parse_checksum_line(text, asset) else {
        return Err(format!(
            "checksum manifest has no SHA-256 entry for {asset}; refusing install"
        ));
    };
    let Some(actual) = file_sha256(path) else {
        return Err("could not compute SHA-256 of downloaded update".into());
    };
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(format!(
            "checksum mismatch: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

/// Parses a `sha256sum` line (`<64-hex>  [*|./]<asset>`) for `asset`.
pub fn parse_checksum_line(text: &str, asset: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let (hash, rest) = trimmed.split_once(char::is_whitespace)?;
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let name = rest.trim_start();
        let name = name.strip_prefix('*').unwrap_or(name);
        let name = name.strip_prefix("./").unwrap_or(name);
        (name == asset).then(|| hash.to_ascii_lowercase())
    })
}

fn file_sha256(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut ctx = Sha256Context::new(&SHA256);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    let digest = ctx.finish();
    let mut out = String::with_capacity(64);
    for byte in digest.as_ref() {
        out.push_str(&format!("{byte:02x}"));
    }
    Some(out)
}

// ---- Platform installation, sweep, and relaunch -------------------------

/// Installs a verified release archive (`dmg`, `tar.gz`, or `zip`) in place.
#[cfg(target_os = "macos")]
pub fn install(dmg: &Path) -> Result<InstallOutcome, String> {
    match running_bundle() {
        Some(target) if is_writable_bundle_dest(&target) => match install_over(&target, dmg) {
            Ok(()) => return Ok(InstallOutcome::Relaunch { target }),
            Err(err) => {
                log::warn!("in-place bundle swap failed, opening DMG in Finder: {err}");
            }
        },
        Some(target) => {
            log::info!(
                "running bundle at {} is not directly writable; opening DMG in Finder",
                target.display()
            );
        }
        None => {
            log::info!("no enclosing .app bundle (bare binary); opening DMG in Finder");
        }
    }
    open_dmg_in_finder(dmg)
}

#[cfg(target_os = "macos")]
fn open_dmg_in_finder(dmg: &Path) -> Result<InstallOutcome, String> {
    let status = Command::new("/usr/bin/open")
        .arg(dmg.as_os_str())
        .status()
        .map_err(|err| err.to_string())?;
    if !status.success() {
        return Err("open refused to mount the DMG".into());
    }
    let _ = Command::new("/usr/bin/open")
        .args(["-a", "Finder"])
        .status();
    Ok(InstallOutcome::OpenedInstaller {
        path: dmg.to_path_buf(),
    })
}

#[cfg(target_os = "macos")]
fn running_bundle() -> Option<PathBuf> {
    bundle_root_of(&env::current_exe().ok()?)
}

#[cfg(any(target_os = "macos", test))]
fn bundle_root_of(exe: &Path) -> Option<PathBuf> {
    let root = exe.parent()?.parent()?.parent()?;
    (root.extension().is_some_and(|ext| ext == "app")).then(|| root.to_path_buf())
}

#[cfg(any(target_os = "macos", test))]
fn is_writable_bundle_dest(bundle: &Path) -> bool {
    if bundle.starts_with("/Volumes") {
        return false;
    }
    let Some(parent) = bundle.parent() else {
        return false;
    };
    let probe = parent.join(format!(".diving-write-probe-{}", process::id()));
    match File::create(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
struct DetachGuard {
    volume: PathBuf,
    armed: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
impl Drop for DetachGuard {
    fn drop(&mut self) {
        if self.armed.load(Ordering::Acquire) {
            detach(&self.volume);
        }
    }
}

#[cfg(target_os = "macos")]
fn install_over(target: &Path, dmg: &Path) -> Result<(), String> {
    let volume = attach(dmg)?;
    let _guard = DetachGuard {
        volume: volume.clone(),
        armed: Arc::new(AtomicBool::new(true)),
    };
    let result = replace_bundle(target, &volume);
    if result.is_ok() {
        let _ = fs::remove_file(dmg);
    }
    result
}

#[cfg(target_os = "macos")]
fn attach(dmg: &Path) -> Result<PathBuf, String> {
    let out = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-noverify", "-readonly", "-plist"])
        .arg(dmg.as_os_str())
        .output()
        .map_err(|err| err.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "hdiutil attach: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    mount_point_from_plist(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "no mount point in hdiutil output".into())
}

#[cfg(any(target_os = "macos", test))]
fn mount_point_from_plist(xml: &str) -> Option<PathBuf> {
    let after = xml.split("<key>mount-point</key>").nth(1)?;
    let start = after.find("<string>")? + "<string>".len();
    let end = start + after[start..].find("</string>")?;
    Some(PathBuf::from(&after[start..end]))
}

#[cfg(target_os = "macos")]
fn detach(volume: &Path) {
    for attempt in 0..2 {
        if attempt > 0 {
            thread::sleep(Duration::from_secs(1));
        }
        let ok = Command::new("hdiutil")
            .args(["detach", "-force"])
            .arg(volume.as_os_str())
            .output()
            .is_ok_and(|out| out.status.success());
        if ok {
            return;
        }
    }
    log::warn!(
        "installer image {} left mounted; startup sweep will retry",
        volume.display()
    );
}

#[cfg(target_os = "macos")]
fn replace_bundle(target: &Path, volume: &Path) -> Result<(), String> {
    let fresh = volume.join(APP_BUNDLE_NAME);
    if !fresh.exists() {
        return Err(format!("mounted DMG does not contain {APP_BUNDLE_NAME}"));
    }
    if bundle_plist_value(&fresh, "CFBundleIdentifier").as_deref() != Some(BUNDLE_ID) {
        return Err("mounted DMG bundle identifier mismatch".into());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let aside = env::temp_dir().join(format!(
        "{PREVIOUS_BUNDLE_PREFIX}{}-{stamp}.app",
        process::id()
    ));
    fs::rename(target, &aside).map_err(|err| format!("could not move old bundle aside: {err}"))?;
    let out = Command::new("/usr/bin/ditto")
        .arg(fresh.as_os_str())
        .arg(target.as_os_str())
        .output();
    let err_msg = match &out {
        Ok(res) if res.status.success() => {
            log::info!("installed updated bundle at {}", target.display());
            return Ok(());
        }
        Ok(res) => String::from_utf8_lossy(&res.stderr).trim().to_owned(),
        Err(err) => err.to_string(),
    };
    if let Err(restore_err) = fs::rename(&aside, target) {
        log::error!(
            "failed to restore old bundle from {}: {restore_err}",
            aside.display()
        );
    }
    Err(format!("ditto failed: {err_msg}"))
}

#[cfg(any(target_os = "macos", test))]
fn parse_xml_plist_value(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let after = xml.split(&needle).nth(1)?;
    let start = after.find("<string>")? + "<string>".len();
    let end = start + after[start..].find("</string>")?;
    let val = after[start..end].trim();
    (!val.is_empty()).then(|| val.to_owned())
}

#[cfg(target_os = "macos")]
fn bundle_plist_value(app: &Path, key: &str) -> Option<String> {
    let plist_path = app.join("Contents/Info.plist");
    if let Ok(xml) = fs::read_to_string(&plist_path)
        && let Some(val) = parse_xml_plist_value(&xml, key)
    {
        return Some(val);
    }
    let out = Command::new("defaults")
        .arg("read")
        .arg(app.join("Contents/Info").as_os_str())
        .arg(key)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Spawns a detached shell waiting for this process to exit before reopening `Diving.app`.
#[cfg(target_os = "macos")]
pub fn relaunch(target: Option<&Path>) {
    let bundle = target.map(Path::to_path_buf).or_else(running_bundle);
    let Some(bundle) = bundle else {
        return;
    };
    let script = format!(
        "while /bin/kill -0 {pid} 2>/dev/null; do /bin/sleep 0.1; done; /usr/bin/open \"$0\"",
        pid = process::id()
    );
    if let Err(err) = Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(bundle.as_os_str())
        .spawn()
    {
        log::warn!("could not spawn macOS relauncher: {err}");
    }
}

/// Sweeps stale temporary previous bundles and obsolete mounted Diving Installer DMGs.
#[cfg(target_os = "macos")]
pub fn sweep_installer_mounts() {
    sweep_previous_bundles_in(&env::temp_dir(), process::id());
    let running = env::current_exe().ok();
    let Ok(volumes) = fs::read_dir("/Volumes") else {
        return;
    };
    for entry in volumes.flatten() {
        let name = entry.file_name();
        if !name
            .to_str()
            .is_some_and(|s| s.starts_with(INSTALLER_VOLUME_PREFIX))
        {
            continue;
        }
        let volume = entry.path();
        let app = volume.join(APP_BUNDLE_NAME);
        if bundle_plist_value(&app, "CFBundleIdentifier").as_deref() != Some(BUNDLE_ID) {
            continue;
        }
        if running.as_ref().is_some_and(|exe| exe.starts_with(&volume)) {
            continue;
        }
        let mounted_ver =
            bundle_plist_value(&app, "CFBundleShortVersionString").unwrap_or_default();
        if is_newer(&mounted_ver, CURRENT_VERSION) {
            continue;
        }
        let detached = Command::new("hdiutil")
            .args(["detach", "-force"])
            .arg(volume.as_os_str())
            .output()
            .is_ok_and(|out| out.status.success());
        if detached {
            let _ = Command::new(LSREGISTER_PATH)
                .args(["-u"])
                .arg(app.as_os_str())
                .output();
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn sweep_previous_bundles_in(temp_dir: &Path, current_pid: u32) {
    let Ok(entries) = fs::read_dir(temp_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Some(mid) = name_str
            .strip_prefix("diving-previous-")
            .and_then(|s| s.strip_suffix(".app"))
        else {
            continue;
        };
        let Some((pid_str, _)) = mid.split_once('-') else {
            continue;
        };
        let Ok(pid) = pid_str.parse::<u32>() else {
            continue;
        };
        if pid == current_pid {
            continue;
        }
        let _ = fs::remove_dir_all(entry.path());
    }
}

#[cfg(target_os = "linux")]
pub fn install(tarball: &Path) -> Result<InstallOutcome, String> {
    let target = env::current_exe().map_err(|err| format!("current_exe: {err}"))?;
    let target = without_deleted_suffix(&target);
    install_linux_into(tarball, &target)?;
    Ok(InstallOutcome::Relaunch { target })
}

#[cfg(any(target_os = "linux", test))]
fn install_linux_into(tarball: &Path, target: &Path) -> Result<(), String> {
    let unpack = env::temp_dir().join(format!("diving-update-{}", process::id()));
    let _ = fs::remove_dir_all(&unpack);
    fs::create_dir_all(&unpack).map_err(|err| err.to_string())?;
    let res = unpack_tar_gz_and_replace(tarball, &unpack, target);
    let _ = fs::remove_dir_all(&unpack);
    if res.is_ok() {
        let _ = fs::remove_file(tarball);
    }
    res
}

#[cfg(any(target_os = "linux", test))]
fn unpack_tar_gz_and_replace(tarball: &Path, unpack: &Path, target: &Path) -> Result<(), String> {
    let file = File::open(tarball).map_err(|err| err.to_string())?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive
        .unpack(unpack)
        .map_err(|err| format!("tar unpack failed: {err}"))?;
    let exe_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty());
    let fresh = find_unpacked_binary(unpack, exe_name, &["Diving", "diving", "nsh-app-diving"])
        .ok_or_else(|| "release archive does not contain an executable binary".to_owned())?;
    replace_unix_binary(target, &fresh)
}

#[cfg(any(target_os = "linux", test))]
fn replace_unix_binary(target: &Path, fresh: &Path) -> Result<(), String> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("running binary has no file name")?;
    let staged = target.with_file_name(format!("{name}.new"));
    fs::copy(fresh, &staged).map_err(|err| format!("failed to stage {name}.new: {err}"))?;
    #[cfg(unix)]
    if let Err(err) = fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)) {
        let _ = fs::remove_file(&staged);
        return Err(format!("chmod failed: {err}"));
    }
    if let Err(err) = fs::rename(&staged, target) {
        let _ = fs::remove_file(&staged);
        return Err(format!("failed to replace {}: {err}", target.display()));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn without_deleted_suffix(exe: &Path) -> PathBuf {
    match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(clean) => PathBuf::from(clean),
        None => exe.to_path_buf(),
    }
}

#[cfg(target_os = "linux")]
pub fn relaunch(target: Option<&Path>) {
    let resolved = target
        .map(Path::to_path_buf)
        .or_else(|| env::current_exe().ok().map(|p| without_deleted_suffix(&p)));
    let Some(exe) = resolved else {
        return;
    };
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.1; done; exec \"$0\"",
        pid = process::id()
    );
    if let Err(err) = Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(exe.as_os_str())
        .spawn()
    {
        log::warn!("could not spawn Linux relauncher: {err}");
    }
}

#[cfg(target_os = "linux")]
pub fn sweep_installer_mounts() {
    let Ok(entries) = fs::read_dir(env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(UNPACK_PREFIX))
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(target_os = "windows")]
pub fn install(archive: &Path) -> Result<InstallOutcome, String> {
    let target = env::current_exe().map_err(|err| format!("current_exe: {err}"))?;
    install_windows_into(archive, &target)?;
    Ok(InstallOutcome::Relaunch { target })
}

#[cfg(any(target_os = "windows", test))]
fn install_windows_into(archive: &Path, target: &Path) -> Result<(), String> {
    let unpack = env::temp_dir().join(format!("diving-update-{}", process::id()));
    let _ = fs::remove_dir_all(&unpack);
    fs::create_dir_all(&unpack).map_err(|err| err.to_string())?;
    let res = unpack_zip_and_replace(archive, &unpack, target);
    let _ = fs::remove_dir_all(&unpack);
    if res.is_ok() {
        let _ = fs::remove_file(archive);
    }
    res
}

#[cfg(any(target_os = "windows", test))]
fn unpack_zip_and_replace(archive_path: &Path, unpack: &Path, target: &Path) -> Result<(), String> {
    let file = File::open(archive_path).map_err(|err| err.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|err| format!("invalid zip archive: {err}"))?;
    for idx in 0..zip.len() {
        let mut entry = zip.by_index(idx).map_err(|err| err.to_string())?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out_path = unpack.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|err| err.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
            let mut out = File::create(&out_path).map_err(|err| err.to_string())?;
            io::copy(&mut entry, &mut out).map_err(|err| err.to_string())?;
        }
    }

    let exe_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty());
    let fresh = find_unpacked_binary(
        unpack,
        exe_name,
        &["Diving.exe", "diving.exe", "nsh-app-diving.exe"],
    )
    .ok_or_else(|| "release archive does not contain Diving.exe".to_owned())?;

    let old = target.with_extension("exe.old");
    let _ = fs::remove_file(&old);
    if target.exists() {
        fs::rename(target, &old)
            .map_err(|err| format!("could not move running executable aside: {err}"))?;
    }
    if let Err(err) = fs::copy(&fresh, target) {
        if old.exists() {
            let _ = fs::rename(&old, target);
        }
        return Err(format!("could not write updated executable: {err}"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn relaunch(target: Option<&Path>) {
    let resolved = target
        .map(Path::to_path_buf)
        .or_else(|| env::current_exe().ok());
    let Some(exe) = resolved else {
        return;
    };
    let cmd = format!(
        "ping 127.0.0.1 -n 2 > nul & start \"\" \"{}\"",
        exe.display()
    );
    let _ = Command::new("cmd").args(["/C", &cmd]).spawn();
}

#[cfg(target_os = "windows")]
pub fn sweep_installer_mounts() {
    if let Ok(exe) = env::current_exe() {
        let _ = fs::remove_file(exe.with_extension("exe.old"));
    }
    let Ok(entries) = fs::read_dir(env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(UNPACK_PREFIX))
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn install(_asset: &Path) -> Result<InstallOutcome, String> {
    Err("in-place install is not implemented on this platform".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn relaunch(_target: Option<&Path>) {}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn sweep_installer_mounts() {}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn find_unpacked_binary(
    unpack: &Path,
    preferred_name: Option<&str>,
    fallback_names: &[&str],
) -> Option<PathBuf> {
    let mut names: Vec<&str> = Vec::new();
    if let Some(pref) = preferred_name {
        names.push(pref);
    }
    for &name in fallback_names {
        if !names.contains(&name) {
            names.push(name);
        }
    }

    for name in &names {
        let direct = unpack.join(name);
        if direct.is_file() {
            return Some(direct);
        }
    }

    if let Ok(entries) = fs::read_dir(unpack) {
        let subdirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        for name in &names {
            for dir in &subdirs {
                let nested = dir.join(name);
                if nested.is_file() {
                    return Some(nested);
                }
                let bin_nested = dir.join("bin").join(name);
                if bin_nested.is_file() {
                    return Some(bin_nested);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::{Builder as TarBuilder, Header as TarHeader};
    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    fn sha256_hex(data: &[u8]) -> String {
        let mut ctx = Sha256Context::new(&SHA256);
        ctx.update(data);
        let digest = ctx.finish();
        let mut out = String::with_capacity(64);
        for b in digest.as_ref() {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    fn spawn_http_server(
        routes: Vec<(String, String, Vec<u8>)>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock HTTP port");
        let addr = listener.local_addr().expect("mock local_addr");
        let handle = thread::spawn(move || {
            for stream in listener.incoming().take(routes.len() + 4) {
                let Ok(mut stream) = stream else {
                    break;
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut req_line = String::new();
                if reader.read_line(&mut req_line).is_err() {
                    break;
                }
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                        break;
                    }
                }
                let target = req_line.split_whitespace().nth(1).unwrap_or("/");
                if let Some((_, status, body)) = routes.iter().find(|(p, _, _)| p == target) {
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.write_all(body);
                } else {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn asset_matrix_matches_publish_workflow() {
        assert_eq!(
            asset_name_for("macos", "aarch64"),
            Some("Diving-macos-universal.dmg")
        );
        assert_eq!(
            asset_name_for("macos", "x86_64"),
            Some("Diving-macos-universal.dmg")
        );
        assert_eq!(
            asset_name_for("linux", "x86_64"),
            Some("Diving-linux-x86_64.tar.gz")
        );
        assert_eq!(
            asset_name_for("linux", "aarch64"),
            Some("Diving-linux-aarch64.tar.gz")
        );
        assert_eq!(
            asset_name_for("windows", "x86_64"),
            Some("Diving-windows-x86_64.zip")
        );
        assert_eq!(
            asset_name_for("windows", "aarch64"),
            Some("Diving-windows-aarch64.zip")
        );
        assert_eq!(asset_name_for("freebsd", "x86_64"), None);
    }

    #[test]
    fn semver_comparison_handles_tags_and_prereleases() {
        assert!(is_newer("v0.12.4", "0.12.3"));
        assert!(is_newer("0.13.0", "0.12.9"));
        assert!(is_newer("v0.12.4", "v0.12.4-rc.1"));
        assert!(!is_newer("v0.12.3", "0.12.3"));
        assert!(!is_newer("v0.12.2", "0.12.3"));
        assert!(!is_newer("v0.12.3-rc.1", "0.12.3"));
    }

    #[test]
    fn checksum_manifest_parses_star_and_dot_slash_formats() {
        let sample_hash = "a".repeat(64);
        let manifest = format!(
            "{sample_hash}  ./Diving-macos-universal.dmg\n\
             {} *Diving-linux-x86_64.tar.gz\n",
            "B".repeat(64)
        );
        assert_eq!(
            parse_checksum_line(&manifest, "Diving-macos-universal.dmg"),
            Some(sample_hash)
        );
        assert_eq!(
            parse_checksum_line(&manifest, "Diving-linux-x86_64.tar.gz"),
            Some("b".repeat(64))
        );
        assert_eq!(parse_checksum_line(&manifest, "missing.zip"), None);
    }

    #[test]
    fn json_persistence_enforces_48h_cadence_and_skip_version() {
        let dir = TempDir::new().expect("temp dir");
        let t0 = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert!(auto_check_due_in(dir.path(), t0));

        record_outcome_in(
            dir.path(),
            t0,
            &UpdateCheck::Newer {
                version: "v0.12.4".into(),
                notes: "Bug fixes".into(),
            },
        );
        assert!(!auto_check_due_in(
            dir.path(),
            t0 + Duration::from_secs(3600)
        ));
        assert!(auto_check_due_in(dir.path(), t0 + AUTO_CHECK_EVERY));
        assert_eq!(
            nudge_version_in(dir.path(), "0.12.3").as_deref(),
            Some("v0.12.4")
        );
        assert_eq!(
            nudge_notes_in(dir.path(), "0.12.3").as_deref(),
            Some("Bug fixes")
        );

        skip_version_in(dir.path(), "v0.12.4");
        assert_eq!(nudge_version_in(dir.path(), "0.12.3"), None);
        assert_eq!(
            skipped_version_in(dir.path(), "0.12.3").as_deref(),
            Some("v0.12.4")
        );

        clear_skipped_version_in(dir.path());
        assert_eq!(
            nudge_version_in(dir.path(), "0.12.3").as_deref(),
            Some("v0.12.4")
        );
        assert_eq!(skipped_version_in(dir.path(), "0.12.4"), None);
    }

    #[test]
    fn auto_check_stays_disabled_in_test_env() {
        assert!(auto_check_disabled_by_env());
        assert!(should_skip_auto_check(
            Some(Path::new(
                "/Users/tree/cargo-target/debug/build/nsh-app-diving/e6fdf545f8337c88/out/inspector_e2e-e6fdf545f8337c88"
            )),
            &[],
            false,
            false,
            false,
            false,
        ));
        assert!(!should_skip_auto_check(
            Some(Path::new("/Users/tree/cargo-target/debug/nsh-app-diving")),
            &[],
            false,
            false,
            false,
            false,
        ));
        assert!(should_skip_auto_check(
            Some(Path::new("/tmp/target/nsh-app-nsh-app-diving-test")),
            &[],
            false,
            false,
            false,
            false,
        ));
        assert!(should_skip_auto_check(
            Some(Path::new("/tmp/target/nsh-app-nsh-app-diving-macos-host")),
            &[
                "host".into(),
                "--dev-server".into(),
                "ws://127.0.0.1:8790/ws".into(),
                "--data-dir".into(),
                "/tmp/nsh-test-data".into(),
                "--keep-window-size".into(),
                "--background".into(),
            ],
            false,
            false,
            false,
            false,
        ));
        assert!(should_skip_auto_check(
            Some(Path::new("/tmp/target/nsh-app-nsh-app-diving-macos-host")),
            &[
                "host".into(),
                "--dev-server".into(),
                "ws://127.0.0.1:8790/ws".into(),
                "--data-dir=/tmp/nsh-test-data".into(),
            ],
            false,
            false,
            false,
            false,
        ));
        assert!(!should_skip_auto_check(
            Some(Path::new("/tmp/target/nsh-app-nsh-app-diving-macos-host")),
            &[
                "host".into(),
                "--dev-server".into(),
                "ws://127.0.0.1:8790/ws".into(),
                "--app-id".into(),
                "io.github.vicanso.diving".into(),
                "--name".into(),
                "Diving".into(),
            ],
            false,
            false,
            false,
            false,
        ));
        assert!(!should_skip_auto_check(
            Some(Path::new("/Applications/Diving.app/Contents/MacOS/diving")),
            &["diving".into()],
            false,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn plist_and_proxy_parsers_extract_expected_fields() {
        let hdiutil_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>system-entities</key><array>
  <dict><key>mount-point</key><string>/Volumes/Diving Installer</string></dict>
</array></dict></plist>"#;
        assert_eq!(
            mount_point_from_plist(hdiutil_xml),
            Some(PathBuf::from("/Volumes/Diving Installer"))
        );

        let info_plist = format!(
            "<plist><dict><key>CFBundleIdentifier</key>\n  <string>{BUNDLE_ID}</string></dict></plist>"
        );
        assert_eq!(
            parse_xml_plist_value(&info_plist, "CFBundleIdentifier").as_deref(),
            Some(BUNDLE_ID)
        );
        assert_eq!(
            bundle_root_of(Path::new("/Applications/Diving.app/Contents/MacOS/Diving")),
            Some(PathBuf::from("/Applications/Diving.app"))
        );
        assert_eq!(bundle_root_of(Path::new("/usr/local/bin/diving")), None);

        let scutil = "  HTTPEnable : 1\n  HTTPProxy : 127.0.0.1\n  HTTPPort : 8888\n";
        assert_eq!(
            parse_scutil_proxy(scutil).as_deref(),
            Some("http://127.0.0.1:8888")
        );
    }

    #[test]
    fn mock_check_and_download_verify_sha256sums_fallback() {
        let payload = b"diving-binary-payload-v0.12.4".to_vec();
        let hash = sha256_hex(&payload);
        let sums = format!("{hash}  Diving-linux-x86_64.tar.gz\n");
        let latest_json = serde_json::json!({
            "tag_name": "v0.12.4",
            "body": "## Fixes\n- Auto updater"
        })
        .to_string();

        let (base, _srv) = spawn_http_server(vec![
            ("/latest".into(), "200 OK".into(), latest_json.into_bytes()),
            (
                "/Diving-linux-x86_64.tar.gz".into(),
                "200 OK".into(),
                payload.clone(),
            ),
            ("/SHA256SUMS".into(), "200 OK".into(), sums.into_bytes()),
        ]);

        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        let check = rt.block_on(check_from_endpoints(
            &format!("{base}/latest"),
            None,
            "0.12.3",
        ));
        assert_eq!(
            check,
            UpdateCheck::Newer {
                version: "v0.12.4".into(),
                notes: "## Fixes\n- Auto updater".into(),
            }
        );

        let dir = TempDir::new().expect("temp dir");
        let mut progress_events = Vec::new();
        let path = download_and_verify_from_urls(
            &format!("{base}/Diving-linux-x86_64.tar.gz"),
            &[
                &format!("{base}/SHA256SUMS.txt"),
                &format!("{base}/SHA256SUMS"),
            ],
            "Diving-linux-x86_64.tar.gz",
            dir.path(),
            |got, total| progress_events.push((got, total)),
        )
        .expect("verified download via SHA256SUMS fallback");

        assert_eq!(fs::read(&path).expect("read downloaded"), payload);
        assert!(!progress_events.is_empty());

        assert!(
            verify_downloaded_asset(
                &path,
                "Diving-linux-x86_64.tar.gz",
                Some("0".repeat(64).as_str())
            )
            .is_err()
        );
    }

    #[test]
    fn tar_gz_and_zip_install_helpers_swap_target_binary() {
        let dir = TempDir::new().expect("temp dir");
        let linux_target = dir.path().join("diving");
        fs::write(&linux_target, b"old-linux").expect("seed linux bin");

        let tar_gz_path = dir.path().join("Diving-linux-x86_64.tar.gz");
        {
            let tar_file = File::create(&tar_gz_path).expect("create tar.gz");
            let encoder = GzEncoder::new(tar_file, Compression::default());
            let mut builder = TarBuilder::new(encoder);
            let bin = b"new-linux-binary";
            let mut header = TarHeader::new_gnu();
            header
                .set_path("Diving-linux-x86_64/Diving")
                .expect("tar path");
            header.set_size(bin.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, &bin[..]).expect("append bin");
            builder
                .into_inner()
                .expect("finish encoder")
                .finish()
                .expect("flush gz");
        }
        install_linux_into(&tar_gz_path, &linux_target).expect("swap linux binary");
        assert_eq!(
            fs::read(&linux_target).expect("read swapped linux"),
            b"new-linux-binary"
        );

        let win_target = dir.path().join("Diving.exe");
        fs::write(&win_target, b"old-win").expect("seed win bin");
        let zip_path = dir.path().join("Diving-windows-x86_64.zip");
        {
            let zfile = File::create(&zip_path).expect("create zip");
            let mut zw = ZipWriter::new(zfile);
            zw.start_file("Diving/Diving.exe", SimpleFileOptions::default())
                .expect("start zip file");
            zw.write_all(b"new-win-binary").expect("write zip body");
            zw.finish().expect("finish zip");
        }
        install_windows_into(&zip_path, &win_target).expect("swap win binary");
        assert_eq!(
            fs::read(&win_target).expect("read swapped win"),
            b"new-win-binary"
        );
        assert!(win_target.with_extension("exe.old").exists());
    }

    #[test]
    fn sweep_previous_bundles_removes_stale_pid_dirs_only() {
        let dir = TempDir::new().expect("temp dir");
        let stale = dir.path().join("diving-previous-101-999.app");
        let own = dir
            .path()
            .join(format!("diving-previous-{}-999.app", process::id()));
        fs::create_dir_all(&stale).expect("mkdir stale");
        fs::create_dir_all(&own).expect("mkdir own");
        sweep_previous_bundles_in(dir.path(), process::id());
        assert!(!stale.exists());
        assert!(own.exists());
        assert_eq!(
            without_deleted_suffix(Path::new("/usr/local/bin/diving (deleted)")),
            PathBuf::from("/usr/local/bin/diving")
        );
        assert_eq!(
            without_deleted_suffix(Path::new("/usr/local/bin/diving")),
            PathBuf::from("/usr/local/bin/diving")
        );
    }
}
