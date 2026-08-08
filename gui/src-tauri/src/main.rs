//! CEC Support — the customer-facing client, as one binary.
//!
//! A bare launch opens the calm Tauri GUI (the customer's number → approve/deny
//! flow). The same executable also serves the CLI/service verbs so the
//! installer and the OS service can drive it:
//!
//! ```text
//! cec-support                       # open the app (what the customer runs)
//! cec-support run                   # start the client agent, headless
//! cec-support run --service         # …the headless mode the OS service runs
//! cec-support service install|uninstall|status|start|stop|restart
//! cec-support id                    # print this computer's support number
//! cec-support --version
//! ```
//!
//! Like AllMyStuff's GUI, the Rust side is a **thin client of the per-machine
//! node**: it brings up (or reuses) the *same* `allmystuff-serve` stack an
//! AllMyStuff install runs — one `myownmesh` daemon, one node, one identity
//! per machine; the apps are layered clients of that shared engine, not
//! silos — and drives it over the node control socket. Every
//! `cec_*` Tauri command is one short request; the node's `cec://*` events are
//! re-emitted onto Tauri's bus so the Svelte front-end sees them live.
//!
//! NOTE: this crate does not compile in the CEC Support sandbox — it depends on
//! AllMyStuff's heavy node engine + the CEC crates (git deps on the sibling
//! branch, not yet published). It is written to be correct; the frontend is
//! what this repo validates.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use allmystuff_node::node_control::{ensure_node_running_pinned, NodeChild, NodeClient, NodeEvent};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{Emitter, Manager, RunEvent, State};
use tauri_plugin_autostart::ManagerExt;

/// The AllMyStuff version this build pins its bundled `allmystuff-serve` at,
/// stamped by `build.rs` from `.allmystuff-rev` (e.g. `v0.2.25`). Passed to the
/// node bring-up so a **reused, separately-installed** `allmystuff-serve` CEC
/// doesn't own — e.g. one a co-installed AllMyStuff GUI started on a shared home
/// — is asked to update itself to a version CEC can work with, the same way
/// AllMyStuff keeps a reused `myownmesh` current. `None` (an unpinned dev build)
/// skips the check. CEC's *own* bundled sidecar is already at this pin, so the
/// check is a no-op on a normal install.
const ALLMYSTUFF_PIN: Option<&str> = option_env!("ALLMYSTUFF_PIN");

/// Shared Tauri state: the client's handle to the node, plus the node child we
/// spawned (if any) so it's killed when the app exits. A reused service node
/// has no child here and keeps running.
struct AppState {
    node: Arc<NodeClient>,
    node_child: Mutex<Option<NodeChild>>,
    /// Serialises repair of a stale KVM site mapping. Both the background KVM
    /// refresh and a user action can discover the same dead tunnel at once;
    /// only one of them should tear it down while the other waits and retries.
    kvm_tunnel_repair: tokio::sync::Mutex<()>,
    /// Opt-in "keep running in the background": when set, closing the window
    /// hides to the tray instead of quitting. Off by default — close quits.
    keep_background: std::sync::atomic::AtomicBool,
}

// ---------------------------------------------------------------------------
// CEC environment
// ---------------------------------------------------------------------------

/// CEC Support's own app-file home (service state, logs). The **mesh stack is
/// deliberately not here**: the daemon, the node, their sockets, and the
/// machine identity all live in the shared `~/.myownmesh` home, because CEC is
/// a client of the same per-machine engine AllMyStuff runs — the two apps must
/// address the same stack to ride (or bring up) the same node.
fn default_cec_home() -> PathBuf {
    dirs::data_dir()
        .map(|d| d.join("CEC Support"))
        .or_else(|| dirs::home_dir().map(|h| h.join(".cec-support")))
        .unwrap_or_else(|| PathBuf::from(".cec-support"))
}

/// Prepare the environment for the shared per-machine stack. CEC Support is a
/// *client* of the same engine AllMyStuff runs — one `myownmesh` daemon, one
/// `allmystuff-serve`, shared control sockets, one identity per machine — so
/// this deliberately does **not** fork `MYOWNMESH_HOME`. (It used to, which
/// split identity/state into a CEC-private silo while the Windows control
/// pipes stayed shared: the app then only worked when AllMyStuff had already
/// brought the stack up under the real home.)
///
/// It still clears any inherited `MYOWNMESH_TRYSTERO_APP_ID`: the room handle
/// is `SHA-256(app_id : network_id)`, so a stray override in the shell would
/// fork this daemon's rendezvous space and the dial would silently never
/// connect. CEC always wants the default app-id.
fn apply_cec_env() {
    use allmystuff_cec_protocol::CEC_HOME_ENV;

    // CEC's own app files keep their home; the mesh home is untouched.
    let home = std::env::var_os(CEC_HOME_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(default_cec_home);
    std::env::set_var(CEC_HOME_ENV, &home);
    std::env::remove_var("MYOWNMESH_TRYSTERO_APP_ID");
}

/// Open the on-disk log at `<CEC home>/logs/cec-support.log`, shifting a file
/// past ~5 MB to `.old` first (one generation — history without unbounded
/// growth). `None` (unwritable dir, no home) means stdout-only logging.
/// Relies on [`apply_cec_env`] having resolved the home already.
fn open_log_file() -> Option<std::fs::File> {
    let home = std::env::var_os(allmystuff_cec_protocol::CEC_HOME_ENV).map(PathBuf::from)?;
    let dir = home.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("cec-support.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 5 * 1024 * 1024 {
            let old = dir.join("cec-support.log.old");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&path, &old);
        }
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

// ---------------------------------------------------------------------------
// GUI preferences (`<CEC home>/gui-settings.json`)
// ---------------------------------------------------------------------------

/// When CEC Support registers itself to open with the computer.
#[derive(Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum AutostartMode {
    /// **Default.** Open with Windows only while a technician holds a live
    /// standing grant (3-hour / Forever). This is what lets a technician
    /// restart the machine mid-repair: the customer logs back in, the app
    /// relaunches on its own because the grant is still live, and the tech
    /// carries on — then the login item removes itself once the grant lapses.
    #[default]
    WhileGranted,
    /// Always open with Windows, grant or no grant.
    Always,
    /// Never open with Windows.
    Off,
}

/// The GUI's own tiny preference file. Lives in the CEC app home (not the
/// shared mesh home — these are this app's choices, not the machine's).
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct GuiSettings {
    /// One-shot marker: the run-on-boot default has been applied, or the user
    /// has made an explicit choice — either way, never re-default after this,
    /// so turning autostart off *stays* off across launches.
    #[serde(default)]
    autostart_decided: bool,
    /// Opt-in: closing the window hides to the tray instead of quitting.
    #[serde(default)]
    keep_background: bool,
    /// The autostart policy. `None` = a settings file from before this choice
    /// existed; migrated once at startup (see the setup block) to `Always` for
    /// a user who already had run-on-boot on, else the `WhileGranted` default.
    #[serde(default)]
    autostart_mode: Option<AutostartMode>,
}

fn gui_settings_path() -> Option<PathBuf> {
    std::env::var_os(allmystuff_cec_protocol::CEC_HOME_ENV)
        .map(|h| PathBuf::from(h).join("gui-settings.json"))
}

fn load_gui_settings() -> GuiSettings {
    gui_settings_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_gui_settings(s: &GuiSettings) {
    let Some(path) = gui_settings_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(s) {
        let _ = std::fs::write(&path, bytes);
    }
}

// ---------------------------------------------------------------------------
// CEC node-control commands (the fixed contract the sibling node implements)
// ---------------------------------------------------------------------------

/// `{ number, network_id, role, label }` — the customer's own support
/// identity. `number` is a display label; `network_id` is the shared area.
#[tauri::command]
async fn cec_status(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("cec_status", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// Take up residence on the shared support area so a technician can see and
/// dial this device. Called at bring-up; membership is standing. Returns
/// `{ number }` (the display label to read out).
#[tauri::command]
async fn cec_online(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("cec_online", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// This machine's headline hardware (CPU / RAM / GPUs / disks) off a fresh
/// scan — the front door's spec card.
#[tauri::command]
async fn machine_specs(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("machine_specs", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// Open CEC's TikTok in the system browser — the waiting screen's "catch us
/// live" link. The URL is hardcoded here (not a parameter) so the webview
/// never gains an open-anything primitive.
#[tauri::command]
async fn open_tiktok(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt as _;
    app.shell()
        .open("https://www.tiktok.com/@criticalerrorcomputing", None)
        .map_err(|e| e.to_string())
}

/// Open allmystuff.works in the system browser — the waiting screen's upsell
/// ("CEC Support runs on the AllMyStuff system; get the full app for your own
/// machines"). Same hardcoded-URL discipline as [`open_tiktok`]: the webview
/// gets one named door, never an open-anything primitive.
#[tauri::command]
async fn open_allmystuff_works(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt as _;
    app.shell()
        .open("https://allmystuff.works", None)
        .map_err(|e| e.to_string())
}

/// Open the KVM page in the system browser — the "no KVM connected" line on the
/// KVM card. Same hardcoded-URL discipline as [`open_tiktok`] and
/// [`open_allmystuff_works`]: the webview gets one named door, never an
/// open-anything primitive.
#[tauri::command]
async fn open_kvm_store(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt as _;
    app.shell()
        .open("https://support.cec.direct/#kvms", None)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Toolbox
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolboxKind {
    AdminTerminal(&'static str),
    WindowsProgram(&'static str, &'static [&'static str]),
    ElevatedSidecar(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ToolboxSpec {
    label: &'static str,
    kind: ToolboxKind,
}

fn toolbox_spec(action: &str) -> Option<ToolboxSpec> {
    Some(match action {
        "sfc" => ToolboxSpec {
            label: "System File Checker",
            kind: ToolboxKind::AdminTerminal("sfc.exe /scannow"),
        },
        "dism" => ToolboxSpec {
            label: "Windows image repair",
            kind: ToolboxKind::AdminTerminal("DISM.exe /Online /Cleanup-Image /RestoreHealth"),
        },
        "chkdsk" => ToolboxSpec {
            label: "Online disk scan",
            kind: ToolboxKind::AdminTerminal("chkdsk.exe /scan"),
        },
        "flush_dns" => ToolboxSpec {
            label: "Flush DNS cache",
            kind: ToolboxKind::AdminTerminal("ipconfig.exe /flushdns"),
        },
        "event_viewer" => ToolboxSpec {
            label: "Event Viewer",
            kind: ToolboxKind::WindowsProgram("mmc.exe", &["eventvwr.msc"]),
        },
        "device_manager" => ToolboxSpec {
            label: "Device Manager",
            kind: ToolboxKind::AdminTerminal("Start-Process mmc.exe -ArgumentList 'devmgmt.msc'"),
        },
        "services" => ToolboxSpec {
            label: "Services",
            kind: ToolboxKind::AdminTerminal("Start-Process mmc.exe -ArgumentList 'services.msc'"),
        },
        "system_information" => ToolboxSpec {
            label: "System Information",
            kind: ToolboxKind::WindowsProgram("msinfo32.exe", &[]),
        },
        "task_manager" => ToolboxSpec {
            label: "Task Manager",
            kind: ToolboxKind::WindowsProgram("taskmgr.exe", &[]),
        },
        "control_panel" => ToolboxSpec {
            label: "Control Panel",
            kind: ToolboxKind::WindowsProgram("control.exe", &[]),
        },
        "windows_settings" => ToolboxSpec {
            label: "Windows Settings",
            kind: ToolboxKind::WindowsProgram("explorer.exe", &["ms-settings:"]),
        },
        "crucible_tests" => ToolboxSpec {
            label: "Crucible Tests",
            kind: ToolboxKind::ElevatedSidecar("cec-crucible"),
        },
        "registry_editor" => ToolboxSpec {
            label: "Registry Editor",
            kind: ToolboxKind::AdminTerminal("Start-Process regedit.exe"),
        },
        "disk_management" => ToolboxSpec {
            label: "Disk Management",
            kind: ToolboxKind::AdminTerminal("Start-Process mmc.exe -ArgumentList 'diskmgmt.msc'"),
        },
        "computer_management" => ToolboxSpec {
            label: "Computer Management",
            kind: ToolboxKind::AdminTerminal("Start-Process mmc.exe -ArgumentList 'compmgmt.msc'"),
        },
        "system_configuration" => ToolboxSpec {
            label: "System Configuration",
            kind: ToolboxKind::AdminTerminal("Start-Process msconfig.exe"),
        },
        "windows_features" => ToolboxSpec {
            label: "Windows Features",
            kind: ToolboxKind::AdminTerminal("Start-Process optionalfeatures.exe"),
        },
        "resource_monitor" => ToolboxSpec {
            label: "Resource Monitor",
            kind: ToolboxKind::WindowsProgram("resmon.exe", &[]),
        },
        _ => return None,
    })
}

/// Open (or foreground) the dedicated Toolbox webview. Rust owns window
/// creation so the main webview never gets a create-any-window capability.
#[tauri::command]
async fn open_toolbox(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("toolbox") {
        let _ = window.unminimize();
        window.show().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        "toolbox",
        tauri::WebviewUrl::App("index.html?toolbox=1".into()),
    )
    .title("CEC Support Toolbox")
    .inner_size(1040.0, 760.0)
    .min_inner_size(680.0, 520.0)
    .resizable(true)
    .center()
    .build()
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Run one fixed Toolbox action. No arbitrary command crosses the webview
/// boundary; administrator repairs use AMST's normal attached terminal route.
#[tauri::command]
async fn toolbox_run(
    app: tauri::AppHandle,
    action: String,
    run_id: String,
) -> Result<Value, String> {
    let spec = toolbox_spec(&action).ok_or_else(|| format!("unknown Toolbox action: {action}"))?;
    if !toolbox_run_id_valid(&run_id) {
        return Err("invalid Toolbox run id".into());
    }
    toolbox_run_spec(app, spec, &run_id).await
}

#[cfg(windows)]
async fn toolbox_run_spec(
    app: tauri::AppHandle,
    spec: ToolboxSpec,
    run_id: &str,
) -> Result<Value, String> {
    match spec.kind {
        ToolboxKind::AdminTerminal(command) => run_admin_toolbox(&app, spec, run_id, command).await,
        ToolboxKind::WindowsProgram(program, args) => {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;

            std::process::Command::new(program)
                .args(args)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| format!("opening {}: {e}", spec.label))?;
            Ok(
                json!({ "ok": true, "label": spec.label, "output": format!("{} opened.", spec.label) }),
            )
        }
        ToolboxKind::ElevatedSidecar(base) => {
            let command = admin_sidecar_command(base)?;
            run_admin_toolbox(&app, spec, run_id, &command).await
        }
    }
}

#[cfg(windows)]
async fn run_admin_toolbox(
    app: &tauri::AppHandle,
    spec: ToolboxSpec,
    run_id: &str,
    command: &str,
) -> Result<Value, String> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    let app_exe = std::env::current_exe()
        .map_err(|e| format!("finding the CEC Support installation: {e}"))?;
    let install_dir = app_exe
        .parent()
        .ok_or_else(|| "CEC Support installation has no parent directory".to_string())?;
    let amst = install_dir.join("amst.exe");
    if !amst.is_file() {
        return Err(format!(
            "the bundled AMST terminal is missing from {}",
            amst.display()
        ));
    }

    let progress_dir = std::env::temp_dir().join("cec-support-toolbox");
    std::fs::create_dir_all(&progress_dir)
        .map_err(|e| format!("creating the Toolbox progress folder: {e}"))?;
    let progress_file = progress_dir.join(format!("{run_id}.log"));
    let _ = std::fs::remove_file(&progress_file);

    // Run AMST in a real console so the person at the machine can follow the
    // repair. The PowerShell wrapper mirrors each line to a UTF-8 transcript;
    // we tail that file below so the Toolbox window remains useful too.
    let ps_quote = |value: &str| value.replace('\'', "''");
    let amst = ps_quote(&amst.to_string_lossy());
    let command = ps_quote(command);
    let progress_file_ps = ps_quote(&progress_file.to_string_lossy());
    let script = format!(
        "$ErrorActionPreference='Continue'; \
         $utf8 = New-Object System.Text.UTF8Encoding($false); \
         [Console]::OutputEncoding = $utf8; $OutputEncoding = $utf8; \
         & '{amst}' --admin --run '{command}' 2>&1 | ForEach-Object {{ \
             $line = [string]$_; Write-Host $line; \
             [IO.File]::AppendAllText('{progress_file_ps}', $line + [Environment]::NewLine, $utf8) \
         }}; \
         $cecExit = if ($null -eq $LASTEXITCODE) {{ 0 }} else {{ $LASTEXITCODE }}; \
         if ($cecExit -eq 0) {{ Write-Host ''; Write-Host 'CEC Support task completed.' -ForegroundColor Green }} \
         else {{ Write-Host ''; Write-Host \"CEC Support task failed (exit $cecExit).\" -ForegroundColor Red }}; \
         Start-Sleep -Seconds 2; exit $cecExit"
    );
    let mut child = std::process::Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map_err(|e| format!("starting {} in an AMST terminal: {e}", spec.label))?;

    let mut output = String::new();
    let mut bytes_seen = 0usize;
    let exit_code = loop {
        tail_toolbox_progress(app, run_id, &progress_file, &mut bytes_seen, &mut output);
        match child
            .try_wait()
            .map_err(|e| format!("waiting for {}: {e}", spec.label))?
        {
            Some(status) => break status.code(),
            None => tokio::time::sleep(Duration::from_millis(150)).await,
        }
    };
    tail_toolbox_progress(app, run_id, &progress_file, &mut bytes_seen, &mut output);
    let _ = std::fs::remove_file(&progress_file);

    let output = output.trim().to_string();
    if exit_code != Some(0) {
        let detail = output;
        return Err(if detail.is_empty() {
            format!(
                "{} failed{}",
                spec.label,
                exit_code
                    .map(|code| format!(" (exit {code})"))
                    .unwrap_or_default()
            )
        } else {
            format!("{} failed: {detail}", spec.label)
        });
    }
    Ok(json!({
        "ok": true,
        "label": spec.label,
        "output": if output.is_empty() { format!("{} completed.", spec.label) } else { output },
    }))
}

#[cfg(windows)]
fn tail_toolbox_progress(
    app: &tauri::AppHandle,
    run_id: &str,
    path: &std::path::Path,
    bytes_seen: &mut usize,
    output: &mut String,
) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    if bytes.len() < *bytes_seen {
        *bytes_seen = 0;
    }
    let fresh = &bytes[*bytes_seen..];
    *bytes_seen = bytes.len();
    for raw_line in fresh.split(|byte| matches!(byte, b'\r' | b'\n')) {
        if raw_line.is_empty() {
            continue;
        }
        let line = clean_toolbox_output(raw_line);
        append_toolbox_line(output, &line);
        emit_toolbox_progress(app, run_id, "stdout", &line);
    }
}

#[cfg(windows)]
fn admin_sidecar_command(base: &str) -> Result<String, String> {
    let app_exe = std::env::current_exe()
        .map_err(|e| format!("finding the CEC Support installation: {e}"))?;
    let install_dir = app_exe
        .parent()
        .ok_or_else(|| "CEC Support installation has no parent directory".to_string())?;
    let sidecar = install_dir.join(format!("{base}.exe"));
    if !sidecar.is_file() {
        return Err(format!(
            "the bundled Crucible executable is missing from {}",
            sidecar.display()
        ));
    }

    let work_dir = std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| install_dir.to_path_buf())
        .join("Critical Error Computing")
        .join("Crucible");
    std::fs::create_dir_all(&work_dir)
        .map_err(|e| format!("creating the Crucible report folder: {e}"))?;

    let sidecar = sidecar.to_string_lossy().replace('\'', "''");
    let work_dir = work_dir.to_string_lossy().replace('\'', "''");
    Ok(format!(
        "Start-Process -FilePath '{sidecar}' -WorkingDirectory '{work_dir}'"
    ))
}

#[cfg(not(windows))]
async fn toolbox_run_spec(
    _app: tauri::AppHandle,
    _spec: ToolboxSpec,
    _run_id: &str,
) -> Result<Value, String> {
    Err("The repair Toolbox is currently available on Windows.".into())
}

fn toolbox_run_id_valid(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id.len() <= 96
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn append_toolbox_line(output: &mut String, line: &str) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(line.trim_end_matches(['\r', '\n']));
}

/// AMST carries a real PTY, so its output can contain terminal title and color
/// sequences. Keep the useful text/progress while preventing raw escape codes
/// from leaking into the Toolbox progress cards.
fn clean_toolbox_output(input: &[u8]) -> String {
    let mut clean = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            0x1b if input.get(index + 1) == Some(&b'[') => {
                index += 2;
                while index < input.len() {
                    let byte = input[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            0x1b if input.get(index + 1) == Some(&b']') => {
                index += 2;
                while index < input.len() {
                    if input[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            0x1b => index = (index + 2).min(input.len()),
            byte if byte == b'\t' || byte >= 0x20 => {
                clean.push(byte);
                index += 1;
            }
            _ => index += 1,
        }
    }
    String::from_utf8_lossy(&clean).into_owned()
}

fn emit_toolbox_progress(app: &tauri::AppHandle, run_id: &str, stream: &str, line: &str) {
    let chunk = line.trim_end_matches(['\r', '\n']);
    if chunk.is_empty() {
        return;
    }
    let _ = app.emit(
        "toolbox://progress",
        json!({ "runId": run_id, "stream": stream, "chunk": chunk }),
    );
}

/// Temps alone — the sensor read without the full scan, cheap enough for the
/// spec card to poll so its one moving number actually moves.
#[tauri::command]
async fn machine_temps(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("machine_temps", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// Raise (or withdraw) the ask on the global help room. While on, this node
/// beacons "I need help" to every CEC technician; a technician answers by
/// dialing our own number room, so the normal approval still gates everything.
#[tauri::command]
async fn cec_ask_help(state: State<'_, AppState>, on: bool) -> Result<Value, String> {
    state
        .node
        .request("cec_ask_help", json!({ "on": on }))
        .await
        .map_err(|e| e.to_string())
}

/// The technician requests currently awaiting a decision (drives the modal).
#[tauri::command]
async fn cec_pending(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("cec_pending", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// Approve a technician's dial with one of the three scopes. `scope` is
/// validated against the shared [`ApprovalScope`] before being forwarded, so a
/// malformed scope is rejected here rather than on the node.
///
/// [`ApprovalScope`]: allmystuff_cec_protocol::ApprovalScope
#[tauri::command]
async fn cec_approve(
    state: State<'_, AppState>,
    tech: String,
    scope: String,
    session_id: String,
    want_control: bool,
) -> Result<(), String> {
    let canonical = canonical_scope(&scope)?;
    state
        .node
        .request(
            "cec_approve",
            json!({
                "tech": tech,
                "scope": canonical,
                "session_id": session_id,
                "want_control": want_control,
            }),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Decline a technician's dial, or end the current session (leaving any
/// standing grant intact — that's what revoke is for).
#[tauri::command]
async fn cec_deny(
    state: State<'_, AppState>,
    tech: String,
    session_id: String,
) -> Result<(), String> {
    state
        .node
        .request(
            "cec_deny",
            json!({ "tech": tech, "session_id": session_id }),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Revoke a technician's standing approval — "Forget this technician". Bites
/// immediately, mid-session.
#[tauri::command]
async fn cec_revoke(state: State<'_, AppState>, tech: String) -> Result<(), String> {
    state
        .node
        .request("cec_revoke", json!({ "tech": tech }))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Forget a technician's node from the mesh entirely (drops the peer, not just
/// the grant). Paired with [`cec_revoke`] in the Forget flow.
#[tauri::command]
async fn cec_forget_node(state: State<'_, AppState>, node: String) -> Result<(), String> {
    state
        .node
        .request("cec_forget_node", json!({ "node": node }))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The customer's standing approvals. The node's `cec_grants` is a GUI-facing
/// **projection** (flat `scope` string + `control` bool), not the internal
/// consent-store `Grant` (tagged scope + `capabilities`), so it's validated
/// against that projection shape here — a drift is logged rather than
/// mis-rendered. (The old check parsed against the store's `Grant` and so
/// warned on every call even when nothing was wrong — the two shapes can never
/// match.)
#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct GrantView {
    technician: String,
    #[serde(default)]
    agent_name: String,
    scope: String,
    granted_at: u64,
    #[serde(default)]
    expires_at: Option<u64>,
    control: bool,
}

#[tauri::command]
async fn cec_grants(state: State<'_, AppState>) -> Result<Value, String> {
    let v = state
        .node
        .request("cec_grants", json!({}))
        .await
        .map_err(|e| e.to_string())?;
    if serde_json::from_value::<Vec<GrantView>>(v.clone()).is_err() {
        tracing::warn!("cec_grants: node returned grants that don't match the expected GUI shape");
    }
    Ok(v)
}

/// What each connected technician is actually doing right now — the node's
/// `cec_viewing` projection `{ techs: { <tech>: { screen, control } } }`,
/// derived from live routes rather than session state. The pull half of the
/// `cec://viewing` event (which the generic pump already relays), so the
/// access list paints the chip correctly on a mid-session app start.
#[tauri::command]
async fn cec_viewing(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("cec_viewing", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// Set this computer's friendly name (shown to the technician on the mesh). A
/// convenience beyond the core dial/approve contract.
#[tauri::command]
async fn cec_set_label(state: State<'_, AppState>, label: String) -> Result<(), String> {
    state
        .node
        .request("cec_set_label", json!({ "label": label }))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Send one live chat line to the connected technician (`peer` is their device
/// id). Returns the node-assigned `{ id, ts }`. The same `cec_chat_send` the
/// technician side calls — the node keys the transcript by peer and attributes
/// `from` by which side dialed, so on this (customer) node our own lines come
/// back as `"client"`.
#[tauri::command]
async fn cec_chat_send(
    state: State<'_, AppState>,
    peer: String,
    text: String,
) -> Result<Value, String> {
    state
        .node
        .request("cec_chat_send", json!({ "peer": peer, "text": text }))
        .await
        .map_err(|e| e.to_string())
}

/// The persisted chat transcript with `peer`, oldest-first, as
/// `{ messages: [ { id, from, text, ts } ] }` — what the chat panel loads when
/// it opens.
#[tauri::command]
async fn cec_chat_history(state: State<'_, AppState>, peer: String) -> Result<Value, String> {
    state
        .node
        .request("cec_chat_history", json!({ "peer": peer }))
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// KVM & claiming — thin pass-throughs to the node's generic (non-`cec_`)
// control surface. These commands already exist on the node the AllMyStuff
// app drives; the customer app reaches the same ones over the same socket so
// a customer can adopt and manage a CEC KVM plugged into their machine. No new
// node code — the node is the source of truth, we only forward.
// ---------------------------------------------------------------------------

/// The node's live mesh snapshot — `{ ready, me, peers: [ { node, label,
/// owner, claimable, features, sites, kvm { attached_to, web, joining_mesh,
/// meshes } } ] }`. `me` is this computer's own mesh id (the attach-to-this-
/// computer target); the peers carry the claim/KVM adverts the KVM & Claiming
/// card discovers claimable CEC KVMs from. Generic, not CEC-specific, but the
/// card is its only consumer here.
#[tauri::command]
async fn session_snapshot(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("session_snapshot", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// Adopt a claimable device (a CEC KVM) — records this node as its owner. The
/// device confirms by re-advertising its new owner in presence, which the next
/// snapshot reflects. `node` is the KVM's mesh id.
#[tauri::command]
async fn claim_node(state: State<'_, AppState>, node: String) -> Result<Value, String> {
    state
        .node
        .request("claim_node", json!({ "node": node }))
        .await
        .map_err(|e| e.to_string())
}

/// Point a claimed KVM at the machine it controls. `target` is this computer's
/// own node id (from `session_snapshot.me`) — i.e. "this KVM is attached to
/// this computer". The KVM confirms by re-advertising `kvm.attached_to`.
#[tauri::command]
async fn kvm_attach(
    state: State<'_, AppState>,
    node: String,
    target: String,
) -> Result<Value, String> {
    state
        .node
        .request("kvm_attach", json!({ "node": node, "target": target }))
        .await
        .map_err(|e| e.to_string())
}

/// What Wi-Fi networks THIS computer can see, and which one it's on.
///
/// The KVM is the thing that needs to join a network, and it is the one thing
/// that can't help: with no uplink it can't be asked what's nearby, and only a
/// Pro has a scan endpoint at all — on a plain NanoKVM the picker has never had
/// anything to show, so the name had to be typed from memory. That is where it
/// goes wrong: the 2.4 and 5 GHz variants of one network, a trailing space, a
/// name that isn't quite what's printed on the router.
///
/// The host is in the same room on the same radio and already knows. `current`
/// — the network this computer is itself on — is the most useful field of the
/// lot, because it is nearly always the one the KVM should join.
///
/// Nothing here is privileged, so it can't provoke a UAC or keychain prompt on
/// a customer's machine. It reads no password: that IS privileged everywhere
/// (Administrator on Windows, a keychain prompt on macOS, root for
/// NetworkManager), so the customer still types their own — into a form that no
/// longer also asks them to remember the network's exact name.
///
/// Never fails. A platform that can't answer says so in `note`, and the manual
/// field the panel has always had carries the flow.
#[tauri::command]
fn host_wifi_scan() -> Value {
    serde_json::to_value(cec_support_wifi::scan()).unwrap_or(Value::Null)
}

/// Map a peer's exposed site (here, a KVM's own web UI) to a local port,
/// returning `{ localPort }`. The reboot flow maps the KVM's console then
/// POSTs its NanoKVM GPIO endpoint at `http://localhost:<localPort>` over the
/// tunnel (auth is bypassed on the mesh path).
#[tauri::command]
async fn site_map(state: State<'_, AppState>, node: String, port: u16) -> Result<Value, String> {
    state
        .node
        .request("site_map", json!({ "node": node, "port": port }))
        .await
        .map_err(|e| e.to_string())
}

/// One JSON call to a claimed KVM's own web API, over the site tunnel already
/// mapped at `http://localhost:<port>` by [`site_map`].
///
/// This deliberately does NOT live in the webview. The front-end runs on the
/// app's own origin while the tunnel answers on `http://localhost:<port>`, so
/// every such call is cross-origin — and the appliance only sends CORS headers
/// when its *own* auth is disabled (NanoKVM `server/main.go`:
/// `if conf.Authentication == "disable"`), while the tunnel itself is a raw TCP
/// proxy that adds none. A browser `fetch` is therefore blocked both ways: a
/// plain GET reaches the device but its response is unreadable, and any
/// `content-type: application/json` POST (the reboot, the Wi-Fi write) dies at
/// a preflight `OPTIONS` that gin has no route for, never reaching the device
/// at all. Both surface as an opaque `TypeError: Failed to fetch` carrying no
/// status — which is exactly why the Wi-Fi panel could only ever say "there was
/// an error". Rust has no origin and no preflight, so the call simply works,
/// and the real status and body reach the UI.
///
/// No KVM token is needed on this path: the request arrives over the mesh, and
/// the appliance treats mesh-tunneled requests as authenticated by roster
/// membership (`middleware.WithMeshAuth` → `allowByToken`).
///
/// Returns `{ status, body, error }`. `body` is the parsed `{ code, msg, data }`
/// envelope when the device sent JSON and null otherwise (a plain NanoKVM
/// answers an unknown route with HTML). A transport failure — refused, timed
/// out, tunnel gone — comes back as `error: { kind, message }` rather than an
/// `Err`, so the caller can tell a *timeout* apart from a hard failure: writing
/// Wi-Fi credentials legitimately times out when the KVM moves onto the new
/// network and the tunnel drops mid-write, and that must not read as an error.
#[tauri::command]
async fn kvm_api(
    state: State<'_, AppState>,
    port: u16,
    path: String,
    method: Option<String>,
    body: Option<Value>,
    timeout_ms: Option<u64>,
) -> Result<Value, String> {
    // Keep this addressed at the loopback tunnel and nowhere else. A path that
    // doesn't start with '/' can move the host entirely — "@example.com/x"
    // would make the authority parse as userinfo `127.0.0.1:<port>` against
    // host example.com — so anything else is refused rather than sent.
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(format!("invalid device path {path}"));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms.unwrap_or(12_000)))
        // The tunnel is a loopback port; never let a system proxy intercept it.
        .no_proxy()
        // The device's JSON API never redirects, so following one could only
        // take us somewhere we didn't mean to go. A 3xx is reported as-is.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("couldn't start the request: {e}"))?;

    let verb = method.as_deref().unwrap_or("GET").to_ascii_uppercase();
    if !matches!(verb.as_str(), "GET" | "POST" | "DELETE") {
        return Err(format!("unsupported method {verb}"));
    }

    let first = kvm_api_once(&client, port, &path, &verb, body.as_ref()).await;
    // A state-changing request may have reached the appliance even when its
    // reply was lost, so never replay POST/DELETE. GET is safe to heal and
    // retry. HTTP errors are real device replies and likewise do not imply a
    // dead tunnel; only a transport-level failure takes this path.
    if !should_repair_kvm_tunnel(&verb, &first) {
        return Ok(first);
    }

    // The shared node can outlive CEC Support when AllMyStuff owns it. A KVM
    // route may then remain locally "active" after its far side disappeared,
    // and `site_map` quite correctly hands both apps the existing listener --
    // a listener that accepts TCP and immediately closes it. Repair only after
    // the GET above proved that condition. If another caller is already doing
    // so, wait and first try the route it just repaired.
    let repair_guard = match state.kvm_tunnel_repair.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            let guard = state.kvm_tunnel_repair.lock().await;
            let after_wait = kvm_api_once(&client, port, &path, &verb, body.as_ref()).await;
            if !kvm_transport_failed(&after_wait) {
                return Ok(after_wait);
            }
            guard
        }
    };

    let repaired_port = match remap_site_for_local_port(&state.node, port).await {
        Ok(Some(repaired_port)) => repaired_port,
        Ok(None) => {
            tracing::warn!(
                "KVM tunnel on :{port} failed, but the shared node no longer lists that mapping"
            );
            drop(repair_guard);
            return Ok(first);
        }
        Err(e) => {
            tracing::warn!("couldn't repair stale KVM tunnel on :{port}: {e}");
            drop(repair_guard);
            return Ok(first);
        }
    };
    let repaired = kvm_api_once(&client, repaired_port, &path, &verb, body.as_ref()).await;
    drop(repair_guard);
    Ok(repaired)
}

/// Perform one HTTP request through an already-mapped KVM site tunnel.
/// Transport failures are values rather than Rust errors because the UI needs
/// to distinguish a timeout from a refused/closed tunnel.
async fn kvm_api_once(
    client: &reqwest::Client,
    port: u16,
    path: &str,
    verb: &str,
    body: Option<&Value>,
) -> Value {
    let url = format!("http://127.0.0.1:{port}{path}");
    let mut req = match verb {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "DELETE" => client.delete(&url),
        _ => unreachable!("verb validated by kvm_api"),
    };
    if let Some(body) = body {
        req = req.json(body);
    }

    let res = match req.send().await {
        Ok(res) => res,
        Err(e) => {
            let (kind, message) = if e.is_timeout() {
                ("timeout", "the KVM didn't answer in time".to_string())
            } else if e.is_connect() {
                (
                    "connect",
                    "couldn't reach the KVM's console tunnel".to_string(),
                )
            } else {
                ("other", format!("couldn't reach the KVM: {e}"))
            };
            return json!({
                "status": 0,
                "body": Value::Null,
                "error": { "kind": kind, "message": message },
                "localPort": port,
            });
        }
    };

    let status = res.status().as_u16();
    // Read the body even on a non-2xx: the device's own `{ code, msg }` is
    // usually more useful to show than the bare status.
    let text = res.text().await.unwrap_or_default();
    json!({
        "status": status,
        "body": serde_json::from_str::<Value>(&text).ok(),
        "error": Value::Null,
        "localPort": port,
    })
}

fn kvm_transport_failed(out: &Value) -> bool {
    out.get("error").is_some_and(|error| !error.is_null())
}

fn should_repair_kvm_tunnel(verb: &str, out: &Value) -> bool {
    verb == "GET" && kvm_transport_failed(out)
}

/// Find the shared node mapping behind `local_port`, tear down that one route,
/// and recreate it. The node normally preserves the same loopback port; the
/// returned value handles the legitimate case where the OS forces a new one.
async fn remap_site_for_local_port(
    node: &NodeClient,
    local_port: u16,
) -> Result<Option<u16>, String> {
    let mappings = node
        .request("site_mappings", json!({}))
        .await
        .map_err(|e| e.to_string())?;
    let Some((peer, host_port)) = site_mapping_for_local_port(&mappings, local_port)? else {
        return Ok(None);
    };

    node.request("site_unmap", json!({ "node": peer, "port": host_port }))
        .await
        .map_err(|e| e.to_string())?;
    let mapped = node
        .request("site_map", json!({ "node": peer, "port": host_port }))
        .await
        .map_err(|e| e.to_string())?;
    let repaired_port = mapped
        .get("localPort")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| "site_map returned no valid localPort".to_string())?;
    tracing::info!("recreated stale KVM tunnel :{local_port} as :{repaired_port}");
    Ok(Some(repaired_port))
}

fn site_mapping_for_local_port(
    mappings: &Value,
    local_port: u16,
) -> Result<Option<(String, u16)>, String> {
    let rows = mappings
        .as_array()
        .ok_or_else(|| "site_mappings returned a non-array result".to_string())?;
    let Some(row) = rows
        .iter()
        .find(|row| row.get("localPort").and_then(Value::as_u64) == Some(u64::from(local_port)))
    else {
        return Ok(None);
    };
    let peer = row
        .get("node")
        .and_then(Value::as_str)
        .filter(|node| !node.is_empty())
        .ok_or_else(|| "site mapping has no node".to_string())?;
    let host_port = row
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| "site mapping has no valid host port".to_string())?;
    Ok(Some((peer.to_string(), host_port)))
}

/// Open a KVM's own web UI in the system browser — its LAN address (the
/// ethernet or Wi-Fi IP the device reported) or the loopback port of the site
/// tunnel this app already holds open.
///
/// Takes a host and port rather than a URL, and refuses anything that isn't a
/// literal private or loopback IP, so this stays a named door rather than the
/// open-anything primitive [`open_tiktok`] and friends deliberately avoid: the
/// webview cannot use it to reach the public internet, and a hostname (which
/// could resolve anywhere) is never accepted.
#[tauri::command]
async fn open_kvm_console(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    scheme: String,
) -> Result<(), String> {
    use std::net::IpAddr;
    use tauri_plugin_shell::ShellExt as _;

    let ip: IpAddr = host
        .parse()
        .map_err(|_| format!("{host} isn't an IP address"))?;
    let local = match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        // A KVM on IPv6 is reachable at a link-local or unique-local address.
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    };
    if !local {
        return Err(format!("{host} isn't a local address"));
    }
    if scheme != "http" && scheme != "https" {
        return Err(format!("unsupported scheme {scheme}"));
    }
    // Bracket a literal IPv6 host so the URL parses.
    let authority = if ip.is_ipv6() {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    app.shell()
        .open(format!("{scheme}://{authority}"), None)
        .map_err(|e| e.to_string())
}

/// Unclaim a KVM we own — releases our ownership so the appliance resets to its
/// own joining mesh and offers itself for adoption again. Claiming a KVM makes
/// the customer its fleet owner, so `fleet_kick` (the eviction + Release) is the
/// path; a customer's fleet carries no MFA, so no `code` is sent.
#[tauri::command]
async fn fleet_kick(state: State<'_, AppState>, device: String) -> Result<Value, String> {
    state
        .node
        .request("fleet_kick", json!({ "device": device }))
        .await
        .map_err(|e| e.to_string())
}

/// The node's networks (`{ networks: [ { network_id, … } ] }`). The KVM card
/// enumerates these to find where a KVM lives so it can check live
/// reachability — the presence snapshot remembers a KVM's last advert even
/// after it powers off, so a separate liveness read is what drops it.
#[tauri::command]
async fn mesh_networks(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("mesh_networks", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// The live peer list on `network` (`{ peers: [ { device_id, status, … } ] }`).
/// `status` is the reachability signal — `active`/`shelved` when the node can
/// actually reach the peer, a remembered-but-offline value otherwise — which
/// the KVM card uses to drop KVMs that have gone offline.
#[tauri::command]
async fn mesh_peers(state: State<'_, AppState>, network: String) -> Result<Value, String> {
    state
        .node
        .request("mesh_peers", json!({ "network": network }))
        .await
        .map_err(|e| e.to_string())
}

/// Validate a UI scope string and return its canonical wire form. Uses the
/// shared [`ApprovalScope`](allmystuff_cec_protocol::ApprovalScope) as the
/// source of truth for the three allowed values.
fn canonical_scope(scope: &str) -> Result<&'static str, String> {
    use allmystuff_cec_protocol::ApprovalScope;
    let parsed = match scope {
        "once" => ApprovalScope::Once,
        "three_hours" => ApprovalScope::ThreeHours,
        "forever" => ApprovalScope::Forever,
        other => return Err(format!("unknown approval scope: {other}")),
    };
    Ok(match parsed {
        ApprovalScope::Once => "once",
        ApprovalScope::ThreeHours => "three_hours",
        ApprovalScope::Forever => "forever",
    })
}

// ---------------------------------------------------------------------------
// Background service (the local cec-support-service crate — never the node)
// ---------------------------------------------------------------------------

/// The OS background-service status as JSON.
#[tauri::command]
async fn service_status() -> Result<Value, String> {
    tokio::task::spawn_blocking(|| {
        cec_support_service::status_value(false)
            .unwrap_or_else(|_| json!({ "platform": std::env::consts::OS, "supported": false }))
    })
    .await
    .map_err(|e| format!("service status task failed: {e}"))
}

/// Map a UI verb to the service crate's command.
fn service_cmd(verb: &str) -> Option<cec_support_service::ServiceCmd> {
    use cec_support_service::ServiceCmd;
    Some(match verb {
        "install" => ServiceCmd::Install { log: None },
        "start" => ServiceCmd::Start,
        "stop" => ServiceCmd::Stop,
        "restart" => ServiceCmd::Restart,
        "uninstall" => ServiceCmd::Uninstall,
        _ => return None,
    })
}

/// The verb after a `--service-do` flag in argv (the elevated Windows
/// self-invocation; see [`main`]).
fn service_do_verb() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--service-do")?;
    args.get(i + 1).cloned()
}

fn configure_service_environment() {
    if let Some(profile) = process_arg_value("--profile-home") {
        std::env::set_var("MYOWNMESH_HOME", &profile);
        std::env::set_var("ALLMYSTUFF_USER_HOME", &profile);
    }
    if let Some(cec_home) = process_arg_value("--cec-home") {
        std::env::set_var(allmystuff_cec_protocol::CEC_HOME_ENV, cec_home);
    }
    if let Some(sid) = process_arg_value("--client-sid") {
        std::env::set_var("ALLMYSTUFF_CLIENT_SID", sid);
    }
    if let Some(serve_bin) = process_arg_value("--serve-bin") {
        std::env::set_var("ALLMYSTUFF_SERVE_BIN", serve_bin);
    }
    if let Some(mesh_bin) = process_arg_value("--mesh-bin") {
        std::env::set_var("MYOWNMESH_BIN", mesh_bin);
    }
}

fn process_arg_value(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).cloned()
}

async fn service_mutate(verb: &'static str) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || service_mutate_blocking(verb))
        .await
        .map_err(|e| format!("service {verb} task failed: {e}"))?
}

/// Unix: install/manage the per-user service in-process — no privilege, no CLI.
#[cfg(not(windows))]
fn service_mutate_blocking(verb: &str) -> Result<Value, String> {
    let cmd = service_cmd(verb).ok_or_else(|| format!("unknown service action: {verb}"))?;
    match cec_support_service::run(false, cmd) {
        Ok(()) => Ok(json!({ "ok": true, "output": format!("service {verb}: done") })),
        Err(e) => Ok(json!({ "ok": false, "output": format!("{e:#}") })),
    }
}

/// Windows: a service needs admin, so re-launch our own binary elevated to do
/// the work (`--service-do <verb>`, handled in [`main`]). Still no external CLI.
#[cfg(windows)]
fn service_mutate_blocking(verb: &str) -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|e| format!("locating CEC Support: {e}"))?;
    let exe = exe.to_string_lossy().replace('\'', "''");
    let profile = dirs::home_dir()
        .ok_or_else(|| "couldn't resolve the current Windows profile".to_string())?
        .to_string_lossy()
        .replace('\'', "''");
    let cec_home = default_cec_home().to_string_lossy().replace('\'', "''");
    let sid = current_windows_user_sid()?.replace('\'', "''");
    let elevated_args = format!(
        "--service-do {verb} --service-profile \"{profile}\" --service-home \"{cec_home}\" --service-sid {sid}"
    )
    .replace('\'', "''");
    let ps = format!(
        "try {{ $p = Start-Process -FilePath '{exe}' -ArgumentList '{elevated_args}' \
         -Verb RunAs -Wait -PassThru -WindowStyle Hidden; exit $p.ExitCode }} \
         catch {{ exit 1223 }}"
    );
    use std::os::windows::process::CommandExt as _;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("launching elevated CEC Support: {e}"))?;
    let code = out.status.code().unwrap_or(-1);
    if code == 1223 {
        return Err("Administrator approval was declined.".to_string());
    }
    Ok(json!({
        "ok": code == 0,
        "output": if code == 0 {
            format!("service {verb}: done")
        } else {
            format!("service {verb} failed (exit {code})")
        },
    }))
}

#[cfg(windows)]
fn current_windows_user_sid() -> Result<String, String> {
    use std::os::windows::process::CommandExt as _;
    let output = std::process::Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("reading the current Windows account SID: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split([',', '"', '\r', '\n'])
        .map(str::trim)
        .find(|part| part.starts_with("S-1-"))
        .map(str::to_string)
        .ok_or_else(|| "Windows returned no account SID".to_string())
}

#[tauri::command]
async fn service_install(state: State<'_, AppState>) -> Result<Value, String> {
    // An upgraded install may still have a GUI-owned node on the pipe. Release
    // it before the elevated installer starts the replacement service; doing
    // this afterwards lets both processes race to become the machine node.
    state.node_child.lock().take();
    let result = service_mutate("install").await;
    let installed = result
        .as_ref()
        .ok()
        .and_then(|value| value.get("ok"))
        .and_then(Value::as_bool)
        == Some(true);
    if installed && !wait_for_node().await {
        tracing::warn!("installed CEC host did not become ready within the startup grace window");
    }
    if !installed || !NodeClient::probe().await {
        // UAC cancellation or a failed replacement must not strand a machine
        // that was working through the temporary GUI-owned node. The same
        // fallback covers a service that installs but never becomes ready.
        if let Ok(Some(child)) = ensure_node_running_pinned(ALLMYSTUFF_PIN).await {
            state.node_child.lock().replace(child);
        }
    }
    result
}
#[tauri::command]
async fn service_uninstall() -> Result<Value, String> {
    service_mutate("uninstall").await
}
#[tauri::command]
async fn service_start() -> Result<Value, String> {
    service_mutate("start").await
}
#[tauri::command]
async fn service_stop() -> Result<Value, String> {
    service_mutate("stop").await
}
#[tauri::command]
async fn service_restart() -> Result<Value, String> {
    service_mutate("restart").await
}

/// Whether "Open CEC Support when this computer starts" is registered.
#[tauri::command]
fn autostart_get(app: tauri::AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Register / unregister the login item, returning the resulting state.
#[tauri::command]
fn autostart_set(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable().map_err(|e| e.to_string())?;
    } else {
        mgr.disable().map_err(|e| e.to_string())?;
    }
    // An explicit choice — the run-on-boot default must never re-apply over
    // it on a later launch.
    let mut s = load_gui_settings();
    if !s.autostart_decided {
        s.autostart_decided = true;
        save_gui_settings(&s);
    }
    Ok(mgr.is_enabled().unwrap_or(enabled))
}

/// The current autostart policy (`while_granted` / `always` / `off`).
#[tauri::command]
fn autostart_mode_get() -> String {
    let mode = load_gui_settings().autostart_mode.unwrap_or_default();
    autostart_mode_word(mode).to_string()
}

/// Set the autostart policy and apply it right away.
#[tauri::command]
async fn autostart_mode_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    let mode = match mode.as_str() {
        "while_granted" => AutostartMode::WhileGranted,
        "always" => AutostartMode::Always,
        "off" => AutostartMode::Off,
        other => return Err(format!("unknown autostart mode {other:?}")),
    };
    let mut s = load_gui_settings();
    s.autostart_mode = Some(mode);
    s.autostart_decided = true;
    save_gui_settings(&s);
    reconcile_autostart(&app, &state.node).await;
    Ok(())
}

fn autostart_mode_word(mode: AutostartMode) -> &'static str {
    match mode {
        AutostartMode::WhileGranted => "while_granted",
        AutostartMode::Always => "always",
        AutostartMode::Off => "off",
    }
}

/// Enable or disable the OS login item; best-effort (a failure is logged, not
/// fatal — at worst the app doesn't open on boot, which only costs a manual
/// relaunch).
fn set_login_item(app: &tauri::AppHandle, enable: bool) {
    let mgr = app.autolaunch();
    let now = mgr.is_enabled().unwrap_or(false);
    if now == enable {
        return;
    }
    let r = if enable { mgr.enable() } else { mgr.disable() };
    if let Err(e) = r {
        tracing::warn!(
            "couldn't {} run-on-boot: {e}",
            if enable { "enable" } else { "disable" }
        );
    }
}

/// Whether the customer currently holds a **live standing** grant (3-hour or
/// Forever). `cec_grants` already returns only live grants, so any non-`once`
/// scope here means a technician can still reconnect — the condition that keeps
/// grant-scoped autostart armed.
async fn has_live_standing_grant(node: &NodeClient) -> bool {
    let Ok(v) = node.request("cec_grants", json!({})).await else {
        return false;
    };
    v.as_array()
        .map(|arr| {
            arr.iter().any(|g| {
                matches!(
                    g.get("scope").and_then(|s| s.as_str()),
                    Some("three_hours") | Some("forever")
                )
            })
        })
        .unwrap_or(false)
}

/// Bring the OS login item in line with the chosen policy: `Always` on, `Off`
/// off, `WhileGranted` on exactly while a live standing grant exists. Called at
/// startup, whenever the mode changes, and on a periodic sweep (grants expire
/// by the clock, with no event to hook).
async fn reconcile_autostart(app: &tauri::AppHandle, node: &NodeClient) {
    let mode = load_gui_settings().autostart_mode.unwrap_or_default();
    let enable = match mode {
        AutostartMode::Always => true,
        AutostartMode::Off => false,
        AutostartMode::WhileGranted => has_live_standing_grant(node).await,
    };
    set_login_item(app, enable);
}

/// Whether "keep running in the background" is on — closing the window then
/// hides to the tray instead of quitting. Off by default: close means close.
#[tauri::command]
fn background_get(state: State<'_, AppState>) -> bool {
    state
        .keep_background
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// Flip the keep-running-in-background option and persist it.
#[tauri::command]
fn background_set(state: State<'_, AppState>, enabled: bool) -> bool {
    state
        .keep_background
        .store(enabled, std::sync::atomic::Ordering::Relaxed);
    let mut s = load_gui_settings();
    s.keep_background = enabled;
    save_gui_settings(&s);
    enabled
}

// ---------------------------------------------------------------------------
// Self-update (CEC Support's own release feed, not the node's)
// ---------------------------------------------------------------------------

/// Current updater state: running version, install kind, prefs, what's staged.
#[tauri::command]
async fn update_status() -> Result<Value, String> {
    serde_json::to_value(cec_support_updater::status().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Check the release feed right now, ignoring the interval cooldown, and stage
/// anything the apply policy permits.
#[tauri::command]
async fn update_check() -> Result<Value, String> {
    let outcome = cec_support_updater::check_now(true)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(outcome).map_err(|e| e.to_string())
}

/// Apply a staged update to disk. The swap lands immediately, but this process
/// keeps the old build until it restarts.
#[tauri::command]
async fn update_apply() -> Result<Value, String> {
    let applied = cec_support_updater::apply_now().map_err(|e| e.to_string())?;
    Ok(json!({ "applied": applied }))
}

/// Apply a staged update and relaunch into it. Applying *before* the restart is
/// what makes the relaunch land on the new version in one step: a bare restart
/// would re-exec the still-old binary and only swap it in on the following
/// boot. Never returns on success — the process restarts.
#[tauri::command]
async fn update_relaunch(app: tauri::AppHandle) -> Result<(), String> {
    cec_support_updater::apply_now().map_err(|e| e.to_string())?;
    app.restart()
}

/// Change updater preferences (auto-update on/off, channel, policy, interval).
#[tauri::command]
async fn update_set_prefs(prefs: Value) -> Result<Value, String> {
    let prefs: cec_support_updater::UpdatePrefs =
        serde_json::from_value(prefs).map_err(|e| e.to_string())?;
    serde_json::to_value(cec_support_updater::set_prefs(prefs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// GUI plumbing
// ---------------------------------------------------------------------------

/// Bring the main window to the front (single-instance hand-off, tray click).
fn reveal_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Build the tray icon so CEC Support stays reachable while it waits in the
/// background for a technician. Left-click / "Show" reveals the window.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItemBuilder::with_id("show", "Show CEC Support").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("CEC Support")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => reveal_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Subscribe to the node's event stream and re-emit each event on Tauri's bus,
/// so the Svelte front-end sees the `cec://*` events live. Reconnects if the
/// node restarts — and if the node is *gone* (the app that spawned it exited,
/// taking the kill-on-close serve with it), brings the shared stack back up
/// itself and re-hosts, so this app keeps working solo or side by side.
/// Bring the shared stack back up and re-join the support area — the pump's
/// respawn body. A fresh serve isn't on the area, and `cec_online` is
/// idempotent, so re-joining is always safe.
async fn respawn_and_rehost(app: &tauri::AppHandle, node: &NodeClient) {
    match ensure_node_running_pinned(ALLMYSTUFF_PIN).await {
        Ok(Some(child)) => {
            app.state::<AppState>().node_child.lock().replace(child);
        }
        Ok(None) => {}
        Err(e) => tracing::warn!("couldn't bring the node back up: {e:#}"),
    }
    wait_for_node().await;
    if let Err(e) = node.request("cec_online", json!({})).await {
        tracing::warn!("cec_online after node respawn failed: {e:#}");
    }
}

async fn run_event_pump(app: tauri::AppHandle, node: Arc<NodeClient>) {
    use tokio::sync::mpsc;
    // A node reconnect can replay a still-pending permission request. Raise the
    // customer window for the first event of each session, never on every
    // replay/poll while they are deciding.
    let mut foregrounded_requests = HashSet::<String>::new();
    // Consecutive grace windows the socket stayed dead while OUR child kept
    // running — the wedged-not-gone state. Only a repeat offender earns a
    // deliberate, owner-controlled restart.
    let mut wedged_rounds: u32 = 0;
    loop {
        // A client doesn't require whichever app spawned the engine: nothing
        // answering the socket means it's our turn to bring the stack up.
        // Probe with patience first — a serve that is *starting* (spawned,
        // socket not bound yet) must not read as "gone": respawning over it
        // would kill-on-drop the very child being waited on, and the stack
        // would flap spawn/kill forever. Only a socket that stays dead across
        // the whole grace window is really gone.
        if !NodeClient::probe().await {
            wait_for_node().await;
        }
        if !NodeClient::probe().await {
            // Dead socket through the grace window — but if OUR child is still
            // running, the serve is alive behind a busy/wedged socket, not
            // gone: respawning would spawn a bind-loser and then kill the live
            // serve when the old handle is replaced (the spawn/kill metronome).
            // Only respawn over a child confirmed dead; a serve wedged for
            // three straight windows gets a deliberate owner restart instead.
            let own_alive = app
                .state::<AppState>()
                .node_child
                .lock()
                .as_mut()
                .map(|c| c.is_alive())
                .unwrap_or(false);
            if own_alive {
                wedged_rounds += 1;
                if wedged_rounds >= 3 {
                    tracing::warn!(
                        "node socket dead across {wedged_rounds} grace windows with our serve alive — restarting it deliberately"
                    );
                    app.state::<AppState>().node_child.lock().take();
                    wedged_rounds = 0;
                    respawn_and_rehost(&app, &node).await;
                } else {
                    tracing::warn!(
                        "node socket unresponsive but our serve is still running — not respawning over it ({wedged_rounds}/3)"
                    );
                }
            } else {
                wedged_rounds = 0;
                tracing::info!("node is gone — bringing the shared stack back up");
                respawn_and_rehost(&app, &node).await;
            }
        } else {
            wedged_rounds = 0;
        }
        let (tx, mut rx) = mpsc::channel::<NodeEvent>(256);
        if let Err(e) = node.subscribe_events(tx).await {
            tracing::warn!("node event subscribe failed: {e:#}; retrying");
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        while let Some(ev) = rx.recv().await {
            match ev {
                NodeEvent::Emit { event, payload } => {
                    if event == "cec://request" {
                        let request_key = payload
                            .get("session_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            // The wire contract always includes a session id;
                            // retaining the full payload is a stable fallback
                            // for an older node that does not.
                            .unwrap_or_else(|| payload.to_string());
                        if foregrounded_requests.insert(request_key) {
                            reveal_main_window(&app);
                        }
                    }
                    let _ = app.emit(&event, payload);
                }
                NodeEvent::Restart => app.restart(), // never returns
            }
        }
        tracing::info!("node event stream ended; resubscribing");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Wait (briefly) for a node to answer the control socket after a spawn.
async fn wait_for_node() -> bool {
    for _ in 0..50 {
        if NodeClient::probe().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

fn run_gui() -> ExitCode {
    tauri::Builder::default()
        // One running copy: a second launch hands off to the first and exits,
        // instead of standing up a rival client fighting over the node socket.
        // Must be registered before any other plugin.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            reveal_main_window(app);
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .invoke_handler(tauri::generate_handler![
            cec_status,
            cec_online,
            cec_ask_help,
            machine_specs,
            machine_temps,
            open_tiktok,
            open_allmystuff_works,
            open_kvm_store,
            open_toolbox,
            toolbox_run,
            cec_pending,
            cec_approve,
            cec_deny,
            cec_revoke,
            cec_forget_node,
            cec_grants,
            cec_viewing,
            cec_set_label,
            cec_chat_send,
            cec_chat_history,
            session_snapshot,
            claim_node,
            kvm_attach,
            host_wifi_scan,
            site_map,
            kvm_api,
            open_kvm_console,
            fleet_kick,
            mesh_networks,
            mesh_peers,
            service_status,
            service_install,
            service_uninstall,
            service_start,
            service_stop,
            service_restart,
            autostart_get,
            autostart_mode_get,
            autostart_mode_set,
            autostart_set,
            background_get,
            background_set,
            update_status,
            update_check,
            update_apply,
            update_relaunch,
            update_set_prefs,
        ])
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // Close means close — quitting is the default. Only the
                    // explicit "keep running in the background" option turns the
                    // close into a hide-to-tray (Quit then lives in the tray menu).
                    let keep = window
                        .state::<AppState>()
                        .keep_background
                        .load(std::sync::atomic::Ordering::Relaxed);
                    if keep {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .setup(move |app| {
            if let Err(e) = build_tray(app.handle()) {
                tracing::warn!("couldn't create the tray icon: {e}");
            }
            // Migrate the autostart policy for a settings file from before the
            // choice existed: a user who already had run-on-boot ON keeps
            // "Always"; everyone else (fresh install, or a past opt-out) gets
            // the new grant-scoped default. From here the periodic
            // `reconcile_autostart` below owns the login item — grant-scoped
            // mode registers it only while a technician can still reconnect.
            let mut settings = load_gui_settings();
            if settings.autostart_mode.is_none() {
                let was_on =
                    settings.autostart_decided && app.autolaunch().is_enabled().unwrap_or(false);
                settings.autostart_mode = Some(if was_on {
                    AutostartMode::Always
                } else {
                    AutostartMode::WhileGranted
                });
                settings.autostart_decided = true;
                save_gui_settings(&settings);
            }
            // The window is created hidden (tauri.conf `visible: false`) so a
            // start-minimized login-item launch never flashes; reveal it now
            // unless we were asked to start minimized.
            let minimized = std::env::args().any(|a| a == "--minimized");
            if !minimized {
                reveal_main_window(app.handle());
            }

            let node = match NodeClient::new() {
                Ok(n) => Arc::new(n),
                Err(e) => {
                    tracing::error!("couldn't resolve the node socket: {e:#}");
                    return Err(e.into());
                }
            };
            app.manage(AppState {
                node: node.clone(),
                node_child: Mutex::new(None),
                kvm_tunnel_repair: tokio::sync::Mutex::new(()),
                keep_background: std::sync::atomic::AtomicBool::new(settings.keep_background),
            });

            // New NSIS installs are provisioned by the installer hook. Existing
            // machines can arrive through the portable self-updater, so replace
            // the old Session-0 service once when its ImagePath lacks the
            // interactive-host state arguments. Never prompt in development.
            #[cfg(all(windows, not(debug_assertions)))]
            let migrate_privileged_host = {
                let status = cec_support_service::status_value(false).unwrap_or_default();
                status
                    .get("privileged_host_current")
                    .and_then(Value::as_bool)
                    != Some(true)
            };
            #[cfg(not(all(windows, not(debug_assertions))))]
            let migrate_privileged_host = false;

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Upgrade the old service *before* bringing up a temporary GUI
                // node. Older builds previously did these concurrently, so the
                // old service, new service, and GUI could fight over one pipe.
                if migrate_privileged_host {
                    let migrated = match tokio::task::spawn_blocking(|| {
                        service_mutate_blocking("install")
                    })
                    .await
                    {
                        Ok(Ok(value))
                            if value.get("ok").and_then(Value::as_bool) == Some(true) =>
                        {
                            tracing::info!("installed the privileged interactive CEC host");
                            true
                        }
                        Ok(Ok(_)) => {
                            tracing::warn!("privileged CEC host setup did not complete");
                            false
                        }
                        Ok(Err(error)) => {
                            tracing::warn!("privileged CEC host setup failed: {error}");
                            false
                        }
                        Err(error) => {
                            tracing::warn!("privileged CEC host setup task failed: {error}");
                            false
                        }
                    };
                    if migrated && !wait_for_node().await {
                        tracing::warn!(
                            "migrated CEC host did not become ready; starting the GUI fallback"
                        );
                    }
                }
                // One node per machine, shared with AllMyStuff: reuse whatever
                // is already serving the control socket (an AllMyStuff GUI's
                // node, a service node), else spawn a transient one tied to
                // this app. The pin keeps a reused, not-ours node current to
                // what CEC needs.
                match ensure_node_running_pinned(ALLMYSTUFF_PIN).await {
                    Ok(child) => {
                        if let Some(c) = child {
                            handle.state::<AppState>().node_child.lock().replace(c);
                        }
                    }
                    Err(e) => tracing::error!("couldn't bring up the CEC node: {e:#}"),
                }
                wait_for_node().await;
                // Take up residence on the support area straight away, so a
                // launched app is already discoverable to a technician.
                if let Err(e) = node.request("cec_online", json!({})).await {
                    tracing::warn!("cec_online failed: {e:#}");
                }
                // Keep the OS login item in step with the autostart policy. A
                // periodic sweep (not just an event) because a 3-hour grant
                // lapses by the clock with nothing to hook — so grant-scoped
                // autostart both arms on approval and disarms on expiry within
                // a minute. Cheap: one local socket query per tick.
                {
                    let rec_app = handle.clone();
                    let rec_node = node.clone();
                    tauri::async_runtime::spawn(async move {
                        loop {
                            reconcile_autostart(&rec_app, &rec_node).await;
                            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        }
                    });
                }
                run_event_pump(handle, node).await;
            });

            // Self-update ticker. A launch check fires ~30s in (past the
            // interval cooldown, so opening the app is itself a check), then
            // every `check_interval_hours`. Spawned unconditionally —
            // `check_now` no-ops when auto-update is off, and reports rather
            // than stages when this install can't write its own binary.
            //
            // Every outcome is forwarded to the webview as `update://checked`.
            // A background task holds no handle to the UI, so without this a
            // staged update would sit on disk with nothing to announce it —
            // which is precisely how an updater ends up looking like it never
            // runs at all.
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(cec_support_updater::tick_forever_notify(move |outcome| {
                match serde_json::to_value(outcome) {
                    Ok(payload) => {
                        if let Err(e) = update_handle.emit("update://checked", payload) {
                            tracing::warn!("couldn't emit the self-update outcome: {e}");
                        }
                    }
                    Err(e) => tracing::warn!("couldn't serialise the self-update outcome: {e}"),
                }
            }));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the CEC Support GUI")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                // Kill the node we spawned (if any). A reused service node has
                // no child here and keeps running.
                app.state::<AppState>().node_child.lock().take();
            }
        });
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Headless verbs
// ---------------------------------------------------------------------------

/// `cec-support run [--service]` — the headless client agent. Brings up (or
/// reuses) the CEC node, joins the shared support area, and waits. This is what
/// the OS service's `ExecStart`/`binPath` runs (`run --service`), so a repair
/// can reconnect across reboots without the GUI open.
fn run_agent(service: bool) -> ExitCode {
    #[cfg(windows)]
    if service {
        return winsvc::dispatch();
    }
    let _ = service;
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("cec-support: couldn't start async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(async {
        // Hold the child so it (and the mesh daemon under it) dies with us.
        let _child = match ensure_node_running_pinned(ALLMYSTUFF_PIN).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("cec-support: couldn't bring up the CEC node: {e:#}");
                return ExitCode::FAILURE;
            }
        };
        wait_for_node().await;
        let node = match NodeClient::new() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("cec-support: couldn't resolve the node socket: {e:#}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = node.request("cec_online", json!({})).await {
            eprintln!("cec-support: couldn't go online on the support area: {e:#}");
        } else {
            println!("CEC Support is running and waiting for your technician.");
        }
        // Park until asked to stop (Ctrl-C / service stop).
        let _ = tokio::signal::ctrl_c().await;
        ExitCode::SUCCESS
    })
}

#[cfg(windows)]
mod winsvc {
    use std::ffi::OsString;
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use windows_service::define_windows_service;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_dispatcher;

    const SERVICE_NAME: &str = "CECSupport";
    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    define_windows_service!(ffi_service_main, service_main);

    pub fn dispatch() -> ExitCode {
        match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                tracing::error!("CEC Support service dispatcher failed: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn service_main(_args: Vec<OsString>) {
        if let Err(error) = run_service() {
            tracing::error!("CEC Support service stopped with error: {error}");
        }
    }

    fn run_service() -> windows_service::Result<()> {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let status = service_control_handler::register(SERVICE_NAME, handler)?;
        status.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(error) => {
                tracing::error!("couldn't locate the CEC Support service executable: {error}");
                return Ok(());
            }
        };
        let mut agent: Option<allmystuff_node::win_privilege::ConsoleAgent> = None;
        let mut agent_started: Option<Instant> = None;
        let mut short_failures = 0u32;
        let mut next_launch = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            let session_moved = agent.as_ref().is_some_and(|child| child.session_moved());
            let exited = agent.as_ref().is_some_and(|child| !child.alive());
            if session_moved || exited {
                if let Some(child) = agent.take() {
                    if !exited {
                        child.stop();
                    }
                }
                if session_moved {
                    short_failures = 0;
                    next_launch = Instant::now();
                } else {
                    let ran_for = agent_started
                        .take()
                        .map(|at| at.elapsed())
                        .unwrap_or_default();
                    short_failures = if ran_for >= Duration::from_secs(30) {
                        1
                    } else {
                        short_failures.saturating_add(1)
                    };
                    let delay = restart_delay(short_failures);
                    next_launch = Instant::now() + delay;
                    tracing::warn!(
                        ?delay,
                        short_failures,
                        "privileged CEC agent exited; delaying restart"
                    );
                }
            }
            if agent.is_none() && Instant::now() >= next_launch {
                match allmystuff_node::win_privilege::ConsoleAgent::launch(
                    &exe,
                    &["run", "--session-agent"],
                ) {
                    Ok(child) => {
                        tracing::info!(
                            "privileged CEC agent launched in the active console session"
                        );
                        agent = Some(child);
                        agent_started = Some(Instant::now());
                    }
                    Err(error) => {
                        short_failures = short_failures.saturating_add(1);
                        let delay = restart_delay(short_failures);
                        next_launch = Instant::now() + delay;
                        tracing::debug!(
                            ?delay,
                            "waiting for an interactive console session: {error}"
                        );
                    }
                }
            }
        }
        if let Some(child) = agent.take() {
            child.stop();
        }
        status.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;
        Ok(())
    }

    fn restart_delay(short_failures: u32) -> Duration {
        Duration::from_secs(1u64 << short_failures.min(6))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rapid_agent_failures_back_off_and_cap() {
            assert_eq!(restart_delay(1), Duration::from_secs(2));
            assert_eq!(restart_delay(2), Duration::from_secs(4));
            assert_eq!(restart_delay(6), Duration::from_secs(64));
            assert_eq!(restart_delay(100), Duration::from_secs(64));
        }
    }
}

/// `cec-support service <verb>` → the local service crate.
fn run_service(args: &[String]) -> ExitCode {
    let action = args
        .iter()
        .map(String::as_str)
        .find(|a| !a.starts_with('-'));
    let cmd = match action {
        Some("install") => cec_support_service::ServiceCmd::Install { log: None },
        Some("uninstall") | Some("remove") => cec_support_service::ServiceCmd::Uninstall,
        Some("start") => cec_support_service::ServiceCmd::Start,
        Some("stop") => cec_support_service::ServiceCmd::Stop,
        Some("restart") => cec_support_service::ServiceCmd::Restart,
        Some("status") => cec_support_service::ServiceCmd::Status,
        Some(other) => {
            eprintln!("cec-support service: unknown subcommand `{other}`");
            return ExitCode::FAILURE;
        }
        None => {
            eprintln!("Usage: cec-support service <install|uninstall|status|start|stop|restart>");
            return ExitCode::FAILURE;
        }
    };
    match cec_support_service::run(false, cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cec-support service: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// `cec-support id` — print this computer's support number (bringing up the
/// node if needed).
fn run_id() -> ExitCode {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("cec-support: couldn't start async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(async {
        let _child = ensure_node_running_pinned(ALLMYSTUFF_PIN)
            .await
            .ok()
            .flatten();
        wait_for_node().await;
        let node = match NodeClient::new() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("cec-support: couldn't resolve the node socket: {e:#}");
                return ExitCode::FAILURE;
            }
        };
        match node.request("cec_status", json!({})).await {
            Ok(v) => {
                let number = v.get("number").and_then(Value::as_str).unwrap_or("");
                if number.is_empty() {
                    eprintln!("cec-support: no support number yet (is the mesh daemon available?)");
                    ExitCode::FAILURE
                } else {
                    println!("{number}");
                    ExitCode::SUCCESS
                }
            }
            Err(e) => {
                eprintln!("cec-support: couldn't read status: {e:#}");
                ExitCode::FAILURE
            }
        }
    })
}

fn print_help() {
    println!(
        "CEC Support {ver}, one-tap remote help from Critical Error Computing.

USAGE:
    cec-support [COMMAND]

    With no command, opens the CEC Support app (what a customer runs).

COMMANDS:
    run [--service]   Start the client agent headless: join the Silent support
                      mesh and wait. `--service` is the mode the OS service runs.
    service <verb>    Install/manage the background service so CEC Support can
                      reconnect after reboots during a repair
                      (install | uninstall | status | start | stop | restart).
    id                Print this computer's support number.
    version           Print version.
    help              Show this help.",
        ver = env!("CARGO_PKG_VERSION")
    );
}

fn main() -> ExitCode {
    configure_service_environment();
    // Every path resolves the CEC app home + clears any stray app-id override
    // before the shared node socket is addressed.
    apply_cec_env();

    // Swap in anything the updater staged on a previous run, before any of it
    // is loaded. A running executable can't reliably replace itself, so the
    // apply always happens here — at the very start of the *next* launch —
    // rather than at the moment the download finished. Never fatal: a failure
    // logs and leaves the marker for the launch after this one.
    cec_support_updater::apply_pending_if_any();

    #[cfg(windows)]
    if std::env::args().any(|arg| arg == "--service-bootstrap") {
        let verb = process_arg_value("--service-bootstrap").unwrap_or_else(|| "install".into());
        return match service_mutate_blocking(&verb) {
            Ok(value) if value.get("ok").and_then(Value::as_bool) == Some(true) => {
                ExitCode::SUCCESS
            }
            Ok(_) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("CEC Support privileged host setup failed: {error}");
                ExitCode::FAILURE
            }
        };
    }

    // Elevated Windows service action: `<exe> --service-do <verb>` — run the
    // verb in-process and exit, no webview. (Unix calls the crate directly.)
    if let Some(verb) = service_do_verb() {
        if let Some(profile) = process_arg_value("--service-profile") {
            std::env::set_var("CEC_SUPPORT_SERVICE_PROFILE", profile);
        }
        if let Some(home) = process_arg_value("--service-home") {
            std::env::set_var("CEC_SUPPORT_SERVICE_HOME", home);
        }
        if let Some(sid) = process_arg_value("--service-sid") {
            std::env::set_var("CEC_SUPPORT_SERVICE_CLIENT_SID", sid);
        }
        let code = match service_cmd(&verb) {
            Some(cmd) => match cec_support_service::run(false, cmd) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("cec-support service {verb}: {e:#}");
                    1
                }
            },
            None => {
                eprintln!("cec-support: unknown service action `{verb}`");
                2
            }
        };
        return ExitCode::from(code as u8);
    }

    let log = std::env::var("CEC_SUPPORT_LOG").unwrap_or_else(|_| "info,cec_support=info".into());
    // Tee the log to a file under the CEC home: a `windows_subsystem =
    // "windows"` build has no console, so without this an installed app leaves
    // no evidence when something goes wrong. Any file trouble falls back to
    // stdout-only — logging must never block startup.
    let builder = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(log))
        .with_target(false);
    match open_log_file() {
        Some(file) => {
            use tracing_subscriber::fmt::writer::MakeWriterExt;
            builder
                .with_ansi(false)
                .with_writer(std::io::stdout.and(std::sync::Arc::new(file)))
                .init();
        }
        // No log file — stdout only. Colour just when a human is looking:
        // under a service manager this stdout is captured by journald/syslog,
        // and the default (ANSI unconditionally on) writes escape sequences
        // into it that bloat every line and break grep.
        None => builder.with_ansi(std::io::stdout().is_terminal()).init(),
    }

    let args: Vec<String> = std::env::args().skip(1).collect();

    // `--version` / `--help` flags, before the verb dispatch (so they aren't
    // mistaken for a `--minimized` GUI launch).
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("cec-support {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }

    // A `--minimized` login-item launch still opens the GUI (start-minimized is
    // handled in setup), so only *named* verbs branch away from it.
    let cmd = args
        .iter()
        .map(String::as_str)
        .find(|a| !a.starts_with('-'));

    match cmd {
        None => run_gui(),
        Some("run") => run_agent(args.iter().any(|a| a == "--service")),
        Some("service") => run_service(&args[1..]),
        Some("id") => run_id(),
        Some("version") => {
            println!("cec-support {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("cec-support: unknown command `{other}`\n");
            print_help();
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_scope_accepts_the_three_choices() {
        assert_eq!(canonical_scope("once").unwrap(), "once");
        assert_eq!(canonical_scope("three_hours").unwrap(), "three_hours");
        assert_eq!(canonical_scope("forever").unwrap(), "forever");
        assert!(canonical_scope("someday").is_err());
    }

    #[test]
    fn service_cmd_maps_known_verbs() {
        use cec_support_service::ServiceCmd;
        assert!(matches!(
            service_cmd("install"),
            Some(ServiceCmd::Install { .. })
        ));
        assert!(matches!(service_cmd("restart"), Some(ServiceCmd::Restart)));
        assert!(matches!(
            service_cmd("uninstall"),
            Some(ServiceCmd::Uninstall)
        ));
        assert!(service_cmd("frobnicate").is_none());
    }

    #[test]
    fn toolbox_is_a_fixed_allowlist() {
        assert_eq!(
            toolbox_spec("sfc"),
            Some(ToolboxSpec {
                label: "System File Checker",
                kind: ToolboxKind::AdminTerminal("sfc.exe /scannow"),
            })
        );
        assert!(matches!(
            toolbox_spec("device_manager"),
            Some(ToolboxSpec {
                kind: ToolboxKind::AdminTerminal(_),
                ..
            })
        ));
        assert!(matches!(
            toolbox_spec("registry_editor"),
            Some(ToolboxSpec {
                kind: ToolboxKind::AdminTerminal(_),
                ..
            })
        ));
        assert!(matches!(
            toolbox_spec("crucible_tests"),
            Some(ToolboxSpec {
                kind: ToolboxKind::ElevatedSidecar("cec-crucible"),
                ..
            })
        ));
        assert_eq!(toolbox_spec("powershell -Command whatever"), None);
    }

    #[test]
    fn toolbox_progress_run_ids_are_bounded_and_inert() {
        assert!(toolbox_run_id_valid("sfc-1786123456789-1"));
        assert!(toolbox_run_id_valid("disk_management_2"));
        assert!(!toolbox_run_id_valid(""));
        assert!(!toolbox_run_id_valid("sfc/../../other"));
        assert!(!toolbox_run_id_valid(&"x".repeat(97)));
    }

    #[test]
    fn toolbox_progress_lines_are_preserved_in_order() {
        let mut output = String::new();
        append_toolbox_line(&mut output, "Verification 10%\r");
        append_toolbox_line(&mut output, "Verification 20%\n");
        assert_eq!(output, "Verification 10%\nVerification 20%");
    }

    #[test]
    fn toolbox_progress_strips_terminal_escape_sequences() {
        assert_eq!(
            clean_toolbox_output(b"\x1b[32mVerification 42%\x1b[0m"),
            "Verification 42%"
        );
        assert_eq!(clean_toolbox_output(b"\x1b]0;PowerShell\x07Ready"), "Ready");
    }

    #[test]
    fn site_mapping_is_found_by_its_local_tunnel_port() {
        let mappings = json!([
            { "node": "other", "port": 443, "localPort": 47001 },
            { "node": "kvm", "port": 80, "localPort": 47000 }
        ]);
        assert_eq!(
            site_mapping_for_local_port(&mappings, 47000).unwrap(),
            Some(("kvm".into(), 80))
        );
        assert_eq!(site_mapping_for_local_port(&mappings, 49999).unwrap(), None);
    }

    #[test]
    fn malformed_site_mapping_is_not_used_for_repair() {
        let mappings = json!([{ "node": "kvm", "port": 70000, "localPort": 47000 }]);
        assert!(site_mapping_for_local_port(&mappings, 47000).is_err());
        assert!(site_mapping_for_local_port(&json!({}), 47000).is_err());
    }

    #[test]
    fn only_safe_get_transport_failures_trigger_tunnel_repair() {
        let transport_failure = json!({
            "status": 0,
            "body": null,
            "error": { "kind": "other", "message": "empty reply" }
        });
        let device_error = json!({ "status": 503, "body": null, "error": null });

        assert!(should_repair_kvm_tunnel("GET", &transport_failure));
        assert!(!should_repair_kvm_tunnel("POST", &transport_failure));
        assert!(!should_repair_kvm_tunnel("DELETE", &transport_failure));
        assert!(!should_repair_kvm_tunnel("GET", &device_error));
    }
}
