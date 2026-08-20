//! # cec-support-service
//!
//! Installs the CEC Support client as a background OS service so it survives
//! reboots — the "unattended access" half of an AnyDesk-style repair. Three
//! backends, one interface:
//!
//! - **Linux** — a systemd unit (`--system` for a machine-wide `LocalSystem`-
//!   style unit, or a per-user unit with linger).
//! - **macOS** — a launchd agent/daemon.
//! - **Windows** — a Service Control Manager service via `sc.exe`, running
//!   `"<exe>" run --service` as an auto-start `LocalSystem` service. This is
//!   the primary target.
//!
//! It deliberately uses its **own** service identity ([`SERVICE_NAME`]) and
//! installs the CEC Support binary — never AllMyStuff's — so a machine that
//! already runs AllMyStuff keeps its service untouched ("reuse, don't
//! clobber"). The service simply runs the current executable in `run --service`
//! mode; that headless agent joins the customer's Silent support mesh and
//! honours their standing approvals.
//!
//! The rendered unit/plist/`sc` command line are produced by pure functions
//! ([`render_systemd_unit`], [`render_launchd_plist`], [`windows_bin_path`])
//! so they can be unit-tested on any platform, exactly like `allmystuff-service`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Human/registry name of the service.
pub const SERVICE_NAME: &str = "CECSupport";
/// systemd unit file name.
pub const SYSTEMD_UNIT: &str = "cec-support.service";
/// launchd label (reverse-DNS of support.cec.direct).
pub const LAUNCHD_LABEL: &str = "direct.cec.support";
/// Windows SCM service name.
pub const WINDOWS_SERVICE_NAME: &str = "CECSupport";
/// Environment variable that overrides where CEC Support keeps its state.
pub const HOME_ENV: &str = "CEC_SUPPORT_HOME";

/// The verbs a caller can drive, parsed from argv or the GUI's service tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceCmd {
    /// Install + enable + start. `log` sets a log filter env for the service.
    Install { log: Option<String> },
    /// Disable + remove.
    Uninstall,
    /// Start an installed service.
    Start,
    /// Stop a running service.
    Stop,
    /// Restart (stop then start).
    Restart,
    /// Print status as JSON.
    Status,
}

/// Which init system is managing services on this host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    /// Linux systemd.
    Systemd,
    /// macOS launchd.
    Launchd,
    /// Windows Service Control Manager.
    Windows,
}

/// User-level or machine-wide install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Per-user (systemd `--user` / a `LaunchAgent`). No privilege needed.
    User,
    /// Machine-wide (`/etc/systemd/system`, a `LaunchDaemon`, or the Windows
    /// SCM, which is always machine-wide). Needs elevation.
    System,
}

impl Scope {
    fn from_flag(system: bool) -> Self {
        if system {
            Scope::System
        } else {
            Scope::User
        }
    }
}

/// Detect the OS service manager, or an error naming the unsupported platform.
pub fn current_manager() -> Result<Manager> {
    #[cfg(target_os = "linux")]
    {
        Ok(Manager::Systemd)
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Manager::Launchd)
    }
    #[cfg(target_os = "windows")]
    {
        Ok(Manager::Windows)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        bail!("CEC Support service install is not supported on this platform")
    }
}

/// The single entry point. Dispatches `cmd` to the platform backend.
pub fn run(system: bool, cmd: ServiceCmd) -> Result<()> {
    let manager = current_manager()?;
    // Windows SCM is inherently machine-wide; the user/system split collapses.
    let scope = if manager == Manager::Windows {
        Scope::System
    } else {
        Scope::from_flag(system)
    };

    if matches!(cmd, ServiceCmd::Status) {
        println!("{}", serde_json::to_string_pretty(&status_value(system)?)?);
        return Ok(());
    }

    let exe = std::env::current_exe().context("locating the CEC Support executable")?;
    match manager {
        Manager::Systemd => systemd::run(scope, &exe, cmd),
        Manager::Launchd => launchd::run(scope, &exe, cmd),
        Manager::Windows => windows::run(&exe, cmd),
    }
}

/// A stable status document for the GUI's service tab / `cec-support service
/// status --json`.
pub fn status_value(system: bool) -> Result<serde_json::Value> {
    let manager = current_manager().ok();
    let scope = if manager == Some(Manager::Windows) {
        Scope::System
    } else {
        Scope::from_flag(system)
    };
    let installed = match manager {
        Some(Manager::Systemd) => systemd::unit_path(scope).exists(),
        Some(Manager::Launchd) => launchd::plist_path(scope).exists(),
        Some(Manager::Windows) => windows::is_installed(),
        None => false,
    };
    let privileged_host_current =
        manager == Some(Manager::Windows) && installed && windows::is_privileged_host_current();
    Ok(serde_json::json!({
        "platform": std::env::consts::OS,
        "supported": manager.is_some(),
        "manager": manager.map(|m| format!("{m:?}").to_lowercase()),
        "scope": format!("{scope:?}").to_lowercase(),
        "service_name": SERVICE_NAME,
        "installed": installed,
        "privileged_host_current": privileged_host_current,
    }))
}

// ---------------------------------------------------------------------------
// Pure renderers (unit-tested on every platform)
// ---------------------------------------------------------------------------

/// Render the systemd unit for `exec`. `env` are `KEY=VALUE` `Environment=`
/// lines; `system` toggles the hardening + `WantedBy` appropriate to a
/// machine-wide unit.
pub fn render_systemd_unit(exec: &Path, system: bool, env: &[(String, String)]) -> String {
    let exec = exec.display();
    let mut s = String::new();
    s.push_str("[Unit]\n");
    s.push_str("Description=CEC Support — remote help desk agent\n");
    s.push_str("After=network-online.target\n");
    s.push_str("Wants=network-online.target\n\n");
    s.push_str("[Service]\n");
    s.push_str("Type=simple\n");
    for (k, v) in env {
        s.push_str(&format!("Environment={k}={v}\n"));
    }
    s.push_str(&format!("ExecStart={exec} run --service\n"));
    s.push_str("Restart=on-failure\n");
    s.push_str("RestartSec=5\n");
    s.push_str("KillSignal=SIGTERM\n");
    if system {
        s.push_str("DynamicUser=yes\n");
        s.push_str("StateDirectory=cec-support\n");
    }
    s.push_str("\n[Install]\n");
    s.push_str(if system {
        "WantedBy=multi-user.target\n"
    } else {
        "WantedBy=default.target\n"
    });
    s
}

/// Render the launchd plist for `exec`, logging to `log_path`.
pub fn render_launchd_plist(exec: &Path, log_path: &Path) -> String {
    let exec = exec.display();
    let log = log_path.display();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exec}</string>
        <string>run</string>
        <string>--service</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
    <key>StandardOutPath</key><string>{log}</string>
    <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#
    )
}

/// The Windows SCM `binPath=` value: the quoted exe plus `run --service`.
pub fn windows_bin_path(
    exec: &Path,
    profile_home: &Path,
    cec_home: &Path,
    client_sid: Option<&str>,
) -> String {
    let mut value = format!(
        "\"{}\" run --service --profile-home {} --cec-home {}",
        exec.display(),
        windows_quote(&profile_home.to_string_lossy()),
        windows_quote(&cec_home.to_string_lossy()),
    );
    if let Some(sid) = client_sid {
        value.push_str(" --client-sid ");
        value.push_str(&windows_quote(sid));
    }
    value.push_str(" --serve-bin ");
    value.push_str(&windows_quote(&windows_sibling_path(
        exec,
        "allmystuff-serve.exe",
    )));
    value.push_str(" --mesh-bin ");
    value.push_str(&windows_quote(&windows_sibling_path(exec, "myownmesh.exe")));
    value
}

fn windows_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', ""))
}

/// Resolve a sibling using Windows separators even when this pure renderer is
/// unit-tested on Unix. `std::path::Path` follows the *host* OS, so on Linux or
/// macOS it treats the backslashes in `C:\Program Files\...` as ordinary
/// filename characters and `with_file_name` incorrectly drops the directory.
fn windows_sibling_path(exec: &Path, sibling: &str) -> String {
    let exec = exec.to_string_lossy();
    match exec.rfind(['\\', '/']) {
        Some(index) => format!("{}\\{sibling}", &exec[..index]),
        None => sibling.to_string(),
    }
}

// ---------------------------------------------------------------------------
// systemd backend
// ---------------------------------------------------------------------------

mod systemd {
    use super::*;

    pub(super) fn unit_path(scope: Scope) -> PathBuf {
        match scope {
            Scope::System => PathBuf::from("/etc/systemd/system").join(SYSTEMD_UNIT),
            Scope::User => dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from(".config"))
                .join("systemd/user")
                .join(SYSTEMD_UNIT),
        }
    }

    fn systemctl(scope: Scope, args: &[&str]) -> Result<()> {
        let mut cmd = std::process::Command::new("systemctl");
        if scope == Scope::User {
            cmd.arg("--user");
        }
        let status = cmd.args(args).status().context("running systemctl")?;
        if !status.success() {
            bail!("systemctl {args:?} failed");
        }
        Ok(())
    }

    pub(super) fn run(scope: Scope, exe: &Path, cmd: ServiceCmd) -> Result<()> {
        if scope == Scope::System {
            ensure_root()?;
        }
        match cmd {
            ServiceCmd::Install { log } => {
                let env = compute_env(scope, log);
                let unit = render_systemd_unit(exe, scope == Scope::System, &env);
                let path = unit_path(scope);
                write_file(&path, &unit)?;
                systemctl(scope, &["daemon-reload"])?;
                systemctl(scope, &["enable", "--now", SYSTEMD_UNIT])?;
                if scope == Scope::User {
                    // Survive logout on a headless box; best-effort.
                    let _ = enable_linger();
                }
                println!("Installed and started {SYSTEMD_UNIT} ({scope:?} scope).");
                Ok(())
            }
            ServiceCmd::Uninstall => {
                let _ = systemctl(scope, &["disable", "--now", SYSTEMD_UNIT]);
                let _ = std::fs::remove_file(unit_path(scope));
                let _ = systemctl(scope, &["daemon-reload"]);
                println!("Removed {SYSTEMD_UNIT}.");
                Ok(())
            }
            ServiceCmd::Start => systemctl(scope, &["start", SYSTEMD_UNIT]),
            ServiceCmd::Stop => systemctl(scope, &["stop", SYSTEMD_UNIT]),
            ServiceCmd::Restart => systemctl(scope, &["restart", SYSTEMD_UNIT]),
            ServiceCmd::Status => unreachable!("status handled in run()"),
        }
    }

    fn compute_env(scope: Scope, log: Option<String>) -> Vec<(String, String)> {
        let mut env = Vec::new();
        if scope == Scope::System {
            env.push((HOME_ENV.into(), "/var/lib/cec-support".into()));
        } else if let Ok(home) = std::env::var(HOME_ENV) {
            env.push((HOME_ENV.into(), home));
        }
        if let Some(filter) = log {
            env.push(("MYOWNMESH_LOG".into(), filter));
        }
        env
    }

    fn enable_linger() -> Result<()> {
        let user = std::env::var("USER").unwrap_or_default();
        std::process::Command::new("loginctl")
            .args(["enable-linger", &user])
            .status()
            .ok();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// launchd backend
// ---------------------------------------------------------------------------

mod launchd {
    use super::*;

    pub(super) fn plist_path(scope: Scope) -> PathBuf {
        let file = format!("{LAUNCHD_LABEL}.plist");
        match scope {
            Scope::System => PathBuf::from("/Library/LaunchDaemons").join(file),
            Scope::User => dirs::home_dir()
                .unwrap_or_default()
                .join("Library/LaunchAgents")
                .join(file),
        }
    }

    fn log_path(scope: Scope) -> PathBuf {
        match scope {
            Scope::System => PathBuf::from("/Library/Logs/CEC Support/agent.log"),
            Scope::User => dirs::home_dir()
                .unwrap_or_default()
                .join("Library/Logs/CEC Support/agent.log"),
        }
    }

    pub(super) fn run(scope: Scope, exe: &Path, cmd: ServiceCmd) -> Result<()> {
        if scope == Scope::System {
            ensure_root()?;
        }
        let plist = plist_path(scope);
        match cmd {
            ServiceCmd::Install { .. } => {
                let log = log_path(scope);
                if let Some(dir) = log.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                write_file(&plist, &render_launchd_plist(exe, &log))?;
                launchctl(&["load", "-w", &plist.to_string_lossy()])?;
                println!("Installed and loaded {LAUNCHD_LABEL} ({scope:?} scope).");
                Ok(())
            }
            ServiceCmd::Uninstall => {
                let _ = launchctl(&["unload", "-w", &plist.to_string_lossy()]);
                let _ = std::fs::remove_file(&plist);
                println!("Removed {LAUNCHD_LABEL}.");
                Ok(())
            }
            ServiceCmd::Start => launchctl(&["load", "-w", &plist.to_string_lossy()]),
            ServiceCmd::Stop => launchctl(&["unload", "-w", &plist.to_string_lossy()]),
            ServiceCmd::Restart => {
                let _ = launchctl(&["unload", "-w", &plist.to_string_lossy()]);
                launchctl(&["load", "-w", &plist.to_string_lossy()])
            }
            ServiceCmd::Status => unreachable!("status handled in run()"),
        }
    }

    fn launchctl(args: &[&str]) -> Result<()> {
        let status = std::process::Command::new("launchctl")
            .args(args)
            .status()
            .context("running launchctl")?;
        if !status.success() {
            bail!("launchctl {args:?} failed");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Windows backend (compiled only on Windows; a stub elsewhere so the crate
// builds on the Linux/macOS CI runners too)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod windows {
    use std::os::windows::process::CommandExt;

    use super::*;

    /// `sc.exe` is a console program; spawned from the `windows_subsystem =
    /// "windows"` GUI (which has no console of its own) each call would flash
    /// a terminal window. CREATE_NO_WINDOW keeps every invocation invisible —
    /// the GUI polls `is_installed` at startup, so without it the app opens
    /// with console flicker.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub(super) fn is_installed() -> bool {
        std::process::Command::new("sc")
            .args(["query", WINDOWS_SERVICE_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub(super) fn is_privileged_host_current() -> bool {
        std::process::Command::new("sc")
            .args(["qc", WINDOWS_SERVICE_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("--profile-home")
            })
            .unwrap_or(false)
    }

    pub(super) fn run(exe: &Path, cmd: ServiceCmd) -> Result<()> {
        match cmd {
            ServiceCmd::Install { .. } => {
                // Replace cleanly if it already exists.
                let _ = sc(&["stop", WINDOWS_SERVICE_NAME]);
                wait_stopped();
                let _ = sc(&["delete", WINDOWS_SERVICE_NAME]);
                let service_dir = std::env::var_os("ProgramData")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
                    .join("Critical Error Computing")
                    .join("CEC Support")
                    .join("service");
                std::fs::create_dir_all(&service_dir)
                    .with_context(|| format!("creating {}", service_dir.display()))?;
                let staged_exe = service_dir.join("cec-support.exe");
                std::fs::copy(exe, &staged_exe).with_context(|| {
                    format!("staging {} as {}", exe.display(), staged_exe.display())
                })?;
                let source_dir = exe.parent().unwrap_or_else(|| Path::new("."));
                for sidecar in ["allmystuff-serve.exe", "myownmesh.exe"] {
                    let source = source_dir.join(sidecar);
                    if !source.is_file() {
                        bail!(
                            "required bundled service sidecar is missing: {}",
                            source.display()
                        );
                    }
                    std::fs::copy(&source, service_dir.join(sidecar))
                        .with_context(|| format!("staging {}", source.display()))?;
                }
                harden_service_dir(&service_dir)?;
                let profile = std::env::var_os("CEC_SUPPORT_SERVICE_PROFILE")
                    .map(PathBuf::from)
                    .or_else(dirs::home_dir)
                    .context("resolving the installing user's profile")?;
                let cec_home = std::env::var_os("CEC_SUPPORT_SERVICE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| profile.join("AppData/Roaming/CEC Support"));
                let sid = std::env::var("CEC_SUPPORT_SERVICE_CLIENT_SID").ok();
                let bin = windows_bin_path(&staged_exe, &profile, &cec_home, sid.as_deref());
                sc(&[
                    "create",
                    WINDOWS_SERVICE_NAME,
                    "binPath=",
                    &bin,
                    "start=",
                    "auto",
                    "obj=",
                    "LocalSystem",
                    "DisplayName=",
                    SERVICE_NAME,
                ])?;
                let _ = sc(&[
                    "description",
                    WINDOWS_SERVICE_NAME,
                    "CEC Support — remote help desk agent (Critical Error Computing).",
                ]);
                // Auto-restart on crash.
                let _ = sc(&[
                    "failure",
                    WINDOWS_SERVICE_NAME,
                    "reset=",
                    "86400",
                    "actions=",
                    "restart/5000/restart/5000/restart/5000",
                ]);
                sc(&["start", WINDOWS_SERVICE_NAME])?;
                println!("Installed and started the {SERVICE_NAME} service.");
                Ok(())
            }
            ServiceCmd::Uninstall => {
                let _ = sc(&["stop", WINDOWS_SERVICE_NAME]);
                wait_stopped();
                sc(&["delete", WINDOWS_SERVICE_NAME])?;
                let service_dir = std::env::var_os("ProgramData")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
                    .join("Critical Error Computing")
                    .join("CEC Support")
                    .join("service");
                let _ = std::fs::remove_dir_all(service_dir);
                println!("Removed the {SERVICE_NAME} service.");
                Ok(())
            }
            ServiceCmd::Start => sc(&["start", WINDOWS_SERVICE_NAME]),
            ServiceCmd::Stop => sc(&["stop", WINDOWS_SERVICE_NAME]),
            ServiceCmd::Restart => {
                let _ = sc(&["stop", WINDOWS_SERVICE_NAME]);
                sc(&["start", WINDOWS_SERVICE_NAME])
            }
            ServiceCmd::Status => unreachable!("status handled in run()"),
        }
    }

    fn sc(args: &[&str]) -> Result<()> {
        let status = std::process::Command::new("sc")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .context("running sc.exe (run as Administrator)")?;
        if !status.success() {
            bail!("sc {:?} failed (elevation required?)", args);
        }
        Ok(())
    }

    fn harden_service_dir(path: &Path) -> Result<()> {
        let status = std::process::Command::new("icacls")
            .arg(path)
            .args([
                "/inheritance:r",
                "/grant:r",
                "SYSTEM:(OI)(CI)F",
                "Administrators:(OI)(CI)F",
                "Users:(OI)(CI)RX",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .context("hardening the CEC Support service directory")?;
        if !status.success() {
            bail!("couldn't protect the CEC Support service directory");
        }
        Ok(())
    }

    fn wait_stopped() {
        for _ in 0..40 {
            let stopped = std::process::Command::new("sc")
                .args(["query", WINDOWS_SERVICE_NAME])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map(|output| {
                    !output.status.success()
                        || String::from_utf8_lossy(&output.stdout).contains("STOPPED")
                })
                .unwrap_or(true);
            if stopped {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
}

#[cfg(not(windows))]
mod windows {
    use super::*;

    pub(super) fn is_installed() -> bool {
        false
    }

    pub(super) fn is_privileged_host_current() -> bool {
        false
    }

    pub(super) fn run(_exe: &Path, _cmd: ServiceCmd) -> Result<()> {
        bail!("the Windows service backend is only available on Windows")
    }
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

fn write_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn ensure_root() -> Result<()> {
    // Safe: geteuid has no preconditions and no side effects.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        bail!("a machine-wide (--system) install needs root; re-run with sudo");
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_root() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_runs_the_service_mode() {
        let unit = render_systemd_unit(Path::new("/usr/bin/cec-support"), false, &[]);
        assert!(unit.contains("ExecStart=/usr/bin/cec-support run --service"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
        assert!(!unit.contains("DynamicUser")); // user scope
    }

    #[test]
    fn systemd_system_unit_is_hardened() {
        let env = vec![(HOME_ENV.to_string(), "/var/lib/cec-support".to_string())];
        let unit = render_systemd_unit(Path::new("/opt/cec/cec-support"), true, &env);
        assert!(unit.contains("DynamicUser=yes"));
        assert!(unit.contains("StateDirectory=cec-support"));
        assert!(unit.contains("WantedBy=multi-user.target"));
        assert!(unit.contains("Environment=CEC_SUPPORT_HOME=/var/lib/cec-support"));
    }

    #[test]
    fn launchd_plist_is_well_formed() {
        let plist = render_launchd_plist(
            Path::new("/Applications/CEC Support.app/cec-support"),
            Path::new("/tmp/agent.log"),
        );
        assert!(plist.contains(&format!("<string>{LAUNCHD_LABEL}</string>")));
        assert!(plist.contains("<string>run</string>"));
        assert!(plist.contains("<string>--service</string>"));
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
    }

    #[test]
    fn windows_bin_path_is_quoted() {
        let bp = windows_bin_path(
            Path::new(r"C:\Program Files\CEC Support\cec-support.exe"),
            Path::new(r"C:\Users\Chris Paul"),
            Path::new(r"C:\Users\Chris Paul\AppData\Roaming\CEC Support"),
            Some("S-1-5-21-1000"),
        );
        assert_eq!(
            bp,
            r#""C:\Program Files\CEC Support\cec-support.exe" run --service --profile-home "C:\Users\Chris Paul" --cec-home "C:\Users\Chris Paul\AppData\Roaming\CEC Support" --client-sid "S-1-5-21-1000" --serve-bin "C:\Program Files\CEC Support\allmystuff-serve.exe" --mesh-bin "C:\Program Files\CEC Support\myownmesh.exe""#
        );
    }

    #[test]
    fn status_value_has_stable_shape() {
        let v = status_value(false).unwrap();
        assert_eq!(v["service_name"], SERVICE_NAME);
        assert!(v.get("platform").is_some());
        assert!(v.get("installed").is_some());
        assert!(v.get("supported").is_some());
    }
}
