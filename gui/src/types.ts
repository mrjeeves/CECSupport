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

// ---------------------------------------------------------------------------
// KVM & claiming — the customer's "KVM and Claiming" card. These mirror the
// node's generic mesh surface (the same `session_snapshot` / `claim_node` /
// `kvm_attach` / `site_map` the AllMyStuff app drives), trimmed to just the
// fields the card needs. "CEC KVM" isn't a distinct wire type — it's a KVM
// appliance (advertises `FEATURE_KVM`) whose own joining mesh is a
// `cec-kvm-…` network; the store discriminates on that.
// ---------------------------------------------------------------------------

/** The presence feature tag a KVM appliance advertises (mirrors the node's
 *  Rust `FEATURE_KVM`) — a NanoKVM-class device that captures a machine's HDMI
 *  and injects USB-HID, carrying its own web UI. */
export const FEATURE_KVM = "kvm";

/** One site a peer exposes for reverse-proxying (mirrors the node's
 *  `SiteAdvert`). Used here only to find a KVM's own web UI — the address the
 *  Reboot POST is tunnelled to. */
export interface SiteAdvert {
  /** Stable id (`tcp:80`). */
  id: string;
  label: string;
  port: number;
  /** URL scheme ("http", "https", …) — a web scheme is what makes the site
   *  the KVM's console. Absent on a bare TCP service. */
  scheme?: string;
}

/** A KVM appliance's presence binding (the node's `NodeProfile.kvm`,
 *  snake_case on the wire). Present only on a peer advertising `FEATURE_KVM`. */
export interface KvmAdvert {
  /** The node id of the machine this KVM physically controls. Absent = not
   *  bound to anything yet (freshly claimed, or detached). */
  attached_to?: string;
  /** The `SiteAdvert.id` serving the KVM's own web UI (absent = the UI falls
   *  back to the first web-scheme site). */
  web?: string;
  /** The per-device `cec-kvm-…` mesh the KVM ships on and returns to when
   *  reset/unclaimed — the CEC-KVM discriminator. */
  joining_mesh?: string;
  /** Every mesh the KVM is currently joined to. */
  meshes?: string[];
}

/** One peer from `session_snapshot`, trimmed to what the KVM & Claiming card
 *  needs. */
export interface MeshPeer {
  /** The peer's mesh node id (used for claim / attach / site_map). */
  node: string;
  label?: string;
  hostname?: string;
  /** Who owns it (its own presence advert), or null when unowned. */
  owner?: string | null;
  /** Whether it's currently offering itself for adoption. */
  claimable?: boolean;
  /** App feature tags it advertises ("kvm", "sites", …). */
  features?: string[];
  /** Sites it exposes — where a KVM's web UI is found. */
  sites?: SiteAdvert[];
  /** KVM binding, present only on a KVM appliance. */
  kvm?: KvmAdvert;
}

/** The node's live mesh snapshot (`session_snapshot`). `me` is this node's own
 *  mesh id — the attach-to-this-computer target. */
export interface SessionSnapshot {
  ready: boolean;
  me?: string;
  peers?: MeshPeer[];
}

/** One network the node is on (`mesh_networks`) — trimmed to the id we query
 *  live peers for. */
export interface NetworkSummary {
  network_id: string;
}

/** One peer on a network's *live* peer list (`mesh_peers`), trimmed to the
 *  reachability signal. `status` is "active"/"shelved" when the node can
 *  actually reach the peer, and a remembered-but-offline value
 *  ("offline"/"sighted"/"handshaking") otherwise. Distinct from `MeshPeer`,
 *  which is a presence advert the node keeps even after the peer goes away. */
export interface MeshPeerInfo {
  device_id: string;
  status: string;
}

/** A claimable/claimed CEC KVM as the card renders it — the store projects
 *  this from a `MeshPeer` against our own node id, so the card stays a dumb
 *  view of the current lifecycle state. */
export interface CecKvm {
  /** The KVM's mesh node id. */
  node: string;
  /** Friendly label to show. */
  label: string;
  /** Still offering itself for adoption (show the Claim option). */
  claimable: boolean;
  /** We own it (owner == this node). */
  mine: boolean;
  /** It's bound to this computer (kvm.attached_to == this node). */
  attachedHere: boolean;
  /** Ours, not yet attached here, and the customer hasn't answered the
   *  "is it on this computer?" prompt — so the card shows that prompt. */
  promptAttach: boolean;
  /** It advertises a web UI, so Reboot / Wi-Fi (both over the tunnel) are
   *  reachable. */
  hasWeb: boolean;
}

// ---------------------------------------------------------------------------
// KVM Wi-Fi — reading and setting a claimed KVM's own Wi-Fi over the same mesh
// "sites" tunnel the Reboot uses. The appliance already owns the Wi-Fi system;
// these types just mirror what its web API returns. The `GET /api/network/wifi`
// body differs by model — a plain NanoKVM sends a bare `ssid` string, a
// NanoKVM-Pro a `wifi` object (and adds a Pro-only `/scan`) — but the connect
// request (`{ ssid, password }`) is identical on both, so one flow drives them.
// ---------------------------------------------------------------------------

/** The KVM web API's response envelope. `code === 0` is success; any other
 *  value is a failure — and the HTTP status is 200 either way, so the code is
 *  what must be checked, never `res.ok` alone. */
export interface KvmApiRsp<T = unknown> {
  code: number;
  msg?: string;
  data?: T;
}

/** The raw `data` of a KVM's `GET /api/network/wifi`, spanning both model
 *  shapes. Normalized into {@link KvmWifiStatus} before the UI sees it. */
export interface KvmWifiStatusRaw {
  supported?: boolean;
  apMode?: boolean;
  connected?: boolean;
  /** NanoKVM (plain): the connected network's name, inline. */
  ssid?: string;
  /** NanoKVM-Pro: the connected network as an object (null when none). */
  wifi?: { ssid?: string } | null;
}

/** A KVM's current Wi-Fi state, normalized across the two model shapes. */
export interface KvmWifiStatus {
  /** The appliance has a Wi-Fi radio at all (false → the panel says so). */
  supported: boolean;
  /** It's in setup/hotspot (AP) mode rather than joined to a network. */
  apMode: boolean;
  /** It's joined to a network right now. */
  connected: boolean;
  /** The joined network's name, or null when not connected / unknown. */
  ssid: string | null;
}

/** One network from a KVM's scan (`GET /api/network/wifi/scan`, Pro only).
 *  `signal` is dBm (closer to 0 = stronger); `security` is "open" for an
 *  unsecured network. All but `ssid` are best-effort. */
export interface KvmWifiNetwork {
  ssid: string;
  bssid?: string;
  signal?: number;
  security?: string;
  frequency?: number;
}
