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

/** Where a purchase ask stands. `requested` → the beats this app reports
 *  (`seen` / `opened` / `claimed` / `declined`) → the technician's close
 *  (`confirmed` when the order checks out in the store, `cancelled` on a
 *  withdrawal). The two closing states are terminal. */
export type PurchaseState =
  | "requested"
  | "seen"
  | "opened"
  | "claimed"
  | "declined"
  | "confirmed"
  | "cancelled";

/** A purchase ask — the $50 diagnostic session. Carried by the
 *  `cec://purchase` event and the `cec_purchases` snapshot. A technician can
 *  raise one at any point in a help call: before connecting (quoting the work
 *  up front), mid-session, or after disconnecting.
 *
 *  Display strings only: payment happens in the customer's own browser on the
 *  store's hosted checkout (this app opens its own built-in purchase URL — the
 *  wire never chooses where the browser goes), and the technician confirms
 *  against the order that lands in the store. No card details ever touch this
 *  app or the mesh. */
export interface Purchase {
  purchase_id: string;
  /** Empty for an ask made outside a session (before the technician
   *  connected, or after they disconnected). */
  session_id: string;
  /** The technician's device id (the asker). */
  peer: string;
  /** The asker's Agent Name — from the standing approval when one exists (the
   *  name the customer actually let in), else the name the ask carried: the
   *  same trust as the connect prompt, checked against the person on the
   *  phone. */
  agent_name: string;
  /** What's being purchased, e.g. "CEC Diagnostic Session". */
  item: string;
  /** Display price, e.g. "$50" — the checkout page is authoritative. */
  price: string;
  /** Optional free-text from the technician. */
  note: string;
  state: PurchaseState;
  /** Unix seconds of the last state change. */
  updated_at: number;
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
