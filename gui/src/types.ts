// Shared types for the CEC Support client. These mirror the JSON the
// AllMyStuff node emits over its control socket for the CEC commands
// (`cec_*`) and events (`cec://*`) — see `tauri.ts` for the bridge and the
// README for the full contract.

/** The customer's three approval choices. Sent to `cec_approve` as a plain
 *  string; comes back inside a grant as `{ kind }` (the consent store's
 *  internally-tagged `ApprovalScope`). */
export type ApprovalScope = "once" | "three_hours" | "forever";

/** `cec_status` → the customer's own support identity. */
export interface CecStatus {
  /** The short Support number the customer reads to their technician when the
   *  hand-raise queue is crowded, e.g. "123456789". A display/verification
   *  label derived from the device key — never a mesh room. */
  number: string;
  /** The shared support area `network_id` ("cecsupport-clients") — the one
   *  mesh every CEC node lives on. */
  network_id: string;
  /** This node's role — always "client" for the customer app. */
  role: string;
  /** Whether this node is currently asking for help on the support area
   *  (the "Ask for help" button's live state). Absent from an older node. */
  asking_help?: boolean;
  /** This computer's friendly name, if the customer set one. */
  label?: string;
}

/** `machine_specs` → this machine's headline hardware, for the front door's
 *  spec card. Absent from an older node (the card just hides). */
export interface MachineSpecs {
  hostname: string;
  os: string;
  /** DMI system/board label — "Dell Inc. XPS 15" on a laptop, the
   *  motherboard model on a custom build. Null/absent when the firmware
   *  doesn't say (the card hides the row). */
  board?: string | null;
  /** Just the product / model name — the DMI product field without its
   *  maker prefix ("XPS 15", not "Dell Inc. XPS 15"). This is what the
   *  spec card shows: the model identifies the machine, the maker doesn't.
   *  Null/absent on odd firmware — the card falls back to `board`. */
  product?: string | null;
  cpu: {
    brand: string;
    cores: number | null;
    threads: number;
    max_mhz: number | null;
  };
  memory: { total_bytes: number; available_bytes: number };
  gpus: Array<{ name: string; vram_bytes: number | null }>;
  disks: Array<{
    name: string;
    mount: string | null;
    total_bytes: number;
    available_bytes: number;
    removable: boolean;
  }>;
  /** Temperature sensors as the OS exposes them, °C. Empty on most consumer
   *  Windows boards (no ACPI zone without a vendor driver) and absent from a
   *  pre-0.2.35 node — the card hides the row in both cases; it never invents
   *  numbers. */
  temps?: Array<{ label: string; celsius: number }>;
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
  /** The technician's canonical device id. The node includes it on an
   *  **auto-approve** (a standing grant reconnects with no prompt), where there
   *  was no preceding `cec://request` to learn it from — so the chat/session
   *  can bind to the right technician. Absent on a plain state transition. */
  tech?: string;
  /** The technician's Agent Name, alongside `tech` on an auto-approve. */
  agent_name?: string;
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

/** One line of the live support chat, as the node persists and streams it.
 *  Carried by `cec_chat_history` (a transcript snapshot) and the `cec://chat`
 *  event. Mirrors the wire `ChatMessage` in `allmystuff-cec-protocol`. */
export interface CecChatMsg {
  /** The node-assigned line id (deduplicates the sender's own echo). */
  id: string;
  /** Which side wrote the line, as the node attributes it from the
   *  authenticated sender — never the wire's self-declared role. On the
   *  customer's own node "client" is us, "technician" is the tech. */
  from: "client" | "technician";
  text: string;
  /** UNIX **seconds** the line was stamped (not milliseconds) — render with
   *  `new Date(ts * 1000)`. */
  ts: number;
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
