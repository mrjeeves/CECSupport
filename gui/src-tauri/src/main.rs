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

// ---------------------------------------------------------------------------
// CEC node-control commands (the fixed contract the sibling node implements)
// ---------------------------------------------------------------------------

/// `{ number, network_id, role, hosting, label }` — the customer's own support
/// identity + hosting state.
#[tauri::command]
async fn cec_status(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("cec_status", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// Join our own number-derived Silent mesh and wait for a technician to dial.
/// Returns `{ number }`.
#[tauri::command]
async fn cec_start_hosting(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .node
        .request("cec_start_hosting", json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// Leave the Silent mesh — "stop sharing".
#[tauri::command]
async fn cec_stop_hosting(state: State<'_, AppState>) -> Result<(), String> {
    state
        .node
        .request("cec_stop_hosting", json!({}))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
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
        .request("cec_deny", json!({ "tech": tech, "session_id": session_id }))
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

/// The customer's standing approvals. The node's grants are validated against
/// the shared consent-store [`Grant`](allmystuff_cec_consent::Grant) shape
/// before being handed to the GUI, so a wire drift is logged rather than
/// mis-rendered.
#[tauri::command]
async fn cec_grants(state: State<'_, AppState>) -> Result<Value, String> {
    let v = state
        .node
        .request("cec_grants", json!({}))
        .await
        .map_err(|e| e.to_string())?;
    if serde_json::from_value::<Vec<allmystuff_cec_consent::Grant>>(v.clone()).is_err() {
        tracing::warn!("cec_grants: node returned grants that don't match the shared Grant shape");
    }
    Ok(v)
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
    let ps = format!(
        "try {{ $p = Start-Process -FilePath '{exe}' -ArgumentList '--service-do','{verb}' \
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

#[tauri::command]
async fn service_install() -> Result<Value, String> {
    service_mutate("install").await
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
    Ok(mgr.is_enabled().unwrap_or(enabled))
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
async fn run_event_pump(app: tauri::AppHandle, node: Arc<NodeClient>) {
    use tokio::sync::mpsc;
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
            tracing::info!("node is gone — bringing the shared stack back up");
            match ensure_node_running_pinned(ALLMYSTUFF_PIN).await {
                Ok(Some(child)) => {
                    app.state::<AppState>().node_child.lock().replace(child);
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("couldn't bring the node back up: {e:#}"),
            }
            wait_for_node().await;
            // A fresh serve isn't hosting: re-advertise so this machine stays
            // dialable (idempotent when the room is already joined).
            if let Err(e) = node.request("cec_start_hosting", json!({})).await {
                tracing::warn!("cec_start_hosting after node respawn failed: {e:#}");
            }
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
async fn wait_for_node() {
    for _ in 0..50 {
        if NodeClient::probe().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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
            cec_start_hosting,
            cec_stop_hosting,
            cec_pending,
            cec_approve,
            cec_deny,
            cec_revoke,
            cec_forget_node,
            cec_grants,
            cec_set_label,
            service_status,
            service_install,
            service_uninstall,
            service_start,
            service_stop,
            service_restart,
            autostart_get,
            autostart_set,
        ])
        .setup(move |app| {
            if let Err(e) = build_tray(app.handle()) {
                tracing::warn!("couldn't create the tray icon: {e}");
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
            });

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
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
                // Join our Silent mesh straight away, so a launched app is
                // already discoverable to a technician who has the number.
                if let Err(e) = node.request("cec_start_hosting", json!({})).await {
                    tracing::warn!("cec_start_hosting failed: {e:#}");
                }
                run_event_pump(handle, node).await;
            });
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
/// reuses) the CEC node, joins the Silent support mesh, and waits. This is what
/// the OS service's `ExecStart`/`binPath` runs (`run --service`), so a repair
/// can reconnect across reboots without the GUI open.
fn run_agent(_service: bool) -> ExitCode {
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
        if let Err(e) = node.request("cec_start_hosting", json!({})).await {
            eprintln!("cec-support: couldn't start hosting: {e:#}");
        } else {
            println!("CEC Support is running and waiting for your technician.");
        }
        // Park until asked to stop (Ctrl-C / service stop).
        let _ = tokio::signal::ctrl_c().await;
        ExitCode::SUCCESS
    })
}

/// `cec-support service <verb>` → the local service crate.
fn run_service(args: &[String]) -> ExitCode {
    let action = args.iter().map(String::as_str).find(|a| !a.starts_with('-'));
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
            eprintln!(
                "Usage: cec-support service <install|uninstall|status|start|stop|restart>"
            );
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
        "CEC Support {ver} — one-tap remote help from Critical Error Computing.

USAGE:
    cec-support [COMMAND]

    With no command, opens the CEC Support app (what a customer runs).

COMMANDS:
    run [--service]   Start the client agent headless — join the Silent support
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
    // Every path resolves the CEC app home + clears any stray app-id override
    // before the shared node socket is addressed.
    apply_cec_env();

    // Elevated Windows service action: `<exe> --service-do <verb>` — run the
    // verb in-process and exit, no webview. (Unix calls the crate directly.)
    if let Some(verb) = service_do_verb() {
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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(log))
        .with_target(false)
        .init();

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
}
