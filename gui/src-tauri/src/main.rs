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
async fn kvm_attach(state: State<'_, AppState>, node: String, target: String) -> Result<Value, String> {
    state
        .node
        .request("kvm_attach", json!({ "node": node, "target": target }))
        .await
        .map_err(|e| e.to_string())
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
        tracing::warn!("couldn't {} run-on-boot: {e}", if enable { "enable" } else { "disable" });
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
            cec_online,
            cec_ask_help,
            machine_specs,
            machine_temps,
            open_tiktok,
            open_allmystuff_works,
            cec_pending,
            cec_approve,
            cec_deny,
            cec_revoke,
            cec_forget_node,
            cec_grants,
            cec_set_label,
            cec_chat_send,
            cec_chat_history,
            session_snapshot,
            claim_node,
            kvm_attach,
            site_map,
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
        ])
        .on_window_event(|window, event| {
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
                keep_background: std::sync::atomic::AtomicBool::new(settings.keep_background),
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
        None => builder.init(),
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
}
