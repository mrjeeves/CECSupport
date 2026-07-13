// Thin bridge to the Tauri backend.
//
// Everything degrades gracefully when the app runs as a plain web page (no
// Tauri) — `pnpm dev` in a browser, this repo's CI build — so the UI is always
// interactive even without the Rust side or a running node. In web mode the
// `cec_*` calls return null and the store falls back to a small demo.
//
// The command + event names here are the *fixed contract* the sibling agent
// implements on the AllMyStuff node's control socket:
//
//   cec_status {}                         -> { number, network_id, role }
//   cec_online {}                         -> { number }
//   cec_pending {}                        -> ConnectRequest[]
//   cec_approve { tech, scope, session_id, want_control }
//   cec_deny { tech, session_id }
//   cec_revoke { tech }
//   cec_grants {}                         -> Grant[]
//   cec_forget_node { node }
//   cec_set_label { label }               (friendly-name helper; see README)
//
//   event cec://request { tech, agent_name, want_control, session_id, verification_code }
//   event cec://session { session_id, state }
//   event cec://grants  { grants }
//
// The service commands (install / uninstall / status / …) go to the local
// `cec-support-service` crate via separate Tauri commands, NOT the node.

import type {
  CecStatus,
  ConnectRequest,
  Grant,
  ApprovalScope,
  MachineSpecs,
  SessionEvent,
  ServiceStatus,
  ServiceResult,
} from "./types";

/** True when running inside the Tauri webview (vs a plain browser tab). */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function rawInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return (await invoke(cmd, args)) as T;
}

/** Invoke a backend command, returning null (not throwing) in web mode or on
 *  failure — the calm default for a customer app that must never show a stack
 *  trace for a transient hiccup. */
async function tryInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T | null> {
  if (!isTauri()) return null;
  try {
    return await rawInvoke<T>(cmd, args);
  } catch (e) {
    console.warn(`backend command ${cmd} failed:`, e);
    return null;
  }
}

/** Invoke a command that must surface its error to the caller (an approve /
 *  deny the customer explicitly tapped, where silent failure would be worse
 *  than a message). No-op-safe in web mode. */
async function mustInvoke(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<void> {
  if (!isTauri()) return;
  await rawInvoke(cmd, args);
}

// ---- app metadata -----------------------------------------------------

/** The running app's version (e.g. "0.1.0"), or null in web mode. */
export async function appVersion(): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    return await getVersion();
  } catch (e) {
    console.warn("app version unavailable:", e);
    return null;
  }
}

// ---- CEC node control (the customer's own node) ------------------------

/** This customer's support identity. Null in web mode. */
export function cecStatus(): Promise<CecStatus | null> {
  return tryInvoke<CecStatus>("cec_status");
}

/** Take up residence on the shared support area so a technician can see and
 *  dial this device. Returns `{ number }` (the display label). Called once at
 *  startup; idempotent, safe to call again. */
export function cecOnline(): Promise<{ number: string } | null> {
  return tryInvoke<{ number: string }>("cec_online");
}

/** Raise (or withdraw) the hand on the support area. Errors surface — the
 *  customer must know their tap didn't take, not wait on a dead beacon. */
export function cecAskHelp(on: boolean): Promise<void> {
  return mustInvoke("cec_ask_help", { on });
}

/** This machine's headline hardware for the spec card. Null in web mode, on
 *  an older node (no such command), or while the node is still starting —
 *  the card simply doesn't render then. */
export function machineSpecs(): Promise<MachineSpecs | null> {
  return tryInvoke<MachineSpecs>("machine_specs");
}

/** Temps alone — no full scan behind it, cheap to poll. Null in web mode or
 *  on an older node (no such command); the poller just stops updating then. */
export function machineTemps(): Promise<{
  temps: MachineSpecs["temps"];
} | null> {
  return tryInvoke<{ temps: MachineSpecs["temps"] }>("machine_temps");
}

/** Open CEC's TikTok in the system browser — the waiting screen's "catch us
 *  live" link. The URL lives backend-side; web mode opens a plain tab. */
export function openTiktok(): void {
  if (!isTauri()) {
    window.open("https://www.tiktok.com/@criticalerrorcomputing", "_blank");
    return;
  }
  void tryInvoke("open_tiktok");
}

/** Open allmystuff.works in the system browser — the waiting screen's upsell
 *  link (CEC Support runs on the AllMyStuff system; the full app is for your
 *  own machines). Backend-opened in the app, a plain tab in web mode. */
export function openAllmystuffWorks(): void {
  if (!isTauri()) {
    window.open("https://allmystuff.works", "_blank");
    return;
  }
  void tryInvoke("open_allmystuff_works");
}

/** The help/asking state changed (`cec://help`). `asking: false` means the
 *  node auto-withdrew the ask (help arrived) and the waiting card must
 *  follow. `watchers` rides every beacon the node sends: how many live
 *  peers the broadcast actually reached — 0 is "still raising your hand",
 *  1+ is "CEC can see you". */
export async function onCecHelp(
  cb: (e: { asking?: boolean; watchers?: number }) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<{ asking?: boolean; watchers?: number }>("cec://help", (e) => cb(e.payload));
}

/** The technician requests currently awaiting the customer's decision. */
export async function cecPending(): Promise<ConnectRequest[]> {
  const r = await tryInvoke<ConnectRequest[]>("cec_pending");
  return Array.isArray(r) ? r : [];
}

/** Approve a technician's dial with one of the three scopes. `wantControl`
 *  echoes the request so the grant carries the right capability set. Throws on
 *  a real backend failure so the modal can say so. */
export function cecApprove(
  tech: string,
  scope: ApprovalScope,
  sessionId: string,
  wantControl: boolean,
): Promise<void> {
  return mustInvoke("cec_approve", {
    tech,
    scope,
    sessionId,
    wantControl,
  });
}

/** Decline a technician's dial, or end the current session (leaving any
 *  standing grant intact — that's what "Forget" is for). */
export function cecDeny(tech: string, sessionId: string): Promise<void> {
  return mustInvoke("cec_deny", { tech, sessionId });
}

/** Revoke a technician's standing approval — the "Forget this technician"
 *  action. Bites immediately, mid-session. */
export function cecRevoke(tech: string): Promise<void> {
  return mustInvoke("cec_revoke", { tech });
}

/** Forget a technician's node from the mesh entirely (drops the peer, not just
 *  the grant). Paired with {@link cecRevoke} in the Forget flow. */
export function cecForgetNode(node: string): Promise<void> {
  return mustInvoke("cec_forget_node", { node });
}

/** The customer's standing approvals (persistent + this-session). */
export async function cecGrants(): Promise<Grant[]> {
  const r = await tryInvoke<Grant[]>("cec_grants");
  return Array.isArray(r) ? r : [];
}

/** Set this computer's friendly name (shown to the technician on the mesh).
 *  A convenience beyond the core dial/approve contract; see README. */
export function cecSetLabel(label: string): Promise<null> {
  return tryInvoke("cec_set_label", { label });
}

// ---- CEC events (drive the modal, banner, and access list live) --------

/** A technician is dialing in — drive the 3-choice Approve modal. Returns an
 *  unlisten fn (a no-op in web mode). */
export async function onCecRequest(
  cb: (r: ConnectRequest) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<ConnectRequest>("cec://request", (e) => cb(e.payload));
}

/** A session's state changed (connecting / active / ended) — update the
 *  Connected banner. */
export async function onCecSession(
  cb: (s: SessionEvent) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<SessionEvent>("cec://session", (e) => cb(e.payload));
}

/** The standing-grants set changed — refresh the "who can reach me" list. */
export async function onCecGrants(
  cb: (grants: Grant[]) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<{ grants: Grant[] }>("cec://grants", (e) =>
    cb(e.payload.grants ?? []),
  );
}

// ---- background service (the local cec-support-service crate) -----------

/** The OS background-service status ({ installed, supported, manager, … }). */
export function serviceStatus(): Promise<ServiceStatus | null> {
  return tryInvoke<ServiceStatus>("service_status");
}

/** Install + start the CEC Support background service (so it reconnects after
 *  reboots during a repair). Elevates on Windows. Returns { ok, output }. */
export function serviceInstall(): Promise<ServiceResult | null> {
  return tryInvoke<ServiceResult>("service_install");
}

/** Stop + remove the background service. */
export function serviceUninstall(): Promise<ServiceResult | null> {
  return tryInvoke<ServiceResult>("service_uninstall");
}

/** Stop the background service without uninstalling it. */
export function serviceStop(): Promise<ServiceResult | null> {
  return tryInvoke<ServiceResult>("service_stop");
}

/** Whether "Start with this computer" (the login item) is registered. */
export async function autostartGet(): Promise<boolean> {
  const r = await tryInvoke<boolean>("autostart_get");
  return r ?? false;
}

/** Register / unregister the login item. Returns the resulting state. */
export async function autostartSet(enabled: boolean): Promise<boolean> {
  const r = await tryInvoke<boolean>("autostart_set", { enabled });
  return r ?? enabled;
}

/** When the app opens with the computer: only while a technician grant is
 *  live (`while_granted`, the default), `always`, or `off`. */
export type AutostartMode = "while_granted" | "always" | "off";

/** The current autostart policy. */
export async function autostartModeGet(): Promise<AutostartMode> {
  const r = await tryInvoke<string>("autostart_mode_get");
  return r === "always" || r === "off" ? r : "while_granted";
}

/** Set the autostart policy and apply it immediately. */
export async function autostartModeSet(mode: AutostartMode): Promise<void> {
  await tryInvoke("autostart_mode_set", { mode });
}

/** Whether closing the window keeps the app running in the tray (opt-in —
 *  off means the close button really quits). */
export async function backgroundGet(): Promise<boolean> {
  const r = await tryInvoke<boolean>("background_get");
  return r ?? false;
}

/** Flip keep-running-in-background. Returns the resulting state. */
export async function backgroundSet(enabled: boolean): Promise<boolean> {
  const r = await tryInvoke<boolean>("background_set", { enabled });
  return r ?? enabled;
}

/** Copy text to the clipboard — used by the "copy my number" button. Uses the
 *  browser clipboard API (available in the Tauri webview too). */
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch (e) {
    console.warn("clipboard write failed:", e);
  }
  return false;
}
