// Shared types for the CEC Support client. These mirror the JSON the
// AllMyStuff node emits over its control socket for the CEC commands
// (`cec_*`) and events (`cec://*`) — see `tauri.ts` for the bridge and the
// README for the full contract.

/** The customer's three approval choices. Sent to `cec_approve` as a plain
 *  string; comes back inside a grant as `{ kind }` (the consent store's
 *  internally-tagged `ApprovalScope`). */
export type ApprovalScope = "once" | "three_hours" | "forever";

/** `cec_status` → the customer's own support identity + hosting state. */
export interface CecStatus {
  /** The short Support number the customer reads to their technician,
   *  e.g. "123456789". */
  number: string;
  /** The number-derived Silent-mesh `network_id` (e.g. "cec-123456789"). */
  network_id: string;
  /** This node's role — always "client" for the customer app. */
  role: string;
  /** Whether we've joined our own Silent mesh and are waiting for a dial. */
  hosting: boolean;
  /** This computer's friendly name, if the customer set one. */
  label?: string;
}

/** A technician's inbound connect request — drives the 3-choice modal.
 *  Carried by `cec_pending` (a snapshot list) and the `cec://request` event. */
export interface ConnectRequest {
  /** The technician's canonical device id. */
  tech: string;
  /** The Agent Name to show ("‹Agent› is trying to connect"). */
  agent_name: string;
  /** View-only (false) vs full keyboard/mouse control (true). */
  want_control: boolean;
  /** This dial's session id. */
  session_id: string;
  /** A 6-digit code the node still emits, kept for wire compatibility. The
   *  approval UI no longer surfaces it — the customer confirms the technician by
   *  the Agent Name shown ("‹Agent› is trying to connect"), not a read-out code. */
  verification_code: string;
}

/** `cec://session` — a live session's state changed. */
export interface SessionEvent {
  session_id: string;
  /** e.g. "connecting" | "active" | "ended". */
  state: string;
}

/** A live session the customer is currently in, as tracked by the store
 *  (correlating `cec://session` state with the request that opened it). */
export interface LiveSession {
  session_id: string;
  state: string;
  tech: string;
  agent_name: string;
  want_control: boolean;
}

/** One standing approval — the consent store's `Grant`. `cec_grants` returns
 *  these and `cec://grants` streams the current set. */
export interface Grant {
  /** The technician's canonical device id. */
  technician: string;
  /** The Agent Name the customer saw when approving. */
  agent_name: string;
  /** Whether the grant includes keyboard/mouse control (true) or is view-only
   *  (false). The node flattens the consent store's capability set to this
   *  single boolean on the wire (see `Cec::grants`). */
  control: boolean;
  /** How the grant was made. A string over the flat wire, or `{ kind }` from
   *  the consent store's tagged form — read it with {@link grantScope}. */
  scope: ApprovalScope | { kind: ApprovalScope };
  /** Unix seconds the grant was made. */
  granted_at: number;
  /** Absolute expiry (unix seconds), or null for "Forever". */
  expires_at: number | null;
}

/** Read a grant's scope regardless of whether the wire sent a bare string or
 *  the consent store's `{ kind }` tagged object. */
export function grantScope(g: Grant): ApprovalScope {
  return typeof g.scope === "string" ? g.scope : g.scope.kind;
}

/** The OS background-service status, from the `cec-support-service` crate. */
export interface ServiceStatus {
  platform: string;
  supported: boolean;
  manager?: string | null;
  scope?: string;
  service_name?: string;
  installed: boolean;
}

/** The result of a service mutation (install / uninstall / …). */
export interface ServiceResult {
  ok: boolean;
  output: string;
}
