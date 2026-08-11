//! Self-update for CEC Support.
//!
//! Modelled on `allmystuff-updater` so the two behave identically, but
//! deliberately **self-contained**: it links none of the node engine and lives
//! in this repo's light root workspace, so `cargo test` here still builds
//! without a webview or media stack. That's the same call `allmystuff-updater`
//! made when it was ported from `myownmesh-updater` — each app in the
//! ecosystem carries its own updater rather than depending on a sibling's
//! unreleased code.
//!
//! # Where this sits in the ecosystem
//!
//! CEC Support already keeps the *engine beneath it* current: on bring-up it
//! asks a reused, separately-installed `allmystuff-serve` to update itself to
//! the pin CEC was built against (`ALLMYSTUFF_PIN`), and AllMyStuff in turn
//! asks `myownmesh` to update itself to *its* pin — each layer bringing its own
//! bundled sidecar when the update can't be had. What was missing is the top of
//! that chain: nothing ever updated **CEC Support itself**. This crate is that
//! half, so the whole stack moves forward instead of only its lower two thirds.
//!
//! # Shape
//!
//! One verified artifact containing `cec-support` and its required runtime
//! companions (`amst.exe` and Crucible's verified portable payload on Windows).
//! A check fetches the release feed, compares tags, and *stages* that payload under the CEC home; the swap
//! happens on the next launch ([`apply_pending_if_any`]), because running
//! executables can't reliably replace themselves in place. Verification is
//! fail-closed: a published SHA-256 sidecar is mandatory, and when a release
//! signing key is baked in at build time a valid detached minisign signature is
//! required too.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod policy;
pub use policy::{compare_semver, policy_allows, ApplyPolicy};

// ---------------------------------------------------------------------------
// Release feed (build-time + runtime overridable, for white-labelling).
// ---------------------------------------------------------------------------

pub fn default_release_api_stable() -> &'static str {
    option_env!("CEC_RELEASE_URL_STABLE")
        .unwrap_or("https://api.github.com/repos/mrjeeves/CECSupport/releases/latest")
}

pub fn default_release_api_beta() -> &'static str {
    option_env!("CEC_RELEASE_URL_BETA")
        .unwrap_or("https://api.github.com/repos/mrjeeves/CECSupport/releases")
}

const USER_AGENT: &str = concat!("cec-support-self-update/", env!("CARGO_PKG_VERSION"));

/// The minisign public key releases are signed with, baked in at build time.
/// `None` until release signing is configured (set `CEC_RELEASE_PUBKEY` to the
/// base64 public key in the release build env). When configured, the updater
/// refuses any artifact lacking a valid signature; otherwise it still requires
/// the mandatory SHA-256.
fn release_pubkey() -> Option<&'static str> {
    normalize_pubkey(option_env!("CEC_RELEASE_PUBKEY"))
}

/// Recover the key line from whatever was pasted into the build variable.
///
/// CI exports the variable unconditionally, so an unset repo secret arrives as
/// `Some("")` rather than `None` — which must degrade to "unconfigured", never
/// to "require a signature verified against an empty key" (that would fail
/// every update closed against a `.minisig` nobody publishes). A whole
/// `minisign.pub` file pasted in keeps its comment on line 1 and the key on
/// line 2, so selection is positional.
fn normalize_pubkey(key: Option<&str>) -> Option<&str> {
    let mut lines = key?.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next()?;
    Some(lines.next().unwrap_or(first))
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("checksum mismatch for {asset}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        asset: String,
        expected: String,
        actual: String,
    },
}

impl Error {
    fn msg(s: impl Into<String>) -> Self {
        Error::Msg(s.into())
    }
}

// ---------------------------------------------------------------------------
// Paths.
// ---------------------------------------------------------------------------

/// CEC Support's app-file home. Must agree with the app's own
/// `default_cec_home()` / `CEC_HOME_ENV` — duplicated rather than imported so
/// this crate stays free of the node engine. The mesh stack deliberately lives
/// elsewhere (the shared `~/.myownmesh` home); this is only CEC's own files.
const HOME_ENV: &str = "CEC_SUPPORT_HOME";

fn home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os(HOME_ENV) {
        return Ok(PathBuf::from(h));
    }
    dirs::data_dir()
        .map(|d| d.join("CEC Support"))
        .or_else(|| dirs::home_dir().map(|h| h.join(".cec-support")))
        .ok_or_else(|| Error::msg("no home directory"))
}

fn updates_dir() -> Result<PathBuf> {
    let d = home()?.join("updates");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

fn config_path() -> Result<PathBuf> {
    Ok(home()?.join("config.json"))
}

// ---------------------------------------------------------------------------
// Auto-update config (persisted under config.json's "auto_update" key).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoUpdateConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default = "default_auto_apply")]
    pub auto_apply: String,
    #[serde(default = "default_interval")]
    pub check_interval_hours: u32,
    #[serde(default)]
    pub stable_url: Option<String>,
    #[serde(default)]
    pub beta_url: Option<String>,
}

fn default_true() -> bool {
    true
}
fn default_channel() -> String {
    "stable".into()
}
fn default_auto_apply() -> String {
    // "Up to minor": patch + minor bumps apply on their own, major waits.
    // CEC Support sits in front of a customer who is, by definition, not a
    // technician — the fewer update decisions handed to them, the better.
    "minor".into()
}
fn default_interval() -> u32 {
    24
}

impl Default for AutoUpdateConfig {
    fn default() -> Self {
        AutoUpdateConfig {
            enabled: true,
            channel: default_channel(),
            auto_apply: default_auto_apply(),
            check_interval_hours: default_interval(),
            stable_url: None,
            beta_url: None,
        }
    }
}

fn load_auto_update() -> AutoUpdateConfig {
    let Ok(path) = config_path() else {
        return AutoUpdateConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return AutoUpdateConfig::default();
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
        return AutoUpdateConfig::default();
    };
    serde_json::from_value(doc.get("auto_update").cloned().unwrap_or_default()).unwrap_or_default()
}

fn save_auto_update(au: &AutoUpdateConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut doc: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    doc["auto_update"] = serde_json::to_value(au)?;
    std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
    Ok(())
}

fn resolve_release_url(au: &AutoUpdateConfig) -> String {
    let override_url = if au.channel == "beta" {
        au.beta_url.as_deref()
    } else {
        au.stable_url.as_deref()
    };
    match override_url {
        Some(u) if !u.is_empty() => u.to_string(),
        _ if au.channel == "beta" => default_release_api_beta().to_string(),
        _ => default_release_api_stable().to_string(),
    }
}

fn env_disabled() -> bool {
    matches!(
        std::env::var("CEC_SUPPORT_AUTOUPDATE").ok().as_deref(),
        Some("0")
    )
}

// ---------------------------------------------------------------------------
// Public types.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    Raw,
    PackageManager,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub current_version: String,
    pub install_kind: InstallKind,
    pub enabled: bool,
    pub channel: String,
    pub auto_apply: String,
    pub check_interval_hours: u32,
    pub last_check_at: Option<i64>,
    pub staged_version: Option<String>,
    pub release_url: String,
    pub release_url_overridden: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CheckOutcome {
    Disabled,
    NotDue,
    UpToDate {
        current: String,
        latest: String,
    },
    PolicyBlocked {
        current: String,
        latest: String,
        policy: String,
    },
    Staged {
        version: String,
    },
    /// A newer release exists, but this install can't swap its own binary —
    /// a package-managed copy, or a per-machine install this process can't
    /// write to. Reported rather than swallowed so the app can still say so.
    ManualUpdateAvailable {
        current: String,
        latest: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum UpdateNowOutcome {
    PackageManager,
    UpToDate { current: String, latest: String },
    Updated { to: String },
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdatePrefs {
    pub enabled: Option<bool>,
    pub channel: Option<String>,
    pub auto_apply: Option<String>,
    pub check_interval_hours: Option<u32>,
    pub stable_url: Option<String>,
    pub beta_url: Option<String>,
}

fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Debug, Serialize, Deserialize)]
struct InstalledVersionMarker {
    version: String,
    sha256: String,
    /// Hashes of companion executables installed beside the GUI. Older
    /// markers lack this map, so upgraded installs repair their companions.
    #[serde(default)]
    companions: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
enum InstalledVersionState {
    /// A fresh/package install has no updater-owned marker. Its running build
    /// is authoritative and must not trigger a redundant self-download.
    Untracked,
    /// The marker still describes the executable currently on disk.
    Verified(String),
    /// A marker exists but no longer describes the executable. Repair it even
    /// when the release tag equals the running process's version.
    Changed,
}

// ---------------------------------------------------------------------------
// The artifact: CEC Support plus the companions it needs at runtime.
// ---------------------------------------------------------------------------

/// Release-asset stem — `cec-support-<platform>.<ext>`, matching the
/// "Package portable binary" step in `.github/workflows/release.yml`. The
/// installers (`.msi` / `-setup.exe`) are a different thing entirely: they're
/// for first install, and can't be applied by swapping a file.
const ASSET_STEM: &str = "cec-support";

fn bin_name() -> &'static str {
    if cfg!(windows) {
        "cec-support.exe"
    } else {
        "cec-support"
    }
}

fn required_companion_names() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &["amst.exe", "cec-crucible-portable.zip"]
    }
    #[cfg(not(windows))]
    {
        &[]
    }
}

fn platform_triple() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-aarch64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x86_64"
    }
    #[cfg(not(any(
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        "unknown"
    }
}

fn archive_ext() -> &'static str {
    if cfg!(windows) {
        "zip"
    } else {
        "tar.gz"
    }
}

fn platform_asset() -> String {
    format!("{ASSET_STEM}-{}.{}", platform_triple(), archive_ext())
}

// ---------------------------------------------------------------------------
// Install-kind detection.
// ---------------------------------------------------------------------------

pub fn detect_install_kind() -> InstallKind {
    let Ok(exe) = std::env::current_exe() else {
        return InstallKind::Raw;
    };
    if detect_install_kind_from_path(&exe.to_string_lossy()) == InstallKind::PackageManager {
        return InstallKind::PackageManager;
    }
    // Not a foreign package manager — the remaining question is whether the
    // swap can actually happen. CEC Support's MSI is a per-machine install into
    // `C:\Program Files\CEC Support\`, which an unelevated process cannot write
    // to: staging a download there produces an apply that fails forever. Treat
    // it as managed so the *check* still runs and reports, and installing goes
    // back through the installer that owns the files.
    match exe.parent() {
        Some(dir) if !dir_is_writable(dir) => InstallKind::PackageManager,
        _ => InstallKind::Raw,
    }
}

/// Whether `dir` accepts writes from this process. Probing by *doing* it is the
/// only reliable test on Windows, where ACLs (and virtualisation) make a
/// permissions read meaningless — and it's what the swap itself will attempt.
fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".cec-support-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Path-only classification: is this binary owned by a *foreign* package
/// manager that must do the updating? Deliberately not a test for "somewhere
/// under Program Files" — that's where our own MSI lands, and treating it as
/// package-managed is what silently disabled self-update in AllMyStuff.
fn detect_install_kind_from_path(path_str: &str) -> InstallKind {
    if path_str.contains("/Cellar/")
        || path_str.starts_with("/opt/homebrew/")
        || path_str.starts_with("/home/linuxbrew/")
        || path_str.starts_with("/usr/local/Cellar/")
    {
        return InstallKind::PackageManager;
    }
    #[cfg(target_os = "linux")]
    if path_str.starts_with("/usr/bin/") || path_str.starts_with("/usr/sbin/") {
        return InstallKind::PackageManager;
    }
    {
        let lower = path_str.to_lowercase();
        if lower.contains("\\chocolatey\\lib\\") || lower.contains("\\scoop\\apps\\") {
            return InstallKind::PackageManager;
        }
    }
    InstallKind::Raw
}

// ---------------------------------------------------------------------------
// Apply (runs at process start, or on demand).
// ---------------------------------------------------------------------------

/// Apply any staged update before real work starts. Idempotent, and never
/// fatal: a failure leaves the staged marker in place so the next launch
/// retries rather than silently dropping the update. Call this first in `main`.
pub fn apply_pending_if_any() {
    cleanup_old_replaced_binary();
    if let Err(e) = apply_pending() {
        tracing::warn!("self-update apply skipped: {e}");
    }
    // An older updater knows only about cec-support.exe. It leaves the verified
    // archive cached after swapping in a newer GUI, so the new GUI can recover
    // AMST from that same payload on its very first launch.
    if let Err(e) = repair_companions_from_cached_archive() {
        tracing::warn!("self-update companion repair skipped: {e}");
    }
}

/// Apply a staged update now, surfacing the applied version (the swap is on
/// disk; it takes effect on next start), or `None` if nothing was pending.
pub fn apply_now() -> Result<Option<String>> {
    cleanup_old_replaced_binary();
    let applied = apply_pending()?;
    if applied.is_some() {
        return Ok(applied);
    }
    repair_companions_from_cached_archive()
        .map(|repaired| repaired.then(|| current_version().to_string()))
}

fn apply_pending() -> Result<Option<String>> {
    let pending = updates_dir()?.join("pending.json");
    if !pending.exists() {
        return Ok(None);
    }
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&pending)?)?;
    let target_version = doc["version"].as_str().unwrap_or("?").to_string();
    let Some(archive) = doc["path"].as_str().map(PathBuf::from) else {
        // A marker that names nothing usable is junk — clear it.
        let _ = std::fs::remove_file(&pending);
        return Ok(None);
    };

    // A newer staged build is an ordinary update. An equal build is also
    // allowed when the verified marker says the executable on disk changed —
    // that is a repair, not a downgrade. Older builds are never applied.
    if !staged_version_may_apply(
        &target_version,
        current_version(),
        &installed_version_state(),
    ) {
        let _ = std::fs::remove_file(&pending);
        return Ok(None);
    }

    let Some(target) = installed_path() else {
        let _ = std::fs::remove_file(&pending);
        return Ok(None);
    };
    let staged_dir = archive
        .parent()
        .ok_or_else(|| Error::msg("staged archive has no parent"))?;
    let binary = extract_binary(&archive, staged_dir, bin_name())?;
    // Install companions first. If the GUI swap then fails, the old GUI can
    // still use the newer AMST and the pending marker remains for a retry. The
    // opposite order could leave a new GUI without the companion it requires.
    install_companions_from_archive(&archive, &target)?;
    atomic_replace(&binary, &target)?;
    record_installed_version(&target_version);

    let _ = std::fs::remove_file(&pending);
    tracing::info!("self-update applied {target_version}");
    Ok(Some(target_version))
}

/// Where CEC Support is installed. `None` when the running binary sits inside
/// an OS bundle we must not mutate.
///
/// The bundle guard is load-bearing: inside a macOS `.app` an in-place Mach-O
/// swap breaks the bundle's signature and identity, so the relaunch comes back
/// refused or running the stale cached image. A bundled app is updated by its
/// own installer, never by swapping a binary underneath it.
fn installed_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    (!path_in_os_bundle(&exe)).then_some(exe)
}

fn companion_target(installed_binary: &Path, name: &str) -> Result<PathBuf> {
    installed_binary
        .parent()
        .map(|dir| dir.join(name))
        .ok_or_else(|| Error::msg("installed binary has no parent directory"))
}

fn required_companions_present(installed_binary: &Path) -> bool {
    required_companion_names().iter().all(|name| {
        companion_target(installed_binary, name)
            .map(|path| path.is_file())
            .unwrap_or(false)
    })
}

fn install_companions_from_archive(archive: &Path, installed_binary: &Path) -> Result<()> {
    let staged_dir = archive
        .parent()
        .ok_or_else(|| Error::msg("update archive has no parent"))?;
    let companions = required_companion_names()
        .iter()
        .map(|name| extract_binary(archive, staged_dir, name).map(|path| (*name, path)))
        .collect::<Result<Vec<_>>>()?;
    for (name, staged) in companions {
        atomic_replace(&staged, &companion_target(installed_binary, name)?)?;
    }
    Ok(())
}

fn installed_version_marker_path() -> Option<PathBuf> {
    updates_dir().ok().map(|d| d.join("installed.version"))
}

fn installed_version_state() -> InstalledVersionState {
    let Some(marker_path) = installed_version_marker_path() else {
        return InstalledVersionState::Untracked;
    };
    if !marker_path.exists() {
        return installed_path()
            .filter(|binary| !required_companions_present(binary))
            .map(|_| InstalledVersionState::Changed)
            .unwrap_or(InstalledVersionState::Untracked);
    }
    let Some(binary) = installed_path() else {
        return InstalledVersionState::Changed;
    };
    validated_marker_version(&marker_path, &binary)
        .map(InstalledVersionState::Verified)
        .unwrap_or(InstalledVersionState::Changed)
}

fn validated_marker_version(marker_path: &Path, binary: &Path) -> Option<String> {
    let text = std::fs::read_to_string(marker_path).ok()?;
    let marker: InstalledVersionMarker = serde_json::from_str(&text).ok()?;
    let actual = sha256_file(binary).ok()?;
    if actual != marker.sha256 {
        return None;
    }
    for name in required_companion_names() {
        let expected = marker.companions.get(*name)?;
        let actual = sha256_file(&companion_target(binary, name).ok()?).ok()?;
        if &actual != expected {
            return None;
        }
    }
    Some(marker.version)
}

fn record_installed_version(version: &str) {
    let Some(marker_path) = installed_version_marker_path() else {
        return;
    };
    let Some(binary) = installed_path() else {
        return;
    };
    let Ok(sha256) = sha256_file(&binary) else {
        return;
    };
    let mut companions = BTreeMap::new();
    for name in required_companion_names() {
        let Ok(target) = companion_target(&binary, name) else {
            return;
        };
        let Ok(hash) = sha256_file(&target) else {
            return;
        };
        companions.insert((*name).to_string(), hash);
    }
    let marker = InstalledVersionMarker {
        version: version.to_string(),
        sha256,
        companions,
    };
    if let Ok(text) = serde_json::to_string(&marker) {
        let _ = std::fs::write(marker_path, text);
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read as _;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn install_needs_update(latest: &str) -> bool {
    version_state_needs_update(&installed_version_state(), current_version(), latest)
}

fn version_state_needs_update(state: &InstalledVersionState, running: &str, latest: &str) -> bool {
    match state {
        InstalledVersionState::Untracked => {
            compare_semver(running, latest) == std::cmp::Ordering::Less
        }
        InstalledVersionState::Verified(version) => {
            compare_semver(version, latest) == std::cmp::Ordering::Less
        }
        InstalledVersionState::Changed => {
            compare_semver(running, latest) != std::cmp::Ordering::Greater
        }
    }
}

fn staged_version_may_apply(target: &str, running: &str, state: &InstalledVersionState) -> bool {
    match compare_semver(target, running) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => matches!(state, InstalledVersionState::Changed),
        std::cmp::Ordering::Less => false,
    }
}

fn path_in_os_bundle(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_str().is_some_and(|s| s.ends_with(".app")))
}

/// Atomically replace `target` with `staged`. A same-dir temp + rename keeps
/// the swap atomic on the target's filesystem. Unix can rename over a running
/// executable (the live process keeps its old inode); Windows can't, so the
/// running binary is side-renamed to `<name>.old` (which Windows *does* allow
/// while it's mapped) and rolled back if the swap-in then fails.
fn atomic_replace(staged: &Path, target: &Path) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| Error::msg("target has no parent dir"))?;
    let tmp = dir.join(format!(".cec-support-update-{}.tmp", std::process::id()));
    std::fs::copy(staged, &tmp).map_err(|e| {
        Error::msg(format!(
            "cannot copy staged binary into {}: {e}",
            dir.display()
        ))
    })?;
    set_exec_perms(&tmp);

    #[cfg(not(windows))]
    {
        std::fs::rename(&tmp, target).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    }
    #[cfg(windows)]
    {
        match std::fs::rename(&tmp, target) {
            Ok(()) => Ok(()),
            Err(_) => rename_via_side_swap_windows(&tmp, target).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            }),
        }
    }
}

#[cfg(windows)]
fn rename_via_side_swap_windows(src: &Path, dst: &Path) -> Result<()> {
    let old = old_binary_path(dst);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(dst, &old).map_err(|e| {
        Error::msg(format!(
            "could not rename running binary aside to {}: {e}",
            old.display()
        ))
    })?;
    if let Err(e) = std::fs::rename(src, dst) {
        // Roll back so we never leave the install without a binary.
        let _ = std::fs::rename(&old, dst);
        return Err(Error::msg(format!(
            "swap-in failed after side-rename ({e}); restored original binary"
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn old_binary_path(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|s| s.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("cec-support"));
    name.push(".old");
    target.with_file_name(name)
}

/// Delete the `<exe>.old` litter a previous Windows side-swap left behind.
/// Cheap, idempotent, runs at startup.
fn cleanup_old_replaced_binary() {
    #[cfg(windows)]
    if let Some(p) = installed_path() {
        let mut targets = vec![p.clone()];
        for name in required_companion_names() {
            if let Ok(target) = companion_target(&p, name) {
                targets.push(target);
            }
        }
        for target in targets {
            let old = old_binary_path(&target);
            if old.exists() {
                let _ = std::fs::remove_file(&old);
            }
        }
    }
}

/// Finish a multi-file update that was downloaded by an older updater. That
/// updater extracts only the GUI but keeps the verified archive in the version
/// cache, allowing this version to install AMST without another download or a
/// manual setup run.
fn repair_companions_from_cached_archive() -> Result<bool> {
    if required_companion_names().is_empty()
        || matches!(
            installed_version_state(),
            InstalledVersionState::Verified(_)
        )
    {
        return Ok(false);
    }
    let Some(target) = installed_path() else {
        return Ok(false);
    };
    let archive = updates_dir()?
        .join(current_version())
        .join(platform_asset());
    if !archive.is_file() {
        return Ok(false);
    }
    install_companions_from_archive(&archive, &target)?;
    record_installed_version(current_version());
    tracing::info!("self-update repaired runtime companions from the cached payload");
    Ok(true)
}

#[cfg(unix)]
fn set_exec_perms(to: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
fn set_exec_perms(_to: &Path) {}

// ---------------------------------------------------------------------------
// Check + stage.
// ---------------------------------------------------------------------------

/// Run one check. With `force`, ignore the interval cooldown. Stages a
/// permitted update; never applies (that happens on next launch).
pub async fn check_now(force: bool) -> Result<CheckOutcome> {
    let au = load_auto_update();
    // A forced check is attended (startup, header Refresh, or Check now).
    // Turning off automatic updates stops the ticker; it must not turn an
    // explicit version check into a cached "disabled" answer.
    if check_is_disabled(au.enabled, force, env_disabled()) {
        return Ok(CheckOutcome::Disabled);
    }
    if !force && !is_due(au.check_interval_hours) {
        return Ok(CheckOutcome::NotDue);
    }

    // A package-managed / unwritable install can't stage or apply anything, but
    // it can still look — that's the difference between "self-update is off
    // here" and telling the customer a new version exists.
    let managed = detect_install_kind() == InstallKind::PackageManager;

    let release = fetch_release(&au).await?;
    let latest = release_tag(&release)?;
    let current = current_version().to_string();
    // Stamped only after a successful fetch: one offline moment must not cost a
    // full `check_interval_hours` before the next attempt.
    stamp_check_now();

    if !install_needs_update(&latest) {
        return Ok(CheckOutcome::UpToDate { current, latest });
    }
    if managed {
        return Ok(CheckOutcome::ManualUpdateAvailable { current, latest });
    }

    let genuine_upgrade = compare_semver(&current, &latest) == std::cmp::Ordering::Less;
    let pol = ApplyPolicy::parse(&au.auto_apply).unwrap_or(ApplyPolicy::Patch);
    if genuine_upgrade && !policy_allows(pol, &current, &latest) {
        return Ok(CheckOutcome::PolicyBlocked {
            current,
            latest,
            policy: au.auto_apply.clone(),
        });
    }

    stage_release(&release, &latest).await?;
    Ok(CheckOutcome::Staged { version: latest })
}

/// Automatic-update preference gates only unattended ticks. The environment
/// switch is an operator kill switch and therefore also gates attended checks.
fn check_is_disabled(auto_enabled: bool, force: bool, disabled_by_env: bool) -> bool {
    disabled_by_env || (!auto_enabled && !force)
}

/// Log what a check decided — *every* outcome. Swallowing the uninteresting
/// ones makes a ticker that is silently disabled and one that is running fine
/// and finding nothing look identical (no output at any log level), which is
/// the hardest part of "it never checks" to diagnose.
fn log_check_outcome(outcome: &CheckOutcome) {
    match outcome {
        CheckOutcome::Staged { version } => {
            tracing::info!("self-update staged {version}; applies on next launch");
        }
        CheckOutcome::ManualUpdateAvailable { current, latest } => {
            tracing::info!(
                "self-update: {latest} is available (running {current}), but this install is \
                 package-managed or not writable by this process — reinstall to update"
            );
        }
        CheckOutcome::UpToDate { current, latest } => {
            tracing::debug!("self-update: up to date (running {current}, latest {latest})");
        }
        CheckOutcome::PolicyBlocked {
            current,
            latest,
            policy,
        } => {
            tracing::info!(
                "self-update: {latest} is available (running {current}) but the '{policy}' \
                 apply policy holds it back"
            );
        }
        CheckOutcome::NotDue => tracing::debug!("self-update: not due yet, skipping this tick"),
        CheckOutcome::Disabled => tracing::debug!("self-update: disabled, skipping this tick"),
    }
}

async fn run_check(force: bool, notify: &(dyn Fn(&CheckOutcome) + Send + Sync)) {
    match check_now(force).await {
        Ok(outcome) => {
            log_check_outcome(&outcome);
            notify(&outcome);
        }
        Err(e) => tracing::warn!("self-update check failed: {e}"),
    }
}

/// Background auto-update ticker — the half that makes self-update "set and
/// forget". A **launch check** fires shortly after start and ignores the
/// interval cooldown, so opening the app is itself a check; then the timer
/// takes over at `check_interval_hours` (re-read each loop, so a settings
/// change takes effect without a restart). Whatever it stages applies on the
/// next launch (see [`apply_pending_if_any`]).
///
/// Spawning this is what separates an updater that runs from one that only ever
/// reacts to a "Check now" button.
pub async fn tick_forever() {
    tick_forever_notify(|_| {}).await
}

/// [`tick_forever`], plus a callback fired with the outcome of every check.
///
/// The desktop app passes a closure that emits a Tauri event, which is what
/// makes "an update is ready" actually reach the customer — a background task
/// holds no handle to the UI and otherwise cannot tell it anything.
pub async fn tick_forever_notify<F>(notify: F)
where
    F: Fn(&CheckOutcome) + Send + Sync + 'static,
{
    // Let a freshly launched app settle (node bring-up, first-run AV scans of
    // the sidecars) before the first network hit.
    tokio::time::sleep(Duration::from_secs(30)).await;
    run_check(true, &notify).await;
    loop {
        let hours = load_auto_update().check_interval_hours.max(1);
        tokio::time::sleep(Duration::from_secs(hours as u64 * 3600)).await;
        run_check(false, &notify).await;
    }
}

/// User-driven "update now". Ignores policy + interval (consent implied) but
/// still defers to a package manager. Stages and applies to disk immediately;
/// the running process picks it up on restart.
pub async fn update_now() -> Result<UpdateNowOutcome> {
    if detect_install_kind() == InstallKind::PackageManager {
        return Ok(UpdateNowOutcome::PackageManager);
    }
    let au = load_auto_update();
    let release = fetch_release(&au).await?;
    let latest = release_tag(&release)?;
    let current = current_version().to_string();
    if !install_needs_update(&latest) {
        return Ok(UpdateNowOutcome::UpToDate { current, latest });
    }
    stage_release(&release, &latest).await?;
    stamp_check_now();
    match apply_now()? {
        Some(to) => Ok(UpdateNowOutcome::Updated { to }),
        None => Ok(UpdateNowOutcome::UpToDate { current, latest }),
    }
}

/// The latest release version on the configured channel (read-only — it stages
/// nothing).
pub async fn latest_version() -> Result<Option<String>> {
    let au = load_auto_update();
    let release = fetch_release(&au).await?;
    Ok(release_tag(&release).ok())
}

pub fn status() -> Result<UpdateStatus> {
    let au = load_auto_update();
    let overridden = if au.channel == "beta" {
        au.beta_url.as_deref().is_some_and(|u| !u.is_empty())
    } else {
        au.stable_url.as_deref().is_some_and(|u| !u.is_empty())
    };
    Ok(UpdateStatus {
        current_version: current_version().to_string(),
        install_kind: detect_install_kind(),
        enabled: au.enabled && !env_disabled(),
        channel: au.channel.clone(),
        auto_apply: au.auto_apply.clone(),
        check_interval_hours: au.check_interval_hours,
        last_check_at: last_check_at(),
        staged_version: staged_version(),
        release_url: resolve_release_url(&au),
        release_url_overridden: overridden,
    })
}

pub fn set_prefs(prefs: UpdatePrefs) -> Result<UpdateStatus> {
    let mut au = load_auto_update();
    if let Some(v) = prefs.enabled {
        au.enabled = v;
    }
    if let Some(v) = prefs.channel {
        au.channel = v;
    }
    if let Some(v) = prefs.auto_apply {
        au.auto_apply = v;
    }
    if let Some(v) = prefs.check_interval_hours {
        au.check_interval_hours = v.max(1);
    }
    if let Some(v) = prefs.stable_url {
        au.stable_url = (!v.is_empty()).then_some(v);
    }
    if let Some(v) = prefs.beta_url {
        au.beta_url = (!v.is_empty()).then_some(v);
    }
    save_auto_update(&au)?;
    status()
}

// ---------------------------------------------------------------------------
// Stamps.
// ---------------------------------------------------------------------------

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn last_check_at() -> Option<i64> {
    let p = updates_dir().ok()?.join("last_check.json");
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    doc["at"].as_i64()
}

fn stamp_check_now() {
    if let Ok(dir) = updates_dir() {
        let _ = std::fs::write(
            dir.join("last_check.json"),
            serde_json::json!({ "at": now_secs() }).to_string(),
        );
    }
}

fn is_due(interval_hours: u32) -> bool {
    match last_check_at() {
        Some(at) => now_secs() - at >= (interval_hours as i64) * 3600,
        None => true,
    }
}

fn staged_version() -> Option<String> {
    let p = updates_dir().ok()?.join("pending.json");
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    doc["version"].as_str().map(str::to_string)
}

// ---------------------------------------------------------------------------
// Network: fetch / stage.
// ---------------------------------------------------------------------------

fn release_tag(release: &serde_json::Value) -> Result<String> {
    // The "latest" endpoint returns an object; the "list" endpoint an array
    // — take the first entry there.
    let obj = if release.is_array() {
        release
            .get(0)
            .ok_or_else(|| Error::msg("empty release list"))?
    } else {
        release
    };
    obj["tag_name"]
        .as_str()
        .map(|s| s.trim_start_matches('v').to_string())
        .ok_or_else(|| Error::msg("release missing tag_name"))
}

async fn fetch_release(au: &AutoUpdateConfig) -> Result<serde_json::Value> {
    let url = resolve_release_url(au);
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json().await?)
}

async fn stage_release(release: &serde_json::Value, version: &str) -> Result<()> {
    let obj = if release.is_array() {
        release
            .get(0)
            .ok_or_else(|| Error::msg("empty release list"))?
    } else {
        release
    };
    let assets = obj["assets"]
        .as_array()
        .ok_or_else(|| Error::msg("release has no assets"))?;
    let dir = updates_dir()?.join(version);
    std::fs::create_dir_all(&dir)?;

    let asset_name = platform_asset();
    let asset = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(asset_name.as_str()))
        .ok_or_else(|| Error::msg(format!("release has no asset {asset_name}")))?;
    let url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| Error::msg("asset missing download url"))?;

    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
    let dest = dir.join(&asset_name);
    download_verify_stage(&client, assets, url, &dest, &asset_name).await?;

    std::fs::write(
        updates_dir()?.join("pending.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "version": version,
            "path": dest.to_string_lossy(),
        }))?,
    )?;
    Ok(())
}

async fn download_verify_stage(
    client: &reqwest::Client,
    assets: &[serde_json::Value],
    url: &str,
    dest: &Path,
    asset_name: &str,
) -> Result<()> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    // Integrity: a published checksum is mandatory. Falling through to a
    // warning when the sidecar is missing would let anyone able to omit it
    // serve any payload, so this refuses to stage instead.
    let expected = find_sha256(assets, asset_name, client)
        .await
        .ok_or_else(|| {
            Error::msg(format!(
                "no checksum sidecar for {asset_name}; refusing to stage unverified"
            ))
        })?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(Error::ChecksumMismatch {
            asset: asset_name.to_string(),
            expected,
            actual,
        });
    }

    // Authenticity: when a release signing key is baked in, a valid detached
    // minisign signature over the artifact is required before staging.
    match release_pubkey() {
        Some(pubkey) => {
            let sig_name = format!("{asset_name}.minisig");
            let sig_asset = assets
                .iter()
                .find(|a| a["name"].as_str() == Some(sig_name.as_str()))
                .ok_or_else(|| {
                    Error::msg(format!("no signature for {asset_name}; refusing to stage"))
                })?;
            let sig_url = sig_asset["browser_download_url"]
                .as_str()
                .ok_or_else(|| Error::msg("signature asset missing url"))?;
            let sig_text = fetch_text(client, sig_url).await?;
            verify_signature(pubkey, &bytes, &sig_text)
                .map_err(|e| Error::msg(format!("signature check failed for {asset_name}: {e}")))?;
        }
        None => tracing::warn!(
            "release signing not configured in this build; {asset_name} verified by SHA-256 only"
        ),
    }

    std::fs::write(dest, &bytes)?;
    Ok(())
}

fn verify_signature(pubkey: &str, bytes: &[u8], sig_text: &str) -> std::result::Result<(), String> {
    use minisign_verify::{PublicKey, Signature};
    let pk = PublicKey::from_base64(pubkey).map_err(|e| e.to_string())?;
    let sig = Signature::decode(sig_text).map_err(|e| e.to_string())?;
    pk.verify(bytes, &sig, false).map_err(|e| e.to_string())
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// The expected SHA-256 for `asset_name`, from its published `.sha256` sidecar.
/// The file is `sha256sum` format ("<hex>  <name>"), so take the first field.
async fn find_sha256(
    assets: &[serde_json::Value],
    asset_name: &str,
    client: &reqwest::Client,
) -> Option<String> {
    let want = format!("{asset_name}.sha256");
    let asset = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(want.as_str()))?;
    let url = asset["browser_download_url"].as_str()?;
    let text = fetch_text(client, url).await.ok()?;
    text.split_whitespace().next().map(str::to_string)
}

/// Pull `bin_name` out of a staged archive into `out_dir`, returning its path.
/// A bare (un-archived) binary is passed through unchanged.
fn extract_binary(archive: &Path, out_dir: &Path, bin_name: &str) -> Result<PathBuf> {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let out = out_dir.join(bin_name);

    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let f = std::fs::File::open(archive)?;
        let dec = flate2::read::GzDecoder::new(f);
        let mut ar = tar::Archive::new(dec);
        for entry in ar.entries()? {
            let mut entry = entry?;
            let is_match = entry
                .path()?
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == bin_name)
                .unwrap_or(false);
            if is_match {
                let mut dst = std::fs::File::create(&out)?;
                std::io::copy(&mut entry, &mut dst)?;
                return Ok(out);
            }
        }
        Err(Error::msg(format!("{bin_name} not found in {name}")))
    } else if name.ends_with(".zip") {
        let f = std::fs::File::open(archive)?;
        let mut zip = zip::ZipArchive::new(f).map_err(|e| Error::msg(e.to_string()))?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).map_err(|e| Error::msg(e.to_string()))?;
            let fname = Path::new(file.name())
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string);
            if fname.as_deref() == Some(bin_name) {
                let mut dst = std::fs::File::create(&out)?;
                std::io::copy(&mut file, &mut dst)?;
                return Ok(out);
            }
        }
        Err(Error::msg(format!("{bin_name} not found in {name}")))
    } else {
        // Already a bare binary.
        Ok(archive.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CEC_SUPPORT_HOME` is process-global; serialize the tests that mutate it
    /// so cargo's parallel runner can't cross their temp dirs.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn attended_checks_ignore_only_the_automatic_update_preference() {
        assert!(check_is_disabled(false, false, false));
        assert!(!check_is_disabled(false, true, false));
        assert!(!check_is_disabled(true, false, false));
        assert!(check_is_disabled(true, true, true));
    }

    #[test]
    fn package_managers_are_detected_but_program_files_is_not() {
        assert_eq!(
            detect_install_kind_from_path("/opt/homebrew/bin/cec-support"),
            InstallKind::PackageManager
        );
        assert_eq!(
            detect_install_kind_from_path(
                r"C:\ProgramData\chocolatey\lib\cec-support\cec-support.exe"
            ),
            InstallKind::PackageManager
        );
        // Our own MSI lands in Program Files. Classifying that as
        // package-managed is exactly what silently switched AllMyStuff's
        // self-update — the check included — off for every MSI install, so the
        // path alone must not decide it; `dir_is_writable` asks at runtime.
        assert_eq!(
            detect_install_kind_from_path(r"C:\Program Files\CEC Support\cec-support.exe"),
            InstallKind::Raw
        );
        assert_eq!(
            detect_install_kind_from_path(r"C:\Users\me\AppData\Local\CEC Support\cec-support.exe"),
            InstallKind::Raw
        );
    }

    #[test]
    fn write_probe_tells_a_writable_dir_from_a_missing_one() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(dir_is_writable(tmp.path()));
        // It must clean up after itself — a stray file next to the installed
        // binary on every status() call would be its own bug report.
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
        assert!(!dir_is_writable(&tmp.path().join("nope")));
    }

    #[test]
    fn asset_name_has_stem_triple_and_ext() {
        let a = platform_asset();
        assert!(a.starts_with("cec-support-"));
        assert!(a.ends_with(".tar.gz") || a.ends_with(".zip"));
        assert_eq!(
            bin_name(),
            if cfg!(windows) {
                "cec-support.exe"
            } else {
                "cec-support"
            }
        );
    }

    #[test]
    fn installed_marker_is_bound_to_the_binary_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join(bin_name());
        let marker_path = tmp.path().join("installed.version");
        std::fs::write(&binary, b"cec support current").unwrap();
        let mut companions = BTreeMap::new();
        for name in required_companion_names() {
            let companion = tmp.path().join(name);
            std::fs::write(&companion, b"companion current").unwrap();
            companions.insert((*name).to_string(), sha256_file(&companion).unwrap());
        }
        let marker = InstalledVersionMarker {
            version: "0.2.39".into(),
            sha256: sha256_file(&binary).unwrap(),
            companions,
        };
        std::fs::write(&marker_path, serde_json::to_string(&marker).unwrap()).unwrap();

        assert_eq!(
            validated_marker_version(&marker_path, &binary).as_deref(),
            Some("0.2.39")
        );

        std::fs::write(&binary, b"cec support rolled back").unwrap();
        assert_eq!(validated_marker_version(&marker_path, &binary), None);

        // The GUI hash alone cannot certify a payload whose AMST companion was
        // removed or rolled back.
        std::fs::write(&binary, b"cec support current").unwrap();
        if let Some(name) = required_companion_names().first() {
            std::fs::write(tmp.path().join(name), b"companion rolled back").unwrap();
            assert_eq!(validated_marker_version(&marker_path, &binary), None);

            // Markers written by the previous GUI-only updater deserialize but
            // cannot certify AMST, forcing the cached-payload repair path.
            std::fs::write(
                &marker_path,
                serde_json::json!({
                    "version": "0.2.39",
                    "sha256": sha256_file(&binary).unwrap(),
                })
                .to_string(),
            )
            .unwrap();
            assert_eq!(validated_marker_version(&marker_path, &binary), None);
        }

        // A malformed or old bare-version marker cannot falsely certify an
        // executable either.
        std::fs::write(&marker_path, "0.2.39\n").unwrap();
        assert_eq!(validated_marker_version(&marker_path, &binary), None);
    }

    #[test]
    fn changed_install_repairs_equal_version_without_allowing_downgrade() {
        assert!(!version_state_needs_update(
            &InstalledVersionState::Untracked,
            "0.2.39",
            "0.2.39"
        ));
        assert!(version_state_needs_update(
            &InstalledVersionState::Untracked,
            "0.2.39",
            "0.2.40"
        ));
        assert!(version_state_needs_update(
            &InstalledVersionState::Verified("0.2.38".into()),
            "0.2.39",
            "0.2.39"
        ));
        assert!(version_state_needs_update(
            &InstalledVersionState::Changed,
            "0.2.39",
            "0.2.39"
        ));
        assert!(!version_state_needs_update(
            &InstalledVersionState::Changed,
            "0.2.40",
            "0.2.39"
        ));

        assert!(staged_version_may_apply(
            "0.2.39",
            "0.2.39",
            &InstalledVersionState::Changed
        ));
        assert!(!staged_version_may_apply(
            "0.2.39",
            "0.2.39",
            &InstalledVersionState::Untracked
        ));
        assert!(!staged_version_may_apply(
            "0.2.38",
            "0.2.39",
            &InstalledVersionState::Changed
        ));
    }

    #[test]
    fn os_bundle_paths_are_left_alone() {
        assert!(path_in_os_bundle(Path::new(
            "/Applications/CEC Support.app/Contents/MacOS/cec-support"
        )));
        assert!(!path_in_os_bundle(Path::new(
            r"C:\Program Files\CEC Support\cec-support.exe"
        )));
        assert!(!path_in_os_bundle(Path::new("/usr/local/bin/cec-support")));
    }

    #[test]
    fn release_tag_handles_object_and_array() {
        let obj = serde_json::json!({ "tag_name": "v0.1.28" });
        assert_eq!(release_tag(&obj).unwrap(), "0.1.28");
        let arr = serde_json::json!([{ "tag_name": "v0.2.0" }]);
        assert_eq!(release_tag(&arr).unwrap(), "0.2.0");
    }

    #[test]
    fn empty_baked_pubkey_is_treated_as_unconfigured() {
        // CI may export CEC_RELEASE_PUBKEY unconditionally, so an unset repo
        // variable reaches the compiler as Some("") — that must degrade to
        // SHA-256-only, never "require a signature verified against an empty
        // key", which would fail every update closed against a `.minisig`
        // nobody publishes.
        assert_eq!(normalize_pubkey(Some("")), None);
        assert_eq!(normalize_pubkey(None), None);
        assert_eq!(normalize_pubkey(Some("   \r\n  ")), None);
        let real = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        assert_eq!(normalize_pubkey(Some(real)), Some(real));
        // A whole minisign.pub pasted in (CRLF, comment on line 1).
        let whole = format!("untrusted comment: minisign public key ABC\r\n{real}\r\n");
        assert_eq!(normalize_pubkey(Some(&whole)), Some(real));
    }

    #[test]
    fn signature_verification_fails_closed_on_garbage() {
        assert!(verify_signature("not-a-key", b"payload", "not-a-sig").is_err());
    }

    #[test]
    fn auto_apply_defaults_to_up_to_minor() {
        assert_eq!(AutoUpdateConfig::default().auto_apply, "minor");
        assert_eq!(
            ApplyPolicy::parse(&AutoUpdateConfig::default().auto_apply),
            Some(ApplyPolicy::Minor)
        );
        assert!(AutoUpdateConfig::default().enabled);
    }

    #[test]
    fn config_round_trips_under_a_temp_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(HOME_ENV, tmp.path());

        let au = AutoUpdateConfig {
            channel: "beta".into(),
            auto_apply: "patch".into(),
            ..AutoUpdateConfig::default()
        };
        save_auto_update(&au).unwrap();
        let back = load_auto_update();
        assert_eq!(back.channel, "beta");
        assert_eq!(back.auto_apply, "patch");
        // Other config keys survive an auto_update write.
        let cfg: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config_path().unwrap()).unwrap())
                .unwrap();
        assert!(cfg.get("auto_update").is_some());

        std::env::remove_var(HOME_ENV);
    }

    #[test]
    fn release_url_follows_the_channel_and_overrides() {
        let mut au = AutoUpdateConfig::default();
        assert_eq!(resolve_release_url(&au), default_release_api_stable());
        au.channel = "beta".into();
        assert_eq!(resolve_release_url(&au), default_release_api_beta());
        au.beta_url = Some("https://example.invalid/feed".into());
        assert_eq!(resolve_release_url(&au), "https://example.invalid/feed");
        // An empty override falls back to the built-in feed rather than
        // resolving to an empty URL every fetch then fails on.
        au.beta_url = Some(String::new());
        assert_eq!(resolve_release_url(&au), default_release_api_beta());
    }

    #[test]
    fn extract_binary_pulls_the_named_file_from_a_tar_gz() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("cec-support-linux-x86_64.tar.gz");
        {
            let f = std::fs::File::create(&archive).unwrap();
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            let payload = b"#!/bin/sh\necho hi\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "cec-support", &payload[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let bin = extract_binary(&archive, &out, "cec-support").unwrap();
        assert!(bin.exists());
        let s = std::fs::read_to_string(&bin).unwrap();
        assert!(s.contains("echo hi"));
    }

    #[test]
    fn self_update_zip_carries_gui_and_runtime_tools() {
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("cec-support-windows-x86_64.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("cec-support.exe", options).unwrap();
            zip.write_all(b"new gui").unwrap();
            zip.start_file("amst.exe", options).unwrap();
            zip.write_all(b"new amst").unwrap();
            zip.start_file("cec-crucible-portable.zip", options)
                .unwrap();
            zip.write_all(b"new crucible portable payload").unwrap();
            zip.finish().unwrap();
        }
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        assert_eq!(
            std::fs::read(extract_binary(&archive, &out, "cec-support.exe").unwrap()).unwrap(),
            b"new gui"
        );
        assert_eq!(
            std::fs::read(extract_binary(&archive, &out, "amst.exe").unwrap()).unwrap(),
            b"new amst"
        );
        assert_eq!(
            std::fs::read(extract_binary(&archive, &out, "cec-crucible-portable.zip").unwrap())
                .unwrap(),
            b"new crucible portable payload"
        );
    }

    #[cfg(windows)]
    #[test]
    fn companion_install_repairs_old_or_missing_runtime_tools() {
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("install");
        let stage = tmp.path().join("stage");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        let installed_gui = install.join("cec-support.exe");
        std::fs::write(&installed_gui, b"running gui").unwrap();
        for name in required_companion_names() {
            std::fs::write(install.join(name), format!("old {name}")).unwrap();
        }
        let archive = stage.join("cec-support-windows-x86_64.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("cec-support.exe", options).unwrap();
            zip.write_all(b"new gui").unwrap();
            for name in required_companion_names() {
                zip.start_file(name, options).unwrap();
                zip.write_all(format!("new {name}").as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }

        install_companions_from_archive(&archive, &installed_gui).unwrap();
        for name in required_companion_names() {
            assert_eq!(
                std::fs::read(install.join(name)).unwrap(),
                format!("new {name}").as_bytes()
            );
        }
        let missing = required_companion_names()[0];
        std::fs::remove_file(install.join(missing)).unwrap();
        install_companions_from_archive(&archive, &installed_gui).unwrap();
        assert_eq!(
            std::fs::read(install.join(missing)).unwrap(),
            format!("new {missing}").as_bytes()
        );
    }

    #[test]
    fn a_stale_pending_marker_never_downgrades() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(HOME_ENV, tmp.path());

        // A marker naming an OLDER version than the running build must be
        // discarded, not applied — otherwise a leftover pending.json rolls the
        // customer back on the next launch.
        let dir = updates_dir().unwrap();
        std::fs::write(
            dir.join("pending.json"),
            serde_json::json!({ "version": "0.0.1", "path": "/nonexistent" }).to_string(),
        )
        .unwrap();
        assert_eq!(apply_pending().unwrap(), None);
        assert!(!dir.join("pending.json").exists());

        std::env::remove_var(HOME_ENV);
    }

    #[test]
    fn staged_version_reads_the_marker() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(HOME_ENV, tmp.path());

        assert_eq!(staged_version(), None);
        std::fs::write(
            updates_dir().unwrap().join("pending.json"),
            serde_json::json!({ "version": "9.9.9", "path": "/x" }).to_string(),
        )
        .unwrap();
        assert_eq!(staged_version().as_deref(), Some("9.9.9"));

        std::env::remove_var(HOME_ENV);
    }

    #[test]
    fn interval_gate_opens_when_never_checked_and_closes_right_after() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(HOME_ENV, tmp.path());

        assert!(is_due(24), "a home that never checked is always due");
        stamp_check_now();
        assert!(!is_due(24), "a check just now is not due again");
        assert!(is_due(0), "a zero-hour interval is always due");

        std::env::remove_var(HOME_ENV);
    }
}
