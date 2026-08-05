// The single reactive store for the CEC Support client (Svelte 5 runes).
//
// It owns the customer's view of their own node: the Support number + hosting
// state, the technician requests awaiting a decision (which drive the 3-choice
// modal), the live sessions (the Connected banner), and the standing grants
// (the access list). It subscribes to the `cec://*` events so all of that
// stays live, and exposes the handful of actions the UI needs.
//
// In web mode (no Tauri) it loads a small, calm demo so the page is always
// explorable — the same graceful-degradation discipline as `tauri.ts`.

import {
  appVersion,
  autostartGet,
  autostartSet,
  autostartModeGet,
  autostartModeSet,
  type AutostartMode,
  backgroundGet,
  backgroundSet,
  cecApprove,
  cecAskHelp,
  cecChatHistory,
  cecChatSend,
  cecDeny,
  cecForgetNode,
  cecGrants,
  cecViewing,
  cecPending,
  cecRevoke,
  onUpdateChecked,
  updateApply,
  updateCheck,
  updateRelaunch,
  updateSetPrefs,
  updateStatus,
  cecSetLabel,
  claimNode,
  fleetKick,
  kvmAttach,
  machineSpecs,
  cecOnline,
  cecStatus,
  isTauri,
  meshNetworks,
  meshPeers,
  onCecChat,
  onCecGrants,
  onCecHelp,
  onCecRequest,
  onCecSession,
  onCecViewing,
  serviceInstall,
  serviceStatus,
  serviceStop,
  serviceUninstall,
  sessionSnapshot,
  siteMap,
  kvmApiCall,
  openKvmConsole,
  hostWifiScan,
} from "./tauri";
import { FEATURE_KVM } from "./types";
import type {
  AccessRow,
  ApprovalScope,
  CecChatMsg,
  CecKvm,
  CecStatus,
  ConnectRequest,
  Grant,
  KvmApiCallResult,
  KvmApiRsp,
  KvmInfo,
  KvmHelpStatus,
  KvmLink,
  KvmVersion,
  KvmWifiNetwork,
  KvmWifiStatus,
  KvmWifiStatusRaw,
  HostWifi,
  LiveSession,
  MachineSpecs,
  MeshPeer,
  ServiceStatus,
  SessionEvent,
  SessionSnapshot,
  SiteAdvert,
  UpdateStatus,
  CheckOutcome,
  UpdatePrefs,
} from "./types";

/** The stable machine identity inside a mesh device id: the bare pubkey with
 *  MyOwnMesh's 5-char display suffix (`-AB12C`) stripped. Chat lines stream
 *  under the node's canonical (stripped) peer key, so the thread map is keyed
 *  by this too — otherwise a session that carried the display id would file its
 *  history under one key and the live `cec://chat` echoes under another. Mirrors
 *  the node's `pubkey_part`. */
function canonicalTech(id: string): string {
  const dash = id.lastIndexOf("-");
  if (dash > 0) {
    const suffix = id.slice(dash + 1);
    if (suffix.length === 5 && /^[0-9a-zA-Z]+$/.test(suffix)) {
      return id.slice(0, dash);
    }
  }
  return id;
}

function errMsg(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

/** Upper-case the first letter so a lower-case reason from the backend reads
 *  as a sentence when it lands in the UI. */
function capitalise(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

/** What one {@link CecStore.kvmApi} call produced: the device's envelope when
 *  the call succeeded, or a null `rsp` with a `reason` worth showing a human.
 *  `timedOut` is carried separately because a timeout is not always a failure —
 *  a Wi-Fi write times out exactly when it works and the KVM changes network. */
interface KvmApiOutcome<T> {
  rsp: KvmApiRsp<T> | null;
  timedOut: boolean;
  reason: string | null;
  /** The tunnel port that carried the request, including a repaired mapping. */
  localPort: number;
}

/** Fold a KVM's `GET /api/network/wifi` body — either model shape — into the
 *  one {@link KvmWifiStatus} the UI reads. A plain NanoKVM carries the SSID
 *  inline (`ssid`); a Pro nests it (`wifi.ssid`). */
function normalizeWifi(data: KvmWifiStatusRaw): KvmWifiStatus {
  const ssid = (data.ssid ?? data.wifi?.ssid ?? "").trim();
  return {
    supported: !!data.supported,
    apMode: !!data.apMode,
    connected: !!data.connected,
    ssid: ssid || null,
  };
}

/** Drop nameless/empty entries and order a scan strongest-signal first (the
 *  order a customer wants when picking their network). */
function sortNetworks(list: KvmWifiNetwork[]): KvmWifiNetwork[] {
  return list
    .filter((w) => w && typeof w.ssid === "string" && w.ssid.length > 0)
    .slice()
    .sort((a, b) => (b.signal ?? -999) - (a.signal ?? -999));
}

/** Canned "Open" targets for the browser preview, so the menu is explorable
 *  without a device (demo mode only). */
function demoLinks(): KvmLink[] {
  return [
    { kind: "wired", label: "Ethernet", detail: "192.168.1.42", host: "192.168.1.42", port: 80, scheme: "http" },
    { kind: "wireless", label: "Wi-Fi", detail: "192.168.1.77", host: "192.168.1.77", port: 80, scheme: "http" },
    { kind: "usb", label: "USB", detail: "192.168.7.1", host: "192.168.7.1", port: 80, scheme: "http" },
    { kind: "mesh", label: "Mesh", detail: "Through this app", host: "127.0.0.1", port: 8000, scheme: "http" },
  ];
}

/** A canned scan for the browser preview so the network picker is explorable
 *  without a device (demo mode only). */
function demoScanList(): KvmWifiNetwork[] {
  return [
    { ssid: "CEC-Guest", bssid: "aa:bb:cc:00:11:22", signal: -47, security: "wpa2" },
    { ssid: "Reception 5G", bssid: "aa:bb:cc:00:11:23", signal: -64, security: "wpa2" },
    { ssid: "Lobby-Open", bssid: "aa:bb:cc:00:11:24", signal: -73, security: "open" },
  ];
}

/** How long a KVM keeps reading "reachable" after the last time the daemon
 *  reported it live (active/shelved) on some network. Bridges the gap while the
 *  daemon dials a freshly discovered claimable on the local-claim network (a
 *  second or two of `sighted`/`handshaking` before the link is `active`) and
 *  any later transport rebuild, so a genuinely present KVM doesn't drop out of
 *  the card between our on-demand refreshes. Mirrors the AllMyStuff app's
 *  `PRESENCE_GRACE_MS`; an explicit `offline`/`error` clears it immediately, so
 *  a powered-off KVM still drops within one refresh, not after the window. */
/** Consecutive discovery passes a peer may go without reading live before the
 *  card drops it. Counted in SAMPLES, not milliseconds — one tolerates a look
 *  that lands mid-dial; the next says it's really gone. At the searching
 *  cadence below that's ~20 s to notice a KVM has been unplugged, and at the
 *  settled cadence ~2 min. */
const REACHABLE_MAX_MISSES = 1;

/** Shortest time the header's Refresh stays spinning. A discovery pass is
 *  usually near-instant, and feedback that lasts less than a blink reads as
 *  nothing having happened — the complaint the button exists to answer. */
const REFRESH_MIN_SPIN_MS = 550;

/** How often to look for KVMs while we can't see one. Short, because this is
 *  the wait a customer actually lives through: they plug the thing in and
 *  watch for it to turn up. Having to press Refresh to find out is the
 *  annoyance, and a cheap local snapshot is a fine price for not asking. */
const DISCOVERY_INTERVAL_SEARCHING_MS = 10_000;

/** …and once one is on the card. The question that needed answering has been
 *  answered; this is only keeping it honest — a KVM unplugged, a claim landing
 *  — so it can be far cheaper. It's also the expensive cadence: a pass with a
 *  KVM on the card re-reads each attached device's hand-raise over the site
 *  tunnel, whereas a searching pass has nothing to ask and stays local. */
const DISCOVERY_INTERVAL_FOUND_MS = 60_000;


/** The device's own interface classification → the Open menu's link kind.
 *
 *  A lookup rather than a chain of ternaries because it's now the one place
 *  that decides which of a KVM's addresses are offered as a way in. Anything
 *  absent here (the device's "Other" — bridges, tunnels, loopback) is not a
 *  door and is skipped. Keys mirror `service/vm/ip.go` on both models. */
const KVM_LINK_KINDS: Record<string, KvmLink["kind"] | undefined> = {
  Wired: "wired",
  Wireless: "wireless",
  USB: "usb",
};

/** Menu label per link kind. "USB" stays the bare acronym — it's what's printed
 *  on the cable the customer is looking at. */
const KVM_LINK_LABELS: Record<KvmLink["kind"], string> = {
  wired: "Ethernet",
  wireless: "Wi-Fi",
  usb: "USB",
  mesh: "Mesh",
};

class CecStore {
  /** Whether we're running in the browser preview (no backend). */
  readonly demo = !isTauri();

  version = $state<string | null>(null);
  /** Updater state — null until read, and in web mode where there's no
   *  backend. */
  updateInfo = $state<UpdateStatus | null>(null);
  /** Result of the most recent check (manual or from the background ticker). */
  updateOutcome = $state<CheckOutcome | null>(null);
  updateBusy = $state(false);
  status = $state<CecStatus | null>(null);
  /** Technician requests awaiting a decision; `request` shows the first. */
  pending = $state<ConnectRequest[]>([]);
  /** Live sessions keyed by session id. */
  sessions = $state<Record<string, LiveSession>>({});
  /** What each technician's live routes actually carry right now (canonical
   *  tech id → screen/control), pushed by the node on every change. This is
   *  the truth the "Viewing/Controlling your screen" chip renders: a session
   *  outlives the console (chat rides it), so session state alone would keep
   *  the chip lit after the technician closed the console. */
  viewing = $state<Record<string, { screen: boolean; control: boolean }>>({});
  /** Standing approvals ("who can reach me"). */
  grants = $state<Grant[]>([]);
  service = $state<ServiceStatus | null>(null);
  autostart = $state(false);
  /** When the app opens with the computer (see [`AutostartMode`]). Default is
   *  "while_granted" — it opens on boot only while a technician grant is live,
   *  so a repair survives a restart without leaving the app on the login
   *  screen forever. */
  autostartMode = $state<AutostartMode>("while_granted");
  /** Opt-in: closing the window keeps the app in the tray. Off by default —
   *  the close button really quits. */
  keepBackground = $state(false);
  /** Unix seconds, re-read each second so expiry countdowns tick. */
  now = $state(Math.floor(Date.now() / 1000));
  /** Which screen is showing. `start` is the front door (Ask for help, with
   *  the support number shown inline as a copyable fallback); `settings` is
   *  the gear. The standalone "number" screen was removed — the number never
   *  warranted a whole view of its own. */
  view = $state<"start" | "settings">("start");
  /** Whether this machine is currently asking for help on the global help
   *  room — drives the start screen's waiting card. Synced from `cec_status`
   *  and cleared live by the `cec://help` event when help arrives. */
  askingHelp = $state(false);
  /** Whether the raised hand is confirmed up. A hand is asking-room
   *  membership now: the node reports `raised: true` once the join has
   *  round-tripped through the daemon — from that moment this device is
   *  present in the queue room's signaling and every watching technician
   *  can see it. (An older node reports beacon `watchers` counts instead;
   *  any positive count means the same thing.) */
  helpRaised = $state(false);
  /** This machine's headline hardware for the spec card (null until the node
   *  answers — the card hides). Fetched once the node is up; a fresh scan
   *  each launch is plenty for a spec sheet. */
  specs = $state<MachineSpecs | null>(null);
  /** True until the one-shot spec fetch resolves — the card shows a reading
   *  skeleton meanwhile. Goes false on ANY resolution: with data the card
   *  fills in, with null (an older node without the command) it hides — a
   *  spinner must never outlive the possibility of an answer. */
  specsPending = $state(true);
  toast = $state<string | null>(null);
  busy = $state(false);

  /** Chat transcripts keyed by the technician's canonical device id. Filled
   *  from `cec_chat_history` when a chat opens and kept live by `cec://chat`. */
  chatThreads = $state<Record<string, CecChatMsg[]>>({});
  /** Unread inbound (technician) lines per tech, cleared when their chat is on
   *  screen — drives a small badge on the access-list name. */
  chatUnread = $state<Record<string, number>>({});
  /** The technician whose chat currently fills the top-left card (canonical id),
   *  or null for the normal Ask-for-help / waiting card. Auto-set to the tech on
   *  the live session while connected, cleared on disconnect; also set by hand
   *  when the customer taps a name in the access list. */
  activeChatPeer = $state<string | null>(null);

  /** The node's live mesh snapshot for the KVM & Claiming card — peers
   *  presence has found (with their claim/KVM adverts) plus our own node id
   *  (`me`). Null until the node answers; the card hides while empty. Refreshed
   *  on demand (bring-up, app refocus, the card's Refresh, and after a
   *  claim/attach), never on a steady poll — a claimable KVM is rare and the
   *  front door shouldn't hammer the node. */
  snapshot = $state<SessionSnapshot | null>(null);
  /** The set of peers the node can currently reach — canonical ids that read
   *  live (status active/shelved) on some network within the last
   *  {@link REACHABLE_MAX_MISSES}+1 discovery passes. The presence snapshot
   *  remembers a KVM's last advert even after it powers off, so this is the
   *  liveness cross-check that lets the card drop an offline KVM. `null` =
   *  reachability unknown (web mode, or the node couldn't be asked) → the card
   *  fails open and doesn't filter on it. */
  private reachable = $state<Set<string> | null>(null);
  /** When each peer (canonical id) was last seen live (active/shelved), in ms.
   *  Backs the reachability grace: a claimable KVM is genuinely `active` on the
   *  full-mesh local-claim network, but the daemon's dial-up and routine ICE
   *  churn can dip it to a transient state between our on-demand refreshes — the
   *  grace holds it "reachable" across that dip so it doesn't flicker out of the
   *  card, exactly as the AllMyStuff app's presence grace does. A plain Map (not
   *  reactive): it feeds `reachable`, which is the reactive signal. */
  private reachMisses = new Map<string, number>();
  /** Claimed-but-unattached KVMs the customer has answered "not this computer"
   *  for, keyed by canonical node id — so the "is it attached here?" prompt
   *  doesn't keep nagging. Session-local (clears on restart). */
  private attachAsked = $state<Record<string, boolean>>({});

  // ---- KVM "ask for help" ----------------------------------------------
  /** Each attached KVM's hand-raise state, keyed by canonical node id. The
   *  device is the source of truth: its own button raises the same hand, so
   *  this is re-read rather than assumed after every action. */
  kvmHelp = $state<Record<string, KvmHelpStatus>>({});
  /** Nodes with a raise/lower in flight (their button shows a pending label). */
  helpBusy = $state<Record<string, boolean>>({});

  // ---- "Open" menu (reach the KVM's own web UI) -------------------------
  /** The KVM whose Open menu is showing (its node id), or null when closed. */
  linksFor = $state<string | null>(null);
  /** The ways to reach that KVM's web UI, best-first. Empty until loaded. */
  kvmLinks = $state<KvmLink[]>([]);
  /** The address read is in flight. */
  linksLoading = $state(false);
  /** Why the LAN addresses couldn't be read, or null. The Mesh entry is still
   *  offered in that case — it doesn't depend on the device answering. */
  linksError = $state<string | null>(null);

  // ---- KVM Wi-Fi panel -------------------------------------------------
  /** The KVM whose Wi-Fi panel is open (its node id), or null when closed. The
   *  KvmClaimCard renders the modal only while this is set. */
  wifiFor = $state<string | null>(null);
  /** The open KVM's current Wi-Fi state, or null until the first read lands. */
  wifiStatus = $state<KvmWifiStatus | null>(null);
  /** Nearby networks to choose from, or null when nobody could supply a list —
   *  the picker then hides and it's manual SSID entry only. An empty array means
   *  "we looked and found nothing". Sourced from the KVM when it can scan and
   *  from this computer when it can't; see {@link wifiScanSource}. */
  wifiScan = $state<KvmWifiNetwork[] | null>(null);
  /** Where {@link wifiScan} came from, so the panel can say so. The distinction
   *  matters to whoever is reading it: the KVM's list is what the KVM can
   *  actually reach, while this computer's is a very good proxy for it — same
   *  room, same radio — but a proxy. */
  wifiScanSource = $state<"kvm" | "host" | null>(null);
  /** The network THIS computer is on, when it would tell us. Nearly always the
   *  one the KVM should join, so the panel leads with it. */
  wifiHostCurrent = $state<string | null>(null);
  /** Why this computer couldn't produce a list, when it couldn't. A fact about
   *  the operating system (macOS gates scanning behind Location access), not an
   *  error — shown only when there's no list from either side. */
  wifiHostNote = $state<string | null>(null);
  /** Reading the initial status when the panel opens. */
  wifiLoading = $state(false);
  /** A re-scan is in flight. */
  wifiScanning = $state(false);
  /** A connect / disconnect is in flight. */
  wifiBusy = $state(false);
  /** An inline message for the Wi-Fi panel (a failed or ambiguous connect),
   *  or null. Separate from the global toast so it sits next to the form. */
  wifiError = $state<string | null>(null);
  /** The resolved console tunnel port for the open panel — mapped once when it
   *  opens and reused for every call, so status / scan / connect don't each
   *  re-tunnel. Cleared on close. */
  private wifiPort: number | null = null;

  /** A pending confirmation popup — the in-app modal (never `window.confirm`,
   *  which a customer's webview may block or style inconsistently). Set by
   *  `askConfirm`; the ConfirmDialog renders it and calls `runConfirm` /
   *  `cancelConfirm`. Null when nothing is being confirmed. */
  confirmDialog = $state<{
    title: string;
    body: string;
    confirmLabel: string;
    danger?: boolean;
    onConfirm: () => void | Promise<void>;
  } | null>(null);

  private unlisteners: Array<() => void> = [];
  private timer: ReturnType<typeof setInterval> | undefined;
  private chatSyncTimer: ReturnType<typeof setInterval> | undefined;
  private discoveryTimer: ReturnType<typeof setTimeout> | undefined;
  private toastTimer: ReturnType<typeof setTimeout> | undefined;

  /** The connect request to prompt about (first pending), or null. */
  get request(): ConnectRequest | null {
    return this.pending[0] ?? null;
  }

  /** Sessions that haven't ended — connecting or active (a "requested" one is
   *  still just a prompt, not a connection). Feeds the access list's live dot
   *  and chip. */
  get liveSessions(): LiveSession[] {
    return Object.values(this.sessions).filter(
      (s) => s.state === "connecting" || s.state === "active",
    );
  }

  /** Whether this device is live on the support area — the app is up and the
   *  node answered with our number. (There is no separate "hosting" toggle
   *  now: residence on the area is standing from bring-up.) */
  get online(): boolean {
    return !!this.status?.number;
  }

  /** The customer's Support number, grouped for reading aloud (e.g. 123 456 789). */
  get grouped(): string {
    const n = this.status?.number ?? "";
    if (n.length === 9)
      return `${n.slice(0, 3)} ${n.slice(3, 6)} ${n.slice(6)}`;
    return n;
  }

  /** "Name (HOSTNAME)" — the same pair the technician's card shows, so both
   *  sides can match word for word. Empty until the node answers. */
  get computerName(): string {
    const label = this.status?.label?.trim() ?? "";
    const host = this.specs?.hostname?.trim() ?? "";
    if (label && host && label.toLowerCase() !== host.toLowerCase()) {
      return `${label} (${host})`;
    }
    return label || host || "";
  }

  /** The chat transcript with a technician (canonical id), oldest-first. */
  chatThread(peer: string): CecChatMsg[] {
    return this.chatThreads[canonicalTech(peer)] ?? [];
  }

  /** Unread inbound lines from a technician (canonical id) — the badge count. */
  chatUnreadFor(peer: string): number {
    return this.chatUnread[canonicalTech(peer)] ?? 0;
  }

  /** The technician on a live session right now (their canonical id), or null.
   *  This is the "we're connected" signal that swaps the front door for chat —
   *  the first live session's tech when there's more than one. */
  get connectedTech(): string | null {
    const s = this.liveSessions[0];
    return s ? canonicalTech(s.tech) : null;
  }

  /** Whether a technician (canonical id) is on a live session — gates whether
   *  the chat composer can actually send (chat is live-only). */
  isConnectedTo(peer: string): boolean {
    const want = canonicalTech(peer);
    return this.liveSessions.some((s) => canonicalTech(s.tech) === want);
  }

  /** The access list's rows: every standing grant, each joined to that
   *  technician's live session (if any) — plus a row for a live session whose
   *  grant hasn't landed yet (a `cec://grants` event can trail the session by a
   *  beat). One list is the single place "who can connect" and "who is
   *  connected right now" both show, so the live signal can never render
   *  somewhere the customer isn't looking. */
  get accessRows(): AccessRow[] {
    const live = this.liveSessions;
    const rows: AccessRow[] = this.grants.map((g) => {
      const key = canonicalTech(g.technician);
      return {
        key,
        technician: g.technician,
        agent_name: g.agent_name || "A CEC technician",
        grant: g,
        live: live.find((s) => canonicalTech(s.tech) === key) ?? null,
        viewing: this.viewing[key] ?? null,
      };
    });
    for (const s of live) {
      // A session event that arrived before its tech id was known can't be
      // keyed; skip it rather than render an anonymous row.
      if (!s.tech) continue;
      const key = canonicalTech(s.tech);
      if (rows.some((r) => r.key === key)) continue;
      rows.push({
        key,
        technician: s.tech,
        agent_name: s.agent_name || "A CEC technician",
        grant: null,
        live: s,
        viewing: this.viewing[key] ?? null,
      });
    }
    return rows;
  }

  /** Whether a support engagement is underway in any form: the hand is up, a
   *  technician is connected (or connecting), or standing grants exist. Drives
   *  where the KVM & Claiming card sits — the right rail during an engagement
   *  (the left column is busy with the session), the bottom of the quiet left
   *  column otherwise. */
  get engaged(): boolean {
    return this.askingHelp || this.liveSessions.length > 0 || this.grants.length > 0;
  }

  /** A friendly display name for a technician peer (canonical id): the live
   *  session's Agent Name if connected, else the standing grant's, else a
   *  generic label. */
  chatPeerName(peer: string): string {
    const want = canonicalTech(peer);
    const live = this.liveSessions.find((s) => canonicalTech(s.tech) === want);
    if (live?.agent_name) return live.agent_name;
    const grant = this.grants.find((g) => canonicalTech(g.technician) === want);
    if (grant?.agent_name) return grant.agent_name;
    return "Your technician";
  }

  /** Set on destroy so the bring-up retry loop ends with the store. */
  private stopped = false;

  async init(): Promise<void> {
    this.version = await appVersion();

    // Listeners first, so nothing the node emits during bring-up is missed.
    this.unlisteners.push(await onCecRequest((r) => this.onRequest(r)));
    this.unlisteners.push(await onCecSession((s) => this.onSession(s)));
    this.unlisteners.push(await onCecGrants((g) => (this.grants = g)));
    this.unlisteners.push(await onCecViewing((v) => (this.viewing = v)));
    this.unlisteners.push(
      await onCecChat((e) => this.appendChat(e.peer, e.message)),
    );
    this.unlisteners.push(
      await onCecHelp((e) => {
        // The node withdraws the ask itself when a session is approved (help
        // arrived) — the waiting card must follow without a manual refresh. Not
        // while a request is in flight, though: `busy` (the just-tapped ask)
        // owns the flag then, so a stale bring-up beacon can't flick the
        // optimistic card off.
        if (e.asking === false && !this.busy) this.askingHelp = false;
        // The waiting card's "raising your hand…" vs "CEC can see you"
        // signal. A new node says `raised: true` when the asking-room join
        // lands (membership IS the hand); an older node reports per-beacon
        // watcher counts — any positive count means the same thing.
        if (e.raised === true) this.helpRaised = true;
        if (typeof e.watchers === "number" && e.watchers > 0) this.helpRaised = true;
      }),
    );

    // The node comes up in parallel with this webview — on a fresh machine its
    // first start (identity generation, first-run AV scans of the sidecars)
    // takes many seconds. A single early fetch returns null and the UI would
    // sit at "Starting up…" forever over a perfectly healthy node. Keep asking
    // until the node answers with our number, then settle into event-driven
    // updates. Runs in the background so the rest of init never blocks on it.
    void this.bringUp();

    // The background self-update ticker's verdict. Registered here rather than
    // in the settings panel so a release found while the customer is anywhere
    // in the app still surfaces — the panel only mounts when they go looking,
    // which is exactly what makes an updater seem never to run.
    this.unlisteners.push(
      await onUpdateChecked((o) => this.applyUpdateChecked(o)),
    );
    void this.loadUpdateStatus();

    this.service = await serviceStatus();
    this.autostart = await autostartGet();
    this.autostartMode = await autostartModeGet();
    this.keepBackground = await backgroundGet();

    this.timer = setInterval(() => {
      this.now = Math.floor(Date.now() / 1000);
    }, 1000);

    // Re-sync the connected technician's transcript on a slow poll. The live
    // `cec://chat` stream is best-effort — a single dropped emit, a node
    // event-stream reconnect gap, or an event that lands before the chat panel
    // has bound to the peer, and that line is otherwise lost to the customer
    // forever (it's only ever appended live, never re-fetched). The node keeps
    // the full transcript, so this is the reliable backstop that guarantees the
    // customer sees the technician's replies. Connected-only and idempotent
    // (see syncActiveChat); a no-op in demo (cec_chat_history returns null).
    this.chatSyncTimer = setInterval(() => this.syncActiveChat(), 4000);

    // Presence isn't pushed to this app, so discovery has to be asked for.
    this.scheduleDiscovery();

    // Look again immediately when the app returns to the foreground, rather
    // than making someone who just brought the window up wait out a tick.
    // Re-sync the live chat then too, so a reply that arrived while the window
    // was hidden is there on return without waiting for the next poll.
    if (typeof document !== "undefined") {
      const onVisible = () => {
        if (document.visibilityState === "visible") {
          void this.refreshKvms();
          this.scheduleDiscovery();
          this.syncActiveChat();
        }
      };
      document.addEventListener("visibilitychange", onVisible);
      this.unlisteners.push(() =>
        document.removeEventListener("visibilitychange", onVisible),
      );
    }

    if (this.demo) this.loadDemo();
  }

  /** Host + fetch status until the node answers (see init). Idempotent and
   *  bounded per-iteration; ends when the number arrives or the store is
   *  destroyed. */
  private async bringUp(): Promise<void> {
    // Demo mode owns its state: loadDemo() paints it once, and this loop's
    // web-mode nulls must never race it clean — it used to eat the demo's
    // grants (the access list never showed in previews), and the moment the
    // demo status carried a number it ate the spec card too.
    if (this.demo) return;
    for (;;) {
      if (this.stopped) return;
      // Idempotent residence on the shared support area; a null (node still
      // starting) is retried on the next round.
      await cecOnline();
      await this.refresh();
      if (this.status?.number) {
        // Node's up — pull the spec card's scan once. Not part of refresh():
        // a full hardware scan per event would be waste for numbers that
        // barely move. Temps are the exception — the card's one moving
        // number — so they get their own lazy poll against the node's
        // scan-free machine_temps.
        this.specs = await machineSpecs();
        this.specsPending = false;
        // The node's up — do a first KVM/claim discovery pass too, so a CEC
        // KVM the customer has plugged in shows up without waiting for a
        // refocus or the card's Refresh. Follow it with the same short burst a
        // claim/attach uses: when THIS app just started the node, its daemon is
        // still dialing a freshly discovered claimable on the local-claim mesh,
        // so the very first reachability sample can land before the link is
        // `active` — the burst re-checks over the next few seconds and catches
        // it (and the grace then holds it), instead of leaving the claimable
        // invisible until a manual refresh. This is the fleetless-customer fix.
        void this.refreshKvms();
        void this.refreshKvmsSoon();
        // Temperature display is parked until it's more accurate and on a
        // 5-second poll (the spec card hides the row for now), so there's
        // nothing to refresh — don't run the old 30s temp poll in the
        // meantime. The node's machine_temps command + the machineTemps
        // bridge stay wired, so re-enabling is just restoring the poll here.
        return;
      }
      await new Promise((r) => setTimeout(r, 2000));
    }
  }

  destroy(): void {
    this.stopped = true;
    for (const un of this.unlisteners) un();
    this.unlisteners = [];
    if (this.timer) clearInterval(this.timer);
    if (this.chatSyncTimer) clearInterval(this.chatSyncTimer);
    if (this.discoveryTimer) clearTimeout(this.discoveryTimer);
    if (this.toastTimer) clearTimeout(this.toastTimer);
  }

  /** Queue the next discovery pass, at a cadence that follows the answer:
   *  {@link DISCOVERY_INTERVAL_SEARCHING_MS} while the card has no KVM,
   *  {@link DISCOVERY_INTERVAL_FOUND_MS} once it does.
   *
   *  A self-rescheduling timeout rather than setInterval, for two reasons: the
   *  interval has to be re-decided after every pass (the whole point), and a
   *  fixed interval would stack passes on top of each other whenever one ran
   *  long — which the settled cadence can, since it talks to the device.
   *
   *  Silent by construction: `refreshKvms`, never `refreshKvmsVisibly`. The
   *  header control spins because a person pressed it. A spinner going off by
   *  itself every ten seconds is exactly the flicker the button was added to
   *  replace. Clears any pending pass first, so callers that want the clock
   *  restarted (a manual Refresh, returning to the foreground) just call it. */
  private scheduleDiscovery(): void {
    if (this.discoveryTimer) clearTimeout(this.discoveryTimer);
    this.discoveryTimer = undefined;
    if (this.stopped || this.demo) return;
    // Settled cadence only while the answer is settled. A peer part-way
    // through its misses is a KVM we're still drawing that didn't read live
    // last pass — most often one just unplugged — and confirming that needs
    // one more sample. At a minute apiece it would sit on the card for two
    // minutes after the cable came out, which is the "it never goes away"
    // complaint with extra steps. Unsure is a reason to look sooner.
    const unsure = [...this.reachMisses.values()].some((n) => n > 0);
    const ms =
      this.cecKvms.length && !unsure
        ? DISCOVERY_INTERVAL_FOUND_MS
        : DISCOVERY_INTERVAL_SEARCHING_MS;
    this.discoveryTimer = setTimeout(() => {
      void (async () => {
        if (this.stopped) return;
        // Don't stack onto a pass the Refresh button is already running.
        if (!this.kvmRefreshing) {
          // Swallowed: a snapshot that fails mid-poll is a transient the next
          // pass retries. Nothing here was asked for, so nothing here gets to
          // put an error in front of the customer.
          try {
            await this.refreshKvms();
          } catch {
            /* next tick */
          }
        }
        this.scheduleDiscovery();
      })();
    }, ms);
  }

  async refresh(): Promise<void> {
    this.status = await cecStatus();
    this.pending = await cecPending();
    this.grants = await cecGrants();
    // Pull the live viewing map too, so an app that starts (or reconnects)
    // mid-session paints the chip without waiting for the next transition.
    this.viewing = await cecViewing();
    // The node is the truth for the ask (it withdraws it itself on approval,
    // and a restart drops it) — mirror it whenever the status lands, but never
    // mid-request: an in-flight ask/cancel (busy) owns the flag, so a status
    // poll landing before the node has registered a just-tapped ask can't stomp
    // the optimistic "Raising your hand…" card back to the front door.
    if (this.status && !this.busy) this.askingHelp = this.status.asking_help === true;
  }

  private async loadGrants(): Promise<void> {
    this.grants = await cecGrants();
  }

  // ---- event handlers --------------------------------------------------

  private onRequest(r: ConnectRequest): void {
    // Remember the request's details keyed by session so the banner can name
    // the technician once the session goes live.
    this.sessions = {
      ...this.sessions,
      [r.session_id]: {
        session_id: r.session_id,
        state: "requested",
        tech: r.tech,
        agent_name: r.agent_name,
        want_control: r.want_control,
      },
    };
    this.pending = [
      ...this.pending.filter((p) => p.session_id !== r.session_id),
      r,
    ];
  }

  private onSession(s: SessionEvent): void {
    if (s.state === "ended") {
      const next = { ...this.sessions };
      delete next[s.session_id];
      this.sessions = next;
      this.pending = this.pending.filter((p) => p.session_id !== s.session_id);
      void this.loadGrants();
      // Disconnected — swap the chat card back for the front door (or the
      // next live tech, if more than one was connected).
      this.reconcileChatPanel();
      return;
    }
    const prev = this.sessions[s.session_id];
    this.sessions = {
      ...this.sessions,
      [s.session_id]: {
        session_id: s.session_id,
        state: s.state,
        // Prefer the event's own `tech`/`agent_name` (the node includes them on
        // an auto-approve, where there was no `cec://request` to learn them
        // from) and only fall back to a prior request's values. Without this an
        // auto-approved reconnect bound the chat to an empty tech id, so the
        // panel showed an empty thread while the technician's lines filed under
        // their real id — invisible.
        tech: s.tech ?? prev?.tech ?? "",
        agent_name: s.agent_name ?? prev?.agent_name ?? "Your technician",
        want_control: prev?.want_control ?? false,
      },
    };
    // Once a session is connecting/active the decision has been made, so drop
    // any lingering prompt for it.
    if (s.state === "active" || s.state === "connecting") {
      this.pending = this.pending.filter((p) => p.session_id !== s.session_id);
      // Connected — the front door becomes the chat with this technician.
      this.reconcileChatPanel();
    }
  }

  /** Keep the top-left card's chat/front-door state in step with the live
   *  sessions: no session → the Ask-for-help card returns; a live session with
   *  nobody shown (or the shown tech dropped) → open the connected tech's chat.
   *  A chat the customer opened by hand (a still-connected tech) is left alone. */
  private reconcileChatPanel(): void {
    const live = this.liveSessions;
    if (live.length === 0) {
      this.activeChatPeer = null;
      return;
    }
    if (
      this.activeChatPeer &&
      live.some((s) => canonicalTech(s.tech) === this.activeChatPeer)
    ) {
      return;
    }
    const peer = canonicalTech(live[0].tech);
    // No usable technician id yet (a session event that arrived before its
    // tech was known) — leave the card as-is rather than binding the chat to an
    // empty key, which shows an empty thread the real lines never reach.
    if (!peer) return;
    this.activeChatPeer = peer;
    this.markChatRead(peer);
    void this.loadChatHistory(peer);
  }

  // ---- actions ---------------------------------------------------------

  async approve(req: ConnectRequest, scope: ApprovalScope): Promise<void> {
    this.busy = true;
    try {
      await cecApprove(req.tech, scope, req.session_id, req.want_control);
      this.pending = this.pending.filter(
        (p) => p.session_id !== req.session_id,
      );
      await this.loadGrants();
      this.notify(
        scope === "once"
          ? "Approved for this session."
          : scope === "three_hours"
            ? "Approved. They can reconnect for the next 3 hours."
            : "Approved. They can reconnect until you remove them.",
      );
    } catch (e) {
      this.notify(`Couldn't approve: ${errMsg(e)}`);
    } finally {
      this.busy = false;
    }
  }

  async deny(req: ConnectRequest): Promise<void> {
    this.busy = true;
    try {
      await cecDeny(req.tech, req.session_id);
      this.pending = this.pending.filter(
        (p) => p.session_id !== req.session_id,
      );
      const next = { ...this.sessions };
      delete next[req.session_id];
      this.sessions = next;
    } catch (e) {
      this.notify(`Couldn't decline: ${errMsg(e)}`);
    } finally {
      this.busy = false;
    }
  }

  /** Forget a technician entirely: end any live session they're on right now,
   *  revoke their standing approval, and drop their node from the mesh. One
   *  button, one outcome — they're off the screen and can't come back without
   *  asking again. Bites immediately. */
  async forget(tech: string): Promise<void> {
    const want = canonicalTech(tech);
    const live = this.liveSessions.filter((s) => canonicalTech(s.tech) === want);
    // Disconnect first (the same End the wire uses for a decline), so the
    // session closes cleanly before the node is dropped from the mesh.
    for (const s of live) {
      try {
        await cecDeny(s.tech, s.session_id);
      } catch {
        // The revoke + forget below still cut access; a failed End just means
        // the transport teardown does the disconnecting.
      }
      const next = { ...this.sessions };
      delete next[s.session_id];
      this.sessions = next;
    }
    if (live.length > 0) this.reconcileChatPanel();
    try {
      await cecRevoke(tech);
      await cecForgetNode(tech);
    } catch (e) {
      this.notify(`Couldn't remove: ${errMsg(e)}`);
    }
    await this.loadGrants();
    this.notify(
      live.length > 0
        ? "Disconnected and removed. They can't reconnect without asking you again."
        : "Removed. They can't reconnect without asking you again.",
    );
  }

  /** "Ask for help": raise this machine's hand on the support area until a
   *  technician connects or the customer cancels. The node ensures area
   *  residence as part of the ask, so a tap on a fresh launch still just works. */
  async askHelp(): Promise<void> {
    // A fresh ask starts unconfirmed — the card shows "raising your
    // hand…" until the node reports the asking-room join landed.
    this.helpRaised = false;
    if (this.demo) {
      this.askingHelp = true;
      // Act out the real sequence: a couple of seconds of hand-raising,
      // then the hand is confirmed up.
      setTimeout(() => {
        if (this.askingHelp) this.helpRaised = true;
      }, 2500);
      return;
    }
    // Flip to the "Raising your hand…" card the instant they tap — BEFORE the
    // node round-trip. Ensuring area residence as part of the ask can take a few
    // seconds on a cold launch, and awaiting it first left the front door sitting
    // on a disabled button with no visible change, which reads as a freeze. The
    // card's spinner + reassurance is the comforting "we've got you" moment, so
    // show it immediately; if the ask fails we drop back to the front door with a
    // message.
    this.askingHelp = true;
    this.busy = true;
    try {
      await cecAskHelp(true);
    } catch (e) {
      this.askingHelp = false;
      this.notify(`Couldn't ask for help: ${errMsg(e)}`);
    } finally {
      this.busy = false;
    }
  }

  /** Withdraw the ask ("Stop asking"). */
  async cancelHelp(): Promise<void> {
    if (this.demo) {
      this.askingHelp = false;
      return;
    }
    this.busy = true;
    try {
      await cecAskHelp(false);
      this.askingHelp = false;
    } catch (e) {
      this.notify(`Couldn't stop the request: ${errMsg(e)}`);
    } finally {
      this.busy = false;
    }
  }

  async setLabel(label: string): Promise<void> {
    await cecSetLabel(label);
    this.status = await cecStatus();
    this.notify("Saved this computer's name.");
  }

  // ---- chat (live, while a technician is connected) --------------------

  /** Show a technician's chat in the top-left card (their device id), priming
   *  its history. Used by the access-list name tap; while connected the panel
   *  also opens on its own (see {@link reconcileChatPanel}). */
  openChat(peer: string): void {
    const key = canonicalTech(peer);
    this.activeChatPeer = key;
    this.markChatRead(key);
    void this.loadChatHistory(key);
  }

  /** Dismiss the chat card. While a session is still live this returns to that
   *  technician's chat (you can't hide the only way to talk to whoever's on your
   *  screen); otherwise the Ask-for-help front door returns. */
  closeChat(): void {
    this.activeChatPeer = null;
    this.reconcileChatPanel();
  }

  /** Clear a technician's unread badge — their chat is on screen. */
  markChatRead(peer: string): void {
    const key = canonicalTech(peer);
    if (!this.chatUnread[key]) return;
    const next = { ...this.chatUnread };
    delete next[key];
    this.chatUnread = next;
  }

  /** Load the persisted transcript with a technician and fold it into the local
   *  thread, oldest-first. Null-tolerant: a failed fetch keeps what we have, and
   *  any live line that beat the reply is preserved (merged + de-duped by id). */
  async loadChatHistory(peer: string): Promise<void> {
    const key = canonicalTech(peer);
    const msgs = await cecChatHistory(key);
    if (!msgs) return;
    const seen = new Set(msgs.map((m) => m.id));
    const extra = (this.chatThreads[key] ?? []).filter((m) => {
      if (seen.has(m.id)) return false;
      // A still-pending optimistic line the fetched transcript already carries
      // (same side + text) must be dropped, or a re-sync doubles the sender's
      // own bubble the moment the node-assigned copy lands in history.
      if (m.id.startsWith("local-")) {
        return !msgs.some((s) => s.from === m.from && s.text === m.text);
      }
      return true;
    });
    const merged = [...msgs, ...extra].sort((a, b) => a.ts - b.ts);
    // Skip the write when nothing changed, so the periodic re-sync doesn't
    // churn the thread array — a fresh reference would re-fire the chat panel's
    // scroll-to-end effect every tick and fight a customer scrolled up to read.
    const current = this.chatThreads[key] ?? [];
    const unchanged =
      merged.length === current.length &&
      merged.every((m, i) => current[i]?.id === m.id);
    if (unchanged) return;
    this.chatThreads = { ...this.chatThreads, [key]: merged };
  }

  /** Re-pull the connected technician's transcript so any line the best-effort
   *  live `cec://chat` event missed still appears. Connected-only, so it's idle
   *  whenever no session is live; {@link loadChatHistory} is idempotent, so a
   *  repeated pull with nothing new is a no-op. */
  private syncActiveChat(): void {
    const tech = this.connectedTech;
    if (tech) void this.loadChatHistory(tech);
  }

  /** Send a line to a technician. Appends it optimistically (from "client", so
   *  the bubble shows the instant Enter is pressed), then calls the node and
   *  reconciles the temporary row to the node-assigned id/ts. The node echoes
   *  the line back over `cec://chat`; {@link appendChat} dedupes that echo. */
  async sendChat(peer: string, text: string): Promise<void> {
    const body = text.trim();
    if (!body) return;
    const key = canonicalTech(peer);
    const tempId = `local-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    this.appendChat(key, {
      id: tempId,
      from: "client",
      text: body,
      // UNIX seconds, to match the node's `ts` (the thread sorts by it).
      ts: Math.floor(Date.now() / 1000),
    });
    if (this.demo) {
      // Act out a technician replying a beat later so the preview chat feels
      // alive without a backend.
      setTimeout(() => {
        this.appendChat(key, {
          id: `demo-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
          from: "technician",
          text: "Thanks, I can see that. Give me one moment.",
          ts: Math.floor(Date.now() / 1000),
        });
      }, 1400);
      return;
    }
    const r = await cecChatSend(key, body);
    // Live-only: a null means there's no session to carry it (or web mode) —
    // keep the calm optimistic line rather than surfacing an error.
    if (!r?.id) return;
    const thread = this.chatThreads[key] ?? [];
    if (
      thread.some((m) => m.id === tempId) &&
      !thread.some((m) => m.id === r.id)
    ) {
      this.chatThreads = {
        ...this.chatThreads,
        [key]: thread.map((m) =>
          m.id === tempId ? { ...m, id: r.id, ts: r.ts } : m,
        ),
      };
    }
  }

  /** Append one line to a technician's thread, deduped by id. Our own line
   *  (client) is echoed back by the node — the still-pending optimistic copy is
   *  collapsed in place rather than doubled. An inbound technician line bumps the
   *  unread badge unless that chat is the one on screen. */
  private appendChat(peer: string, msg: CecChatMsg): void {
    const key = canonicalTech(peer);
    let thread = this.chatThreads[key] ?? [];
    if (thread.some((m) => m.id === msg.id)) return;
    if (msg.from === "client") {
      const pending = thread.findIndex(
        (m) =>
          m.id.startsWith("local-") && m.from === "client" && m.text === msg.text,
      );
      if (pending >= 0) {
        thread = thread.map((m, i) => (i === pending ? msg : m));
        this.chatThreads = { ...this.chatThreads, [key]: thread };
        return;
      }
    }
    this.chatThreads = { ...this.chatThreads, [key]: [...thread, msg] };
    if (msg.from === "technician" && this.activeChatPeer !== key) {
      this.chatUnread = {
        ...this.chatUnread,
        [key]: (this.chatUnread[key] ?? 0) + 1,
      };
    }
  }

  // ---- background service ----------------------------------------------

  async installService(): Promise<void> {
    this.busy = true;
    try {
      const r = await serviceInstall();
      this.service = await serviceStatus();
      this.notify(r?.output ?? "Background service installed.");
    } finally {
      this.busy = false;
    }
  }

  async uninstallService(): Promise<void> {
    this.busy = true;
    try {
      const r = await serviceUninstall();
      this.service = await serviceStatus();
      this.notify(r?.output ?? "Background service removed.");
    } finally {
      this.busy = false;
    }
  }

  async stopService(): Promise<void> {
    this.busy = true;
    try {
      const r = await serviceStop();
      this.service = await serviceStatus();
      this.notify(r?.output ?? "Background service stopped.");
    } finally {
      this.busy = false;
    }
  }

  async setAutostart(on: boolean): Promise<void> {
    this.autostart = await autostartSet(on);
  }

  async setAutostartMode(mode: AutostartMode): Promise<void> {
    this.autostartMode = mode;
    await autostartModeSet(mode);
  }

  async setKeepBackground(on: boolean): Promise<void> {
    this.keepBackground = await backgroundSet(on);
  }

  // ---- KVM & claiming ("KVM and Claiming" card) ------------------------
  //
  // A KVM is a NanoKVM-class appliance the customer plugs into their machine:
  // it advertises FEATURE_KVM and offers itself for adoption. The customer app
  // surfaces any claimable KVM the node sees, exactly as AllMyStuff does. The
  // card walks it through claim → "is it on this computer?" → attached, then
  // offers Update (the appliance's own firmware update, tunnelled through its
  // web UI, falling back to a reboot when it's already current) and Wi-Fi setup
  // (reading and setting the KVM's own Wi-Fi over that same tunnel; see the KVM
  // Wi-Fi section below). Every backend step is an existing node command or a
  // tunnelled call to the KVM's web API; the node/appliance is the source of
  // truth and confirms each by re-advertising or re-reporting its state.

  /** This computer's own mesh node id (the attach-to-this-computer target),
   *  from the snapshot. Empty until the first snapshot lands. */
  get localId(): string {
    return this.snapshot?.me ?? "";
  }

  /** The claimable/claimed KVMs to surface. Rides the same predicate the
   *  AllMyStuff app uses for claimables — a KVM appliance (`FEATURE_KVM`) that
   *  is either offering itself for adoption (claimable and not already someone
   *  else's) or already ours. No CEC-vs-normal distinction: any claimable KVM
   *  the node can see shows here, exactly as it does in AllMyStuff. Projected
   *  against our own id so the card is a dumb view of the lifecycle. */
  get cecKvms(): CecKvm[] {
    const me = this.localId;
    const out: CecKvm[] = [];
    for (const p of this.snapshot?.peers ?? []) {
      if (!(p.features ?? []).includes(FEATURE_KVM)) continue;
      if (this.sameNode(p.node, me)) continue; // never ourselves
      // Drop KVMs the node can no longer reach (offline). Presence remembers a
      // KVM's last advert; `reachable` is the live cross-check. Unknown (null)
      // = fail open, don't filter.
      if (this.reachable && !this.reachable.has(canonicalTech(p.node))) continue;
      const mine = this.sameNode(p.owner ?? null, me);
      const ownedByOther = !!p.owner && !mine;
      // Offering itself for adoption (and not already someone else's), or ours.
      if (!mine && !(p.claimable && !ownedByOther)) continue;
      const attachedHere = this.sameNode(p.kvm?.attached_to ?? null, me);
      out.push({
        node: p.node,
        label: p.label || "KVM",
        claimable: !!p.claimable,
        mine,
        attachedHere,
        promptAttach: mine && !attachedHere && !this.attachAsked[canonicalTech(p.node)],
        hasWeb: !!this.kvmWebSite(p),
      });
    }
    return out;
  }

  /** Whether two mesh ids name the same machine (same pubkey, any suffix) —
   *  canonicalized identically to the chat thread keys. */
  private sameNode(a: string | null | undefined, b: string | null | undefined): boolean {
    if (!a || !b) return false;
    return canonicalTech(a) === canonicalTech(b);
  }

  /** The site serving a KVM's own web UI — the one whose id matches `kvm.web`,
   *  else the first web-scheme site. Undefined when the KVM advertises none
   *  (Update then has nowhere to POST). */
  private kvmWebSite(p: MeshPeer): SiteAdvert | undefined {
    const sites = p.sites ?? [];
    const named = p.kvm?.web ? sites.find((s) => s.id === p.kvm!.web) : undefined;
    return named ?? sites.find((s) => this.siteIsWeb(s));
  }

  private siteIsWeb(s: SiteAdvert): boolean {
    return s.scheme === "http" || s.scheme === "https";
  }

  /** Pull a fresh mesh snapshot (the KVM card's whole data source). On-demand
   *  only — see `snapshot`. */
  /** True while a discovery pass is running — drives the header Refresh's
   *  spin. Held for at least {@link REFRESH_MIN_SPIN_MS} so the control always
   *  reads as having done something: a snapshot usually returns in a few
   *  milliseconds, and a spinner that appears and vanishes within one frame is
   *  indistinguishable from a button that did nothing at all. */
  kvmRefreshing = $state(false);

  /** Look again for KVMs, visibly. The button's whole job is to be believable,
   *  so this owns the minimum-spin rather than leaving each caller to fake it;
   *  the silent background callers use {@link refreshKvms} directly. */
  async refreshKvmsVisibly(): Promise<void> {
    if (this.kvmRefreshing) return;
    this.kvmRefreshing = true;
    const started = Date.now();
    try {
      await this.refreshKvms();
    } finally {
      const held = Date.now() - started;
      if (held < REFRESH_MIN_SPIN_MS) {
        await new Promise((r) => setTimeout(r, REFRESH_MIN_SPIN_MS - held));
      }
      this.kvmRefreshing = false;
      // Someone just looked; restart the clock rather than letting a queued
      // background pass land a second or two behind them.
      this.scheduleDiscovery();
    }
  }

  async refreshKvms(): Promise<void> {
    if (this.demo) return;
    const snap = await sessionSnapshot();
    if (snap) this.snapshot = snap;
    await this.refreshReachable();
    // Pick up hand-raises made at the device itself — the physical button
    // raises the same hand this app does, so the card must reflect it whichever
    // one was pressed.
    await this.refreshKvmHelp();
  }

  /** Re-read the hand-raise state of every KVM linked to this computer. Runs
   *  off the same discovery pass as the card, and stays quiet on failure. */
  private async refreshKvmHelp(): Promise<void> {
    await Promise.all(
      this.cecKvms.filter((k) => k.mine && k.attachedHere && k.hasWeb).map((k) => this.loadKvmHelp(k.node)),
    );
  }

  /** Recompute which peers the node can reach across every network it's on —
   *  the liveness cross-check that lets the card drop an offline KVM (see
   *  `reachable`). A peer counts as reachable if it read live (active/shelved)
   *  on some network within the last {@link REACHABLE_MAX_MISSES}+1 passes.
   *  Tolerating one miss is what lets a *just-plugged-in* KVM show: it reads
   *  `active` only after a second or two of `sighted`/`handshaking` while the
   *  daemon dials it on the full-mesh local-claim network, so a single
   *  point-in-time check races that dial-up and culls a device that is plainly
   *  there (the fleetless-customer bug). An explicit `offline`/`error` drops it
   *  at once, so a powered-off KVM goes without waiting out the misses.
   *  Fail-open: if the node can't be asked for its networks,
   *  reachability is left unknown (null) and nothing is filtered on it. */
  private async refreshReachable(): Promise<void> {
    const nets = await meshNetworks();
    if (!nets) {
      this.reachable = null;
      return;
    }
    // Best status per machine ACROSS networks, so an `offline` row on one mesh
    // can't erase the grace an `active` row on another just earned (the same
    // machine-wide rule the AllMyStuff poll uses).
    const activeCanons = new Set<string>();
    const offlineCanons = new Set<string>();
    for (const net of nets) {
      const peers = await meshPeers(net.network_id);
      if (!peers) continue;
      for (const pr of peers) {
        const canon = canonicalTech(pr.device_id);
        if (pr.status === "active" || pr.status === "shelved") {
          activeCanons.add(canon);
        } else if (pr.status === "offline" || pr.status === "error") {
          offlineCanons.add(canon);
        }
      }
    }
    // Count MISSED SAMPLES, not elapsed time. Wall-clock ageing can't work when
    // sampling is user-driven: too short and a KVM vanishes because nobody
    // pressed Refresh; too long (or never) and a KVM that was switched off
    // hangs around forever, because the presence snapshot keeps serving its
    // last advert and nothing else culls it. A miss count is relative to when
    // we actually looked, so it behaves the same whether the next look is in
    // one second or ten minutes.
    for (const canon of this.reachMisses.keys()) {
      if (!activeCanons.has(canon)) {
        this.reachMisses.set(canon, (this.reachMisses.get(canon) ?? 0) + 1);
      }
    }
    for (const canon of activeCanons) this.reachMisses.set(canon, 0);
    // A peer the daemon explicitly calls offline/error (and that isn't live on
    // some other network) goes at once — no waiting out the misses.
    for (const canon of offlineCanons) {
      if (!activeCanons.has(canon)) this.reachMisses.delete(canon);
    }
    // One miss is tolerated, the second drops it. A KVM reads active only after
    // a second or two of sighted/handshaking while the daemon dials it, so a
    // single look can land mid-dial on a device that is plainly there — that
    // one miss must not cull it. Two consecutive looks with no life is a
    // different claim, and the honest one: switch a KVM off and it is gone on
    // the second refresh rather than sitting in the card forever.
    const live = new Set<string>();
    for (const [canon, misses] of this.reachMisses) {
      if (misses <= REACHABLE_MAX_MISSES) live.add(canon);
      else this.reachMisses.delete(canon);
    }
    this.reachable = live;
  }

  /** Refresh a few times over the next few seconds. A claim/attach is confirmed
   *  by the KVM *re-advertising* its new owner/binding, which lands
   *  asynchronously — one immediate read would miss it. */
  private async refreshKvmsSoon(): Promise<void> {
    for (const ms of [700, 1500, 3000, 5000]) {
      if (this.stopped) return;
      await new Promise((r) => setTimeout(r, ms));
      await this.refreshKvms();
    }
  }

  /** Adopt a claimable CEC KVM. The KVM confirms by re-advertising us as its
   *  owner; the card then offers the "attached to this computer?" prompt. */
  async claimKvm(node: string): Promise<void> {
    if (this.busy) return;
    if (this.demo) {
      this.demoPatchKvm(node, (p) => {
        p.owner = this.snapshot?.me ?? "me";
        p.claimable = false;
      });
      this.notify("Claimed the KVM.");
      return;
    }
    this.busy = true;
    try {
      await claimNode(node);
      this.notify("Claiming the KVM…");
      void this.refreshKvmsSoon();
    } catch (e) {
      this.notify(`Couldn't claim the KVM: ${errMsg(e)}`);
    } finally {
      this.busy = false;
    }
  }

  /** Point a claimed KVM at this computer — the customer answered "yes, it's
   *  attached here". The KVM confirms by re-advertising its binding. */
  async attachKvmHere(node: string): Promise<void> {
    if (this.busy) return;
    if (this.demo) {
      this.demoPatchKvm(node, (p) => {
        p.kvm = { ...(p.kvm ?? {}), attached_to: this.snapshot?.me ?? "me" };
      });
      this.attachAsked = { ...this.attachAsked, [canonicalTech(node)]: true };
      this.notify("Linked the KVM to this computer.");
      return;
    }
    const me = this.localId;
    if (!me) {
      this.notify("Still finding this computer on the mesh. Try again in a moment.");
      return;
    }
    this.busy = true;
    try {
      await kvmAttach(node, me);
      this.attachAsked = { ...this.attachAsked, [canonicalTech(node)]: true };
      this.notify("Linked the KVM to this computer.");
      void this.refreshKvmsSoon();
    } catch (e) {
      this.notify(`Couldn't link the KVM: ${errMsg(e)}`);
    } finally {
      this.busy = false;
    }
  }

  /** The customer answered "not this computer" — stop asking (until they claim
   *  another or restart). */
  dismissAttachPrompt(node: string): void {
    this.attachAsked = { ...this.attachAsked, [canonicalTech(node)]: true };
  }

  /** Update this KVM's firmware, then restart it.
   *
   *  This replaced a Reboot button that POSTed the GPIO reset — which resets
   *  the machine on the other end of the KVM, not the appliance. This panel
   *  manages the appliance.
   *
   *  "Already current" is deliberately not a no-op: the KVM is rebooted anyway,
   *  so one button covers both jobs and nobody has to reason about which to
   *  press. Applying an update doesn't need that second step — the device
   *  restarts its own server (and the mesh daemon, when the bundle carries a
   *  changed one) as the last act of installing, in an order it has good
   *  reasons for. Issuing our own reboot on top would race that, so the two
   *  paths end the same way by different means. */
  async updateKvm(node: string): Promise<void> {
    if (this.busy) return;
    if (this.demo) {
      this.notify("Update sent to the KVM.");
      return;
    }
    this.busy = true;
    try {
      const port = await this.kvmConsolePort(node);
      if (port === null) {
        this.notify("This KVM hasn't published a console yet, so it can't be updated.");
        return;
      }
      const { rsp, reason, localPort } = await this.kvmApi<KvmVersion>(port, "/api/application/version");
      if (!rsp) {
        this.notify(`Couldn't check this KVM's firmware: ${reason}`);
        return;
      }
      if (rsp.code !== 0) {
        this.notify(`Couldn't check this KVM's firmware: ${this.kvmMsg(rsp, "the KVM declined.")}`);
        return;
      }
      const current = (rsp.data?.current ?? "").trim();
      const latest = (rsp.data?.latest ?? "").trim();
      // No `latest` means the device couldn't reach its release channel, not
      // that it's behind — so don't claim an update exists. The reboot still
      // happens, which is the half of this button that never needs the network.
      if (latest && latest !== current) {
        await this.applyKvmUpdate(localPort, latest);
      } else {
        await this.rebootKvmDevice(localPort, current);
      }
    } finally {
      this.busy = false;
    }
  }

  /** Install `latest` on the KVM and let it restart itself.
   *
   *  The device downloads, verifies and installs the bundle BEFORE it answers,
   *  so this waits far longer than a normal call. A timeout here is not a
   *  failure: the install may well have landed and taken the tunnel down with
   *  the restart that follows it, so it reads as "under way". */
  private async applyKvmUpdate(port: number, latest: string): Promise<void> {
    this.notify(`Installing ${latest} on the KVM — this takes a few minutes.`);
    const { rsp, timedOut, reason } = await this.kvmApi(port, "/api/application/update", {
      method: "POST",
      body: {},
      timeoutMs: 300_000,
    });
    if (timedOut) {
      this.notify(`The KVM is still installing ${latest}. It restarts on its own when it's done.`);
      return;
    }
    if (!rsp) {
      this.notify(`Couldn't update the KVM: ${reason}`);
      return;
    }
    if (rsp.code !== 0) {
      this.notify(`Couldn't update the KVM: ${this.kvmMsg(rsp, "the KVM declined.")}`);
      return;
    }
    this.notify(`KVM updated to ${latest}. It's restarting now.`);
  }

  /** Reboot the KVM appliance itself — the up-to-date half of Update.
   *
   *  A reboot takes the device down mid-request by definition, so a timeout is
   *  the expected outcome rather than a fault. */
  private async rebootKvmDevice(port: number, current: string): Promise<void> {
    const on = current ? ` (${current})` : "";
    const { rsp, timedOut, reason } = await this.kvmApi(port, "/api/vm/system/reboot", {
      method: "POST",
      body: {},
    });
    if (timedOut || (rsp && rsp.code === 0)) {
      this.notify(`KVM already on the latest firmware${on} — rebooting it now.`);
      return;
    }
    if (!rsp) {
      this.notify(`KVM already on the latest firmware${on}, but the reboot failed: ${reason}`);
      return;
    }
    this.notify(
      `KVM already on the latest firmware${on}, but the reboot failed: ${this.kvmMsg(rsp, "the KVM declined.")}`,
    );
  }

  // ---- KVM "ask for help" ----------------------------------------------
  //
  // A KVM raises its hand on the shared CEC support area in its own right — it
  // is a help-seeker like a customer's app, not a thing this app raises a hand
  // *about*. The device's physical button does exactly the same, so this is a
  // second way into one path, and the state below is always re-read from the
  // appliance rather than inferred from what we just asked for.
  //
  // A technician who answers is authorised for a bounded window (three hours,
  // enforced and persisted on the device), which is why this surface shows a
  // deadline rather than an open-ended "someone may be connected".

  /** The hand-raise state for `node`, or null until first read. */
  helpFor(node: string): KvmHelpStatus | null {
    return this.kvmHelp[canonicalTech(node)] ?? null;
  }

  /** Whether a raise/lower is in flight for `node`. */
  helpPending(node: string): boolean {
    return !!this.helpBusy[canonicalTech(node)];
  }

  /** What's left of a KVM's support authorisation, as "2h 58m" — or null when
   *  no grant is outstanding. Reads the store's one-second `now` so it ages
   *  itself without a second timer. Rounds up, so a live grant never reads as
   *  "0m left" in the seconds before it lapses. */
  helpTimeLeft(node: string): string | null {
    const st = this.helpFor(node);
    if (!st?.authorised || !st.expiresAt) return null;
    const secs = st.expiresAt - this.now;
    if (secs <= 0) return null;
    const mins = Math.ceil(secs / 60);
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }

  /** How long a grant lasts on this device, phrased for a sentence ("3 hours").
   *  Taken from the appliance so the app never contradicts the real policy. */
  helpWindowLabel(node: string): string {
    const secs = this.helpFor(node)?.grantSeconds ?? 0;
    if (secs <= 0) return "a set time";
    const h = Math.round(secs / 3600);
    if (h >= 1) return h === 1 ? "1 hour" : `${h} hours`;
    const m = Math.max(1, Math.round(secs / 60));
    return m === 1 ? "1 minute" : `${m} minutes`;
  }

  /** Read a KVM's hand-raise state. Quiet on failure — this is ambient status
   *  the card simply omits when unavailable, not something the customer asked
   *  for, so a toast would be noise. */
  async loadKvmHelp(node: string): Promise<void> {
    if (this.demo) {
      this.kvmHelp = {
        ...this.kvmHelp,
        [canonicalTech(node)]: {
          enabled: true,
          asking: false,
          supportId: "123456789",
          authorised: false,
          grantSeconds: 10800,
        },
      };
      return;
    }
    const port = await this.kvmConsolePort(node);
    if (!port) return;
    const { rsp } = await this.kvmApi<KvmHelpStatus>(port, "/api/mesh/help");
    if (!rsp || rsp.code !== 0 || !rsp.data) return;
    this.kvmHelp = { ...this.kvmHelp, [canonicalTech(node)]: rsp.data };
  }

  /** Raise or lower a KVM's hand. The reply carries the resulting state, so one
   *  round-trip both acts and refreshes. */
  async toggleKvmHelp(node: string): Promise<void> {
    const key = canonicalTech(node);
    if (this.helpBusy[key]) return;
    const raised = this.helpFor(node)?.asking ?? false;

    if (this.demo) {
      const cur = this.helpFor(node);
      this.kvmHelp = {
        ...this.kvmHelp,
        [key]: { ...(cur ?? { enabled: true, supportId: "123456789", authorised: false, grantSeconds: 10800 }), asking: !raised } as KvmHelpStatus,
      };
      this.notify(raised ? "Cancelled the help request." : "The KVM has raised its hand.");
      return;
    }

    const port = await this.kvmConsolePort(node);
    if (!port) {
      this.notify("Couldn't reach the KVM's console.");
      return;
    }
    this.helpBusy = { ...this.helpBusy, [key]: true };
    try {
      const { rsp, reason } = await this.kvmApi<KvmHelpStatus>(
        port,
        raised ? "/api/mesh/help/lower" : "/api/mesh/help/raise",
        { method: "POST" },
      );
      if (!rsp || rsp.code !== 0) {
        this.notify(
          rsp
            ? this.kvmMsg(rsp, "The KVM couldn't ask for help.")
            : (reason ?? "The KVM couldn't ask for help."),
        );
        return;
      }
      if (rsp.data) this.kvmHelp = { ...this.kvmHelp, [key]: rsp.data };
      this.notify(
        raised
          ? "Cancelled the help request."
          : `The KVM has raised its hand. A technician who answers gets ${this.helpWindowLabel(node)} of access.`,
      );
    } finally {
      this.helpBusy = { ...this.helpBusy, [key]: false };
    }
  }

  // ---- Reaching the KVM's own web UI ("Open") ---------------------------
  //
  // A KVM's console is a normal web UI, and a customer sometimes needs it
  // directly — for the things this card doesn't wrap. It can be reached three
  // ways, and which one works depends on where the customer is sitting, so the
  // menu offers each rather than picking for them:
  //
  //   Ethernet / Wi-Fi — the device's own LAN addresses, read from
  //                      `GET /api/vm/info` over the tunnel. Either may be
  //                      absent (no lead plugged in, no Wi-Fi joined).
  //   Mesh            — the site tunnel already mapped for this KVM. Needs no
  //                      LAN at all, so it's the one that always works.

  /** Whether the Open menu is showing for `node` (the card's render guard). */
  linksOpenFor(node: string): boolean {
    return this.sameNode(this.linksFor, node);
  }

  /** Toggle the Open menu for a KVM, loading its addresses on the way open. */
  async toggleKvmLinks(node: string): Promise<void> {
    if (this.sameNode(this.linksFor, node)) {
      this.closeKvmLinks();
      return;
    }
    this.linksFor = node;
    this.kvmLinks = [];
    this.linksError = null;
    await this.loadKvmLinks(node);
  }

  /** Close the Open menu and drop its transient state. */
  closeKvmLinks(): void {
    this.linksFor = null;
    this.kvmLinks = [];
    this.linksError = null;
    this.linksLoading = false;
  }

  /** Build the list of ways to reach `node`'s web UI. The Mesh entry is added
   *  whenever the tunnel maps, so the menu is useful even when the device
   *  can't report its LAN addresses. */
  async loadKvmLinks(node: string): Promise<void> {
    if (this.demo) {
      this.kvmLinks = demoLinks();
      return;
    }
    const peer = (this.snapshot?.peers ?? []).find((p) => this.sameNode(p.node, node));
    const site = peer ? this.kvmWebSite(peer) : undefined;
    if (!peer || !site) {
      this.linksError = "This KVM hasn't published a console yet.";
      return;
    }
    // The site advert's scheme describes the TUNNEL, which is always plaintext:
    // the sites plane serves the device's gin engine in-process and never
    // terminates TLS. It is NOT how to reach the device directly. A Pro defaults
    // to https, so using it for a LAN link built `http://<addr>:443` and the
    // device answered, correctly, "Client sent an HTTP request to an HTTPS
    // server". The device reports its own listener separately, below; this is
    // only the fallback for firmware that predates that split.
    const advertScheme: "http" | "https" = site.scheme === "https" ? "https" : "http";

    this.linksLoading = true;
    try {
      const m = await siteMap(peer.node, site.port);
      if (!m) {
        this.linksError = "Couldn't reach this KVM's console.";
        return;
      }
      const links: KvmLink[] = [];

      const { rsp, reason, localPort } = await this.kvmApi<KvmInfo>(m.localPort, "/api/vm/info");
      if (rsp && rsp.code === 0) {
        // How the device says to reach IT — not how to reach the tunnel.
        const direct = rsp.data?.webScheme === "https" ? "https" : rsp.data?.webScheme === "http" ? "http" : advertScheme;
        const directPort = typeof rsp.data?.webPort === "number" && rsp.data.webPort > 0
          ? rsp.data.webPort
          : site.port;
        for (const ip of rsp.data?.ips ?? []) {
          // The device classifies its own interfaces; anything else it reports
          // is a virtual/tunnel address, not a way in. "USB" is the network
          // gadget on the appliance's own cable — the one address that needs no
          // LAN at all, so it's worth offering wherever the device reports one.
          // Firmware that predates USB reporting simply never sends it.
          const kind = KVM_LINK_KINDS[ip.type];
          if (!kind || !ip.addr) continue;
          links.push({
            kind,
            label: KVM_LINK_LABELS[kind],
            detail: ip.addr,
            host: ip.addr,
            port: directPort,
            scheme: direct,
          });
        }
      } else {
        this.linksError = rsp
          ? this.kvmMsg(rsp, "Couldn't read this KVM's addresses.")
          : (reason ?? "Couldn't read this KVM's addresses.");
      }

      // Always last, always present: the tunnel doesn't need the customer to be
      // on the same network as the KVM.
      //
      // http, always, even for a device whose own listener is https — this end
      // of the tunnel is a plaintext loopback proxy. Same reason the direct
      // links above must NOT use it.
      links.push({
        kind: "mesh",
        label: "Mesh",
        detail: "Through this app",
        host: "127.0.0.1",
        port: localPort,
        scheme: "http",
      });
      this.kvmLinks = links;
    } finally {
      this.linksLoading = false;
    }
  }

  /** Open one of those addresses in the customer's browser. */
  async openKvmLink(link: KvmLink): Promise<void> {
    if (this.demo) {
      this.notify(`Would open ${link.scheme}://${link.host}:${link.port}`);
      this.closeKvmLinks();
      return;
    }
    try {
      await openKvmConsole(link.host, link.port, link.scheme);
      this.closeKvmLinks();
    } catch (e) {
      this.notify(`Couldn't open the KVM's page: ${errMsg(e)}`);
    }
  }

  // ---- KVM Wi-Fi -------------------------------------------------------
  //
  // Read and set a claimed KVM's own Wi-Fi over the SAME mesh "sites" tunnel
  // Update uses (`site_map` → a localhost port → the KVM's web API). The
  // appliance already owns the Wi-Fi system and authenticates the tunnel by
  // mesh roster, so no KVM login/token is needed on this path — exactly like
  // the update POST. One flow covers both models: the connect request
  // (`{ ssid, password }`) is identical, and the status read is normalized
  // across the plain-NanoKVM (`ssid`) and Pro (`wifi { … }`) shapes; the Pro's
  // scan is a pure enhancement that a plain NanoKVM simply 404s.

  /** The label of the KVM whose Wi-Fi panel is open (for the modal title). */
  get wifiKvmLabel(): string {
    const k = this.cecKvms.find((x) => this.sameNode(x.node, this.wifiFor));
    return k?.label ?? "KVM";
  }

  /** Resolve the KVM console tunnel port for `node` — the same map Update
   *  performs. Null when the KVM advertises no web UI or the map fails. */
  private async kvmConsolePort(node: string): Promise<number | null> {
    const peer = (this.snapshot?.peers ?? []).find((p) => this.sameNode(p.node, node));
    const site = peer ? this.kvmWebSite(peer) : undefined;
    if (!peer || !site) return null;
    const m = await siteMap(peer.node, site.port);
    return m ? m.localPort : null;
  }

  /** Call one JSON endpoint on the KVM console over the tunnel.
   *
   *  Goes through Rust (`kvmApiCall`), never the webview's `fetch`. The tunnel
   *  is a different origin from the app, the appliance only sends CORS headers
   *  when its own auth is disabled, and the tunnel adds none — so in the webview
   *  a GET's response was unreadable and every JSON POST (the update, the
   *  Wi-Fi write)
   *  died at a preflight gin has no route for. That surfaced as a bare
   *  "Failed to fetch" with no status, which is why this panel could only ever
   *  report *that* something broke, never *what*.
   *
   *  Returns the device's `{ code, msg, data }` envelope on success. On any
   *  failure it returns null and sets `reason` to something worth showing a
   *  human — the transport error, the HTTP status, or the device's own `msg`. */
  private async kvmApi<T>(
    port: number,
    path: string,
    init?: { method?: string; body?: unknown; timeoutMs?: number },
  ): Promise<KvmApiOutcome<T>> {
    let out: KvmApiCallResult;
    try {
      out = await kvmApiCall(port, path, init);
    } catch (e) {
      return { rsp: null, timedOut: false, reason: errMsg(e), localPort: port };
    }

    // No reply at all. A timeout is flagged separately: a caller writing Wi-Fi
    // credentials treats it as "probably worked, the KVM moved networks".
    if (out.error) {
      return {
        rsp: null,
        timedOut: out.error.kind === "timeout",
        reason: capitalise(out.error.message),
        localPort: out.localPort,
      };
    }

    const body = out.body as { code?: unknown; msg?: unknown } | null;
    const hasEnvelope = !!body && typeof body === "object" && typeof body.code === "number";
    const fail = (reason: string): KvmApiOutcome<T> => ({
      rsp: null,
      timedOut: false,
      reason,
      localPort: out.localPort,
    });

    if (out.status === 401 || out.status === 403) {
      // Shouldn't happen over the mesh — roster membership stands in for the
      // KVM login — so say that plainly rather than asking for a password this
      // panel has no field for.
      return fail("The KVM refused the request (not authorised over the mesh).");
    }
    if (out.status === 404) {
      return fail("This KVM doesn't offer that (HTTP 404).");
    }
    if (out.status < 200 || out.status >= 300) {
      const msg = hasEnvelope && typeof body!.msg === "string" ? ` — ${body!.msg}` : "";
      return fail(`The KVM answered HTTP ${out.status}${msg}.`);
    }
    if (!hasEnvelope) {
      return fail("The KVM sent a reply the app didn't understand.");
    }
    return { rsp: body as KvmApiRsp<T>, timedOut: false, reason: null, localPort: out.localPort };
  }

  /** The device's own message for a non-zero envelope, or a fallback. */
  private kvmMsg(rsp: KvmApiRsp<unknown>, fallback: string): string {
    return typeof rsp.msg === "string" && rsp.msg.trim() ? rsp.msg.trim() : fallback;
  }

  /** Open the Wi-Fi panel for a claimed KVM and read its current state. */
  async openKvmWifi(node: string): Promise<void> {
    this.wifiFor = node;
    this.wifiStatus = null;
    this.wifiScan = null;
    this.wifiScanSource = null;
    this.wifiHostCurrent = null;
    this.wifiHostNote = null;
    this.wifiError = null;
    this.wifiPort = null;
    await this.loadKvmWifi(node);
  }

  /** Close the Wi-Fi panel and drop its transient state. */
  closeKvmWifi(): void {
    this.wifiFor = null;
    this.wifiStatus = null;
    this.wifiScan = null;
    this.wifiScanSource = null;
    this.wifiHostCurrent = null;
    this.wifiHostNote = null;
    this.wifiError = null;
    this.wifiPort = null;
    this.wifiLoading = false;
    this.wifiScanning = false;
    this.wifiBusy = false;
  }

  /** Read the KVM's Wi-Fi status, then (if the model supports it) a scan. */
  async loadKvmWifi(node: string): Promise<void> {
    if (this.demo) {
      this.loadDemoWifi();
      return;
    }
    this.wifiLoading = true;
    this.wifiError = null;
    try {
      const port = this.wifiPort ?? (await this.kvmConsolePort(node));
      if (!port) {
        this.wifiError = "Couldn't reach this KVM's console.";
        return;
      }
      this.wifiPort = port;
      const { rsp, reason, localPort } = await this.kvmApi<KvmWifiStatusRaw>(port, "/api/network/wifi");
      this.wifiPort = localPort;
      if (!rsp || rsp.code !== 0 || !rsp.data) {
        // Drop the cached tunnel so a retry re-maps a fresh one — the mapping
        // may have gone stale (the read is the first call on it each open).
        this.wifiPort = null;
        this.wifiError = rsp
          ? this.kvmMsg(rsp, "The KVM couldn't report its Wi-Fi settings.")
          : (reason ?? "Couldn't read the KVM's Wi-Fi settings.");
        return;
      }
      this.wifiStatus = normalizeWifi(rsp.data);
      // Scan is Pro-only: on a plain NanoKVM the route 404s (kvmApi → null).
      if (this.wifiStatus.supported) await this.scanKvmWifi(node);
      // Whatever the KVM couldn't answer, ask this computer. It's in the same
      // room on the same radio, so its list is a very good proxy for what the
      // KVM can reach — and the network it's joined to is nearly always the
      // one the KVM should be on. Runs either way, because `current` is worth
      // having even when the device produced its own list.
      await this.loadHostWifi();
    } finally {
      this.wifiLoading = false;
    }
  }

  /** Re-scan for nearby networks. A no-op picker on a plain NanoKVM (no scan
   *  route) and in AP mode (the device declines with code -1) — manual SSID
   *  entry always remains. */
  async scanKvmWifi(node: string): Promise<void> {
    if (this.demo) {
      this.wifiScan = demoScanList();
      return;
    }
    const port = this.wifiPort ?? (await this.kvmConsolePort(node));
    if (!port) return;
    this.wifiPort = port;
    this.wifiScanning = true;
    try {
      const { rsp, localPort } = await this.kvmApi<{ wifiList?: KvmWifiNetwork[] }>(
        port,
        "/api/network/wifi/scan",
        { timeoutMs: 20000 },
      );
      this.wifiPort = localPort;
      const list = rsp?.data?.wifiList;
      if (rsp && rsp.code === 0 && Array.isArray(list)) {
        this.wifiScan = sortNetworks(list);
        this.wifiScanSource = "kvm";
      }
      // Any other outcome (null = no scan route; code !== 0 = AP mode / busy)
      // leaves wifiScan as it was — the picker stays hidden and the form carries
      // the flow.
    } finally {
      this.wifiScanning = false;
    }
  }

  /** Look again, from whichever side supplied the list.
   *
   *  The KVM's own scan is asked for first where it exists — it's the better
   *  answer, being what the device can actually reach. Only when that produced
   *  nothing does this clear and re-run the host's, because `loadHostWifi`
   *  deliberately won't overwrite a KVM list and a rescan of a host list would
   *  otherwise be a no-op. */
  async rescanWifi(node: string): Promise<void> {
    if (this.wifiBusy) return;
    if (this.wifiStatus?.supported) await this.scanKvmWifi(node);
    if (this.wifiScanSource === "kvm") return;
    this.wifiScan = null;
    this.wifiScanSource = null;
    this.wifiScanning = true;
    try {
      await this.loadHostWifi();
    } finally {
      this.wifiScanning = false;
    }
  }

  /** Ask THIS computer what Wi-Fi it can see, and which network it's on.
   *
   *  The fallback for the case the panel exists to serve: a KVM with no uplink,
   *  which is exactly the KVM that can't be asked what's nearby — and on a plain
   *  NanoKVM there is no scan route to ask down even when it is online. The host
   *  is in the same room on the same radio, so its list is a very good proxy.
   *  Called a proxy, and labelled as one in the panel, because that is what it
   *  is: signal at the laptop on the desk isn't signal at the appliance behind
   *  the machine.
   *
   *  Never overwrites a list the KVM produced — its own scan is the better
   *  answer where it exists, being what the device can actually reach. What this
   *  always contributes is `current`, which no KVM scan can tell us and which is
   *  the single most useful thing here.
   *
   *  Unprivileged: it reads no password (that needs Administrator on Windows, a
   *  keychain prompt on macOS, root under NetworkManager), so it can never put a
   *  consent dialog in front of a customer who only opened a Wi-Fi panel. */
  private async loadHostWifi(): Promise<void> {
    if (this.demo) {
      this.wifiHostCurrent = "CEC-Guest";
      if (!this.wifiScan) {
        this.wifiScan = demoScanList();
        this.wifiScanSource = "host";
      }
      return;
    }
    let host: HostWifi;
    try {
      host = await hostWifiScan();
    } catch {
      return; // best-effort; the manual field carries the flow
    }
    this.wifiHostCurrent = host.current;
    if (this.wifiScan === null && host.networks.length > 0) {
      this.wifiScan = sortNetworks(host.networks);
      this.wifiScanSource = "host";
    }
    // Only worth showing when nobody produced a list at all — otherwise it's
    // an explanation for a problem the reader doesn't have.
    this.wifiHostNote = this.wifiScan === null ? host.note : null;
  }

  /** Point the KVM at a Wi-Fi network. `password` may be blank for an open
   *  network. The device write is slow (it waits to confirm association), and
   *  if Wi-Fi is the KVM's only uplink the tunnel can drop mid-write — so a
   *  missing reply is reported as "sent, may take a moment", not a hard error. */
  async connectKvmWifi(node: string, ssid: string, password: string): Promise<void> {
    const name = ssid.trim();
    if (!name || this.wifiBusy) return;
    this.wifiError = null;
    if (this.demo) {
      this.wifiBusy = true;
      setTimeout(() => {
        this.wifiStatus = { supported: true, apMode: false, connected: true, ssid: name };
        this.wifiBusy = false;
        this.notify(`Connected the KVM to ${name}.`);
      }, 900);
      return;
    }
    const port = this.wifiPort ?? (await this.kvmConsolePort(node));
    if (!port) {
      this.wifiError = "Couldn't reach this KVM's console.";
      return;
    }
    this.wifiPort = port;
    this.wifiBusy = true;
    try {
      const { rsp, timedOut, reason, localPort } = await this.kvmApi(port, "/api/network/wifi/connect", {
        method: "POST",
        body: { ssid: name, password },
        timeoutMs: 40000,
      });
      this.wifiPort = localPort;
      if (rsp && rsp.code === 0) {
        this.notify(`Connected the KVM to ${name}.`);
        await this.loadKvmWifi(node);
        void this.refreshKvms();
      } else if (rsp) {
        // The appliance validates `password` as required on both models, so a
        // blank one comes back as "invalid parameters" — a message that reads
        // like a bug rather than the limitation it is. Name it.
        this.wifiError = password
          ? this.kvmMsg(
              rsp,
              "Couldn't connect. Check the network name and password, then try again.",
            )
          : "This KVM won't accept an empty password, so it can't join an open network.";
      } else if (timedOut) {
        // The write went out but nothing came back. Most often the KVM has
        // moved onto the new network and its mesh link is re-establishing —
        // not a failure we can be sure of, so don't report one.
        this.wifiError =
          "Sent the Wi-Fi details. If the KVM moves onto this network it may drop off for a minute. Reopen Wi-Fi to check.";
        void this.refreshKvms();
      } else {
        this.wifiError = reason ?? "Couldn't send the Wi-Fi details.";
      }
    } finally {
      this.wifiBusy = false;
    }
  }

  /** Confirm before disconnecting — it can strand a Wi-Fi-only KVM. */
  promptDisconnectKvmWifi(node: string, ssid: string | null): void {
    this.askConfirm({
      title: "Disconnect this KVM's Wi-Fi?",
      body: `${ssid ? `"${ssid}"` : "The current network"} will be forgotten. If Wi-Fi is the KVM's only connection, it may go offline until it's reconnected.`,
      confirmLabel: "Disconnect",
      danger: true,
      onConfirm: () => this.disconnectKvmWifi(node),
    });
  }

  /** Drop the KVM's current Wi-Fi network. */
  async disconnectKvmWifi(node: string): Promise<void> {
    if (this.wifiBusy) return;
    this.wifiError = null;
    if (this.demo) {
      this.wifiStatus = { supported: true, apMode: false, connected: false, ssid: null };
      this.notify("Disconnected the KVM's Wi-Fi.");
      return;
    }
    const port = this.wifiPort ?? (await this.kvmConsolePort(node));
    if (!port) {
      this.wifiError = "Couldn't reach this KVM's console.";
      return;
    }
    this.wifiPort = port;
    this.wifiBusy = true;
    try {
      const { rsp, timedOut, reason, localPort } = await this.kvmApi(port, "/api/network/wifi/disconnect", {
        method: "POST",
        timeoutMs: 20000,
      });
      this.wifiPort = localPort;
      if (rsp && rsp.code === 0) {
        this.notify("Disconnected the KVM's Wi-Fi.");
        await this.loadKvmWifi(node);
      } else if (rsp) {
        this.wifiError = this.kvmMsg(rsp, "Couldn't disconnect the Wi-Fi. Try again.");
      } else if (timedOut) {
        this.wifiError =
          "Sent the request. The KVM may drop off for a moment if Wi-Fi was its only connection.";
        void this.refreshKvms();
      } else {
        this.wifiError = reason ?? "Couldn't disconnect the Wi-Fi.";
      }
    } finally {
      this.wifiBusy = false;
    }
  }

  /** Web-preview only: a canned Wi-Fi state so the panel is explorable in the
   *  browser without a device. */
  private loadDemoWifi(): void {
    this.wifiStatus = { supported: true, apMode: false, connected: false, ssid: null };
    this.wifiScan = demoScanList();
    this.wifiLoading = false;
  }

  /** Open the in-app confirmation popup with a caller-supplied action. */
  askConfirm(req: {
    title: string;
    body: string;
    confirmLabel: string;
    danger?: boolean;
    onConfirm: () => void | Promise<void>;
  }): void {
    this.confirmDialog = req;
  }

  /** Run the pending confirm's action, then close the popup. */
  async runConfirm(): Promise<void> {
    const d = this.confirmDialog;
    this.confirmDialog = null;
    if (d) await d.onConfirm();
  }

  /** Dismiss the confirm popup without acting. */
  cancelConfirm(): void {
    this.confirmDialog = null;
  }

  /** Ask to unclaim a KVM behind the confirm popup — unclaiming resets the
   *  appliance, so it's gated. */
  promptUnclaim(node: string, label: string): void {
    this.askConfirm({
      title: "Unclaim this KVM?",
      body: `${label} will reset. It forgets this computer and offers itself for setup again.`,
      confirmLabel: "Unclaim",
      danger: true,
      onConfirm: () => this.unclaimKvm(node),
    });
  }

  /** Unclaim a KVM we own — releases our ownership (fleet_kick; claiming made
   *  us its fleet owner, and a customer's fleet has no MFA, so no code). The
   *  KVM resets to its joining mesh and offers itself for adoption again. */
  async unclaimKvm(node: string): Promise<void> {
    if (this.busy) return;
    // Forget any "attached here?" answer so a later re-claim asks again.
    const next = { ...this.attachAsked };
    delete next[canonicalTech(node)];
    this.attachAsked = next;
    if (this.demo) {
      this.demoPatchKvm(node, (p) => {
        p.owner = null;
        p.claimable = true;
        p.kvm = { ...(p.kvm ?? {}), attached_to: undefined };
      });
      this.notify("Unclaimed. It's offering itself for setup again.");
      return;
    }
    this.busy = true;
    try {
      await fleetKick(node);
      this.notify("Unclaiming the KVM…");
      void this.refreshKvmsSoon();
    } catch (e) {
      this.notify(`Couldn't unclaim the KVM: ${errMsg(e)}`);
    } finally {
      this.busy = false;
    }
  }

  /** Web-preview only: mutate the seeded demo KVM peer so the claim → attach
   *  flow is clickable in the browser. */
  private demoPatchKvm(node: string, patch: (p: MeshPeer) => void): void {
    const peers = (this.snapshot?.peers ?? []).map((p) => {
      if (!this.sameNode(p.node, node)) return p;
      const copy: MeshPeer = { ...p, kvm: p.kvm ? { ...p.kvm } : undefined };
      patch(copy);
      return copy;
    });
    this.snapshot = { ...(this.snapshot ?? { ready: true }), peers };
  }

  // ---- self-update -------------------------------------------------------

  /** Read the updater's current state (version, install kind, prefs, staged). */
  async loadUpdateStatus(): Promise<void> {
    if (this.demo) return;
    this.updateInfo = await updateStatus();
  }

  /** A background check reported in. Only outcomes that mean "something newer
   *  than what you're running exists" are worth a toast — the routine
   *  up-to-date / not-due / disabled ticks refresh state quietly. */
  private applyUpdateChecked(o: CheckOutcome): void {
    this.updateOutcome = o;
    void this.loadUpdateStatus();
    switch (o.outcome) {
      case "staged":
        this.notify(`Update ${o.version} is ready — restart CEC Support to use it`);
        break;
      case "manual_update_available":
        this.notify(`Version ${o.latest} is available — reinstall to update`);
        break;
      case "policy_blocked":
        this.notify(`Version ${o.latest} is available — see Settings to install it`);
        break;
      default:
        break;
    }
  }

  /** Check the release feed now and stage anything permitted. */
  async checkUpdates(): Promise<void> {
    if (this.demo) return;
    this.updateBusy = true;
    this.updateOutcome = null;
    try {
      this.updateOutcome = await updateCheck();
      this.updateInfo = (await updateStatus()) ?? this.updateInfo;
    } finally {
      this.updateBusy = false;
    }
  }

  /** Apply a staged update and restart into it. On success the process
   *  restarts, so nothing after this runs. */
  async applyUpdateAndRestart(): Promise<void> {
    if (this.demo) return;
    this.updateBusy = true;
    try {
      await updateRelaunch();
    } catch (e) {
      this.notify(`Couldn't install the update: ${String(e)}`);
      // Best-effort: leave the staged marker in place for the next launch.
      await updateApply();
      this.updateInfo = (await updateStatus()) ?? this.updateInfo;
    } finally {
      this.updateBusy = false;
    }
  }

  /** Flip an updater preference (e.g. automatic updates on/off). */
  async setUpdatePrefs(prefs: UpdatePrefs): Promise<void> {
    if (this.demo) return;
    const next = await updateSetPrefs(prefs);
    if (next) this.updateInfo = next;
  }

  /** Plain-language summary of a check result, for the settings panel. */
  checkOutcomeText(o: CheckOutcome | null): string | null {
    if (!o) return null;
    switch (o.outcome) {
      case "staged":
        return `Version ${o.version} is ready — restart CEC Support to use it`;
      case "manual_update_available":
        return `Version ${o.latest} is available, but this copy can't update itself — reinstall to update`;
      case "up_to_date":
        return "You're on the latest version";
      case "policy_blocked":
        return `Version ${o.latest} is available and waiting for you`;
      case "disabled":
        return "Automatic updates are off";
      case "not_due":
        return "Checked recently — try again shortly";
      default:
        return null;
    }
  }

  // ---- toasts ----------------------------------------------------------

  notify(message: string): void {
    this.toast = message;
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => (this.toast = null), 4500);
  }

  dismissToast(): void {
    this.toast = null;
  }

  // ---- demo (web preview only) -----------------------------------------

  private loadDemo(): void {
    this.status = {
      number: "123456789",
      network_id: "cecsupport-clients",
      role: "client",
      label: "Reception PC",
    };
    this.grants = [
      {
        technician: "techpubkey-demo-forever",
        agent_name: "Alex at CEC",
        control: true,
        scope: "forever",
        granted_at: this.now - 3600,
        expires_at: null,
      },
      {
        technician: "techpubkey-demo-3h",
        agent_name: "Sam at CEC",
        control: false,
        scope: "three_hours",
        granted_at: this.now - 600,
        expires_at: this.now + 3 * 3600 - 600,
      },
    ];
    // A short transcript with one demo technician, so tapping their name in the
    // access list opens a real-looking conversation in the preview.
    this.chatThreads = {
      "techpubkey-demo-forever": [
        {
          id: "demo-1",
          from: "technician",
          text: "Hi! I'm connected now. I'll take a look at that printer for you.",
          ts: this.now - 300,
        },
        {
          id: "demo-2",
          from: "client",
          text: "Thank you! It just stopped printing this morning.",
          ts: this.now - 250,
        },
        {
          id: "demo-3",
          from: "technician",
          text: "No problem. Give me a couple of minutes and I'll have it sorted.",
          ts: this.now - 240,
        },
      ],
    };
    this.service = {
      platform: "windows",
      supported: true,
      manager: "windows",
      installed: false,
      service_name: "CECSupport",
    };
    // The spec card is a headline feature — the demo shows it fully dressed:
    // usage, disks, temps, and the identity pair in the title.
    this.specsPending = false;
    this.specs = {
      hostname: "RECEPTION-01",
      os: "Windows 11 Pro 24H2",
      // Raw board_name, exactly as firmware reports it (no vendor prefix).
      board: "PRIME B550-PLUS",
      cpu: { brand: "AMD Ryzen 5 5600G", cores: 6, threads: 12, max_mhz: 4464 },
      memory: {
        total_bytes: 16 * 1024 ** 3,
        available_bytes: Math.round(9.3 * 1024 ** 3),
      },
      gpus: [{ name: "AMD Radeon Graphics", vram_bytes: null }],
      disks: [
        {
          name: "Samsung SSD 970 EVO",
          mount: "C:",
          total_bytes: 500 * 1000 ** 3,
          available_bytes: 212 * 1000 ** 3,
          removable: false,
        },
      ],
      temps: [
        { label: "ACPI\\ThermalZone\\TZ00_0", celsius: 47.5 },
        { label: "coretemp Package id 0", celsius: 52.1 },
      ],
    };
    // A claimable KVM (an ordinary NanoKVM — no `cec-kvm-` mesh) so the KVM &
    // Claiming card shows in the preview and proves normal KVMs surface. Claim
    // → "attached to this computer?" → Update / Unclaim is fully clickable via
    // the demo mutations in claimKvm / attachKvmHere / unclaimKvm.
    this.snapshot = {
      ready: true,
      me: "clientpubkey-demo1",
      peers: [
        {
          node: "kvmpubkey-demo1",
          label: "NanoKVM",
          owner: null,
          claimable: true,
          features: ["kvm", "sites"],
          sites: [{ id: "tcp:80", label: "HTTP", port: 80, scheme: "http" }],
          kvm: {
            joining_mesh: "kvm-9f2c1-a7b3",
            web: "tcp:80",
            meshes: ["kvm-9f2c1-a7b3"],
          },
        },
      ],
    };
  }

  /** Simulate a technician dialing in — a dev affordance so the approval modal
   *  can be seen in the browser preview. Only wired up in demo mode. */
  simulateDial(): void {
    if (!this.demo) return;
    this.onRequest({
      tech: "techpubkey-demo-incoming",
      agent_name: "Jordan at CEC",
      want_control: true,
      session_id: `demo-${Date.now()}`,
      verification_code: "7K2Q9M",
    });
  }
}

export const store = new CecStore();
