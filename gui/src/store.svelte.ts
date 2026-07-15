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
  cecPending,
  cecRevoke,
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
  serviceInstall,
  serviceStatus,
  serviceStop,
  serviceUninstall,
  sessionSnapshot,
  siteMap,
} from "./tauri";
import { FEATURE_KVM } from "./types";
import type {
  ApprovalScope,
  CecChatMsg,
  CecKvm,
  CecStatus,
  ConnectRequest,
  Grant,
  KvmApiRsp,
  KvmWifiNetwork,
  KvmWifiStatus,
  KvmWifiStatusRaw,
  LiveSession,
  MachineSpecs,
  MeshPeer,
  ServiceStatus,
  SessionEvent,
  SessionSnapshot,
  SiteAdvert,
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

/** A canned scan for the browser preview so the network picker is explorable
 *  without a device (demo mode only). */
function demoScanList(): KvmWifiNetwork[] {
  return [
    { ssid: "CEC-Guest", bssid: "aa:bb:cc:00:11:22", signal: -47, security: "wpa2" },
    { ssid: "Reception 5G", bssid: "aa:bb:cc:00:11:23", signal: -64, security: "wpa2" },
    { ssid: "Lobby-Open", bssid: "aa:bb:cc:00:11:24", signal: -73, security: "open" },
  ];
}

class CecStore {
  /** Whether we're running in the browser preview (no backend). */
  readonly demo = !isTauri();

  version = $state<string | null>(null);
  status = $state<CecStatus | null>(null);
  /** Technician requests awaiting a decision; `request` shows the first. */
  pending = $state<ConnectRequest[]>([]);
  /** Live sessions keyed by session id. */
  sessions = $state<Record<string, LiveSession>>({});
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
  /** How many watchers the latest help beacon actually reached (from the
   *  node's dispatched-to count). null = no beacon reported yet this ask.
   *  0 = hand going up but nobody wired yet; 1+ = CEC can see the hand. */
  helpWatchers = $state<number | null>(null);
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
  /** The set of peers the node can currently *reach* (canonical ids whose live
   *  status is active/shelved), across every network it's on. The presence
   *  snapshot remembers a KVM's last advert even after it powers off, so this
   *  is the liveness cross-check that lets the card drop an offline KVM. `null`
   *  = reachability unknown (web mode, or the node couldn't be asked) → the
   *  card fails open and doesn't filter on it. */
  private reachable = $state<Set<string> | null>(null);
  /** Claimed-but-unattached KVMs the customer has answered "not this computer"
   *  for, keyed by canonical node id — so the "is it attached here?" prompt
   *  doesn't keep nagging. Session-local (clears on restart). */
  private attachAsked = $state<Record<string, boolean>>({});

  // ---- KVM Wi-Fi panel -------------------------------------------------
  /** The KVM whose Wi-Fi panel is open (its node id), or null when closed. The
   *  KvmClaimCard renders the modal only while this is set. */
  wifiFor = $state<string | null>(null);
  /** The open KVM's current Wi-Fi state, or null until the first read lands. */
  wifiStatus = $state<KvmWifiStatus | null>(null);
  /** Nearby networks from the KVM's scan, or null when the device can't scan
   *  (a plain NanoKVM 404s the scan route) — the picker then hides and it's
   *  manual SSID entry only. An empty array means "can scan, found nothing". */
  wifiScan = $state<KvmWifiNetwork[] | null>(null);
  /** Reading the initial status when the panel opens. */
  wifiLoading = $state(false);
  /** A re-scan is in flight. */
  wifiScanning = $state(false);
  /** A connect / disconnect is in flight. */
  wifiBusy = $state(false);
  /** An inline message for the Wi-Fi panel (a failed or ambiguous connect),
   *  or null. Separate from the global toast so it sits next to the form. */
  wifiError = $state<string | null>(null);
  /** The resolved console tunnel base (`http://localhost:<port>`) for the open
   *  panel — mapped once when it opens and reused for every call, so status /
   *  scan / connect don't each re-tunnel. Cleared on close. */
  private wifiBase: string | null = null;

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
  private toastTimer: ReturnType<typeof setTimeout> | undefined;

  /** The connect request to prompt about (first pending), or null. */
  get request(): ConnectRequest | null {
    return this.pending[0] ?? null;
  }

  /** Sessions that haven't ended — the Connected banner list. */
  get liveSessions(): LiveSession[] {
    return Object.values(this.sessions).filter((s) => s.state !== "ended");
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
        // Every beacon reports how many watchers it reached — the waiting
        // card's "raising your hand…" vs "CEC can see you" signal.
        if (typeof e.watchers === "number") this.helpWatchers = e.watchers;
      }),
    );

    // The node comes up in parallel with this webview — on a fresh machine its
    // first start (identity generation, first-run AV scans of the sidecars)
    // takes many seconds. A single early fetch returns null and the UI would
    // sit at "Starting up…" forever over a perfectly healthy node. Keep asking
    // until the node answers with our number, then settle into event-driven
    // updates. Runs in the background so the rest of init never blocks on it.
    void this.bringUp();

    this.service = await serviceStatus();
    this.autostart = await autostartGet();
    this.autostartMode = await autostartModeGet();
    this.keepBackground = await backgroundGet();

    this.timer = setInterval(() => {
      this.now = Math.floor(Date.now() / 1000);
    }, 1000);

    // Refresh KVM & Claiming discovery when the app returns to the foreground
    // — a customer who plugged in a KVM while the window was hidden sees it on
    // return, without a steady background poll (refreshKvms no-ops in demo).
    if (typeof document !== "undefined") {
      const onVisible = () => {
        if (document.visibilityState === "visible") void this.refreshKvms();
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
        // refocus or the card's Refresh.
        void this.refreshKvms();
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
    if (this.toastTimer) clearTimeout(this.toastTimer);
  }

  async refresh(): Promise<void> {
    this.status = await cecStatus();
    this.pending = await cecPending();
    this.grants = await cecGrants();
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
            ? "Approved — they can reconnect for the next 3 hours."
            : "Approved — they can reconnect until you remove them.",
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

  /** End a live session, leaving any standing grant intact (that's what Forget
   *  is for). Sends the same End the wire uses for a decline. */
  async disconnect(s: LiveSession): Promise<void> {
    try {
      await cecDeny(s.tech, s.session_id);
    } catch (e) {
      this.notify(`Couldn't disconnect: ${errMsg(e)}`);
    }
    const next = { ...this.sessions };
    delete next[s.session_id];
    this.sessions = next;
    this.notify("Disconnected.");
  }

  /** Forget a technician entirely — revoke their standing approval and drop
   *  their node from the mesh. Bites immediately. */
  async forget(tech: string): Promise<void> {
    try {
      await cecRevoke(tech);
      await cecForgetNode(tech);
    } catch (e) {
      this.notify(`Couldn't remove: ${errMsg(e)}`);
    }
    await this.loadGrants();
    this.notify("Removed. They can't reconnect without asking you again.");
  }

  /** "Ask for help": raise this machine's hand on the support area until a
   *  technician connects or the customer cancels. The node ensures area
   *  residence as part of the ask, so a tap on a fresh launch still just works. */
  async askHelp(): Promise<void> {
    // A fresh ask starts with an unknown reach — the card shows "raising
    // your hand…" until the first beacon reports who it reached.
    this.helpWatchers = null;
    if (this.demo) {
      this.askingHelp = true;
      // Act out the real sequence: a couple of seconds of hand-raising,
      // then a watcher hears it.
      setTimeout(() => {
        if (this.askingHelp) this.helpWatchers = 1;
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
    const extra = (this.chatThreads[key] ?? []).filter((m) => !seen.has(m.id));
    this.chatThreads = {
      ...this.chatThreads,
      [key]: [...msgs, ...extra].sort((a, b) => a.ts - b.ts),
    };
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
          text: "Thanks — I can see that. Give me one moment.",
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
  // offers Reboot (the NanoKVM GPIO reset, tunnelled through the KVM's own web
  // UI — exactly how the AllMyStuff app's `kvmFeature` does it) and Wi-Fi setup
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
   *  (Reboot then has nowhere to POST). */
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
  async refreshKvms(): Promise<void> {
    if (this.demo) return;
    const snap = await sessionSnapshot();
    if (snap) this.snapshot = snap;
    await this.refreshReachable();
  }

  /** Recompute which peers the node can currently reach (live status
   *  active/shelved) across every network it's on — the liveness cross-check
   *  that lets the card drop an offline KVM (see `reachable`). Fail-open: if the
   *  node can't be asked for its networks, reachability is left unknown (null)
   *  and nothing is filtered on it. */
  private async refreshReachable(): Promise<void> {
    const nets = await meshNetworks();
    if (!nets) {
      this.reachable = null;
      return;
    }
    const live = new Set<string>();
    for (const net of nets) {
      const peers = await meshPeers(net.network_id);
      if (!peers) continue;
      for (const pr of peers) {
        if (pr.status === "active" || pr.status === "shelved") {
          live.add(canonicalTech(pr.device_id));
        }
      }
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
      this.notify("Still finding this computer on the mesh — try again in a moment.");
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

  /** Reboot the machine the KVM controls — map the KVM's web UI to a local
   *  port, then POST the NanoKVM GPIO reset over the tunnel (auth is bypassed
   *  on the mesh path, so no token). Mirrors the AllMyStuff app's `kvmFeature`. */
  async rebootKvm(node: string): Promise<void> {
    if (this.busy) return;
    if (this.demo) {
      this.notify("Reboot sent to the KVM.");
      return;
    }
    const peer = (this.snapshot?.peers ?? []).find((p) => this.sameNode(p.node, node));
    const site = peer ? this.kvmWebSite(peer) : undefined;
    if (!peer || !site) {
      this.notify("This KVM hasn't published a console yet — can't reboot it.");
      return;
    }
    this.busy = true;
    try {
      const m = await siteMap(peer.node, site.port);
      if (!m) {
        this.notify("Couldn't reach the KVM's console.");
        return;
      }
      const res = await fetch(`http://localhost:${m.localPort}/api/vm/gpio`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ type: "reset" }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      this.notify("Reboot sent to the KVM.");
    } catch (e) {
      this.notify(`Couldn't reboot the KVM: ${errMsg(e)}`);
    } finally {
      this.busy = false;
    }
  }

  // ---- KVM Wi-Fi -------------------------------------------------------
  //
  // Read and set a claimed KVM's own Wi-Fi over the SAME mesh "sites" tunnel
  // the Reboot uses (`site_map` → a localhost port → the KVM's web API). The
  // appliance already owns the Wi-Fi system and authenticates the tunnel by
  // mesh roster, so no KVM login/token is needed on this path — exactly like
  // the reboot POST. One flow covers both models: the connect request
  // (`{ ssid, password }`) is identical, and the status read is normalized
  // across the plain-NanoKVM (`ssid`) and Pro (`wifi { … }`) shapes; the Pro's
  // scan is a pure enhancement that a plain NanoKVM simply 404s.

  /** The label of the KVM whose Wi-Fi panel is open (for the modal title). */
  get wifiKvmLabel(): string {
    const k = this.cecKvms.find((x) => this.sameNode(x.node, this.wifiFor));
    return k?.label ?? "KVM";
  }

  /** Resolve the KVM console tunnel base (`http://localhost:<port>`) for `node`
   *  — the same map the Reboot performs. Null when the KVM advertises no web UI
   *  or the map fails. */
  private async kvmConsoleBase(node: string): Promise<string | null> {
    const peer = (this.snapshot?.peers ?? []).find((p) => this.sameNode(p.node, node));
    const site = peer ? this.kvmWebSite(peer) : undefined;
    if (!peer || !site) return null;
    const m = await siteMap(peer.node, site.port);
    return m ? `http://localhost:${m.localPort}` : null;
  }

  /** Call one JSON endpoint on the KVM console over the tunnel and return its
   *  `{ code, msg, data }` envelope. Returns null on a network error, a
   *  timeout, a non-200, or a body that isn't that envelope (a plain NanoKVM
   *  answering an unknown route with a 404 / HTML) — callers read null as
   *  "unavailable" or, for a write, "sent but unconfirmed". */
  private async kvmApi<T>(
    base: string,
    path: string,
    init?: { method?: string; body?: unknown; timeoutMs?: number },
  ): Promise<KvmApiRsp<T> | null> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), init?.timeoutMs ?? 12000);
    try {
      const hasBody = init?.body !== undefined;
      const res = await fetch(`${base}${path}`, {
        method: init?.method ?? "GET",
        headers: hasBody ? { "content-type": "application/json" } : undefined,
        body: hasBody ? JSON.stringify(init?.body) : undefined,
        signal: controller.signal,
      });
      if (!res.ok) return null;
      const json: unknown = await res.json();
      if (
        !json ||
        typeof json !== "object" ||
        typeof (json as { code?: unknown }).code !== "number"
      ) {
        return null;
      }
      return json as KvmApiRsp<T>;
    } catch {
      return null;
    } finally {
      clearTimeout(timer);
    }
  }

  /** Open the Wi-Fi panel for a claimed KVM and read its current state. */
  async openKvmWifi(node: string): Promise<void> {
    this.wifiFor = node;
    this.wifiStatus = null;
    this.wifiScan = null;
    this.wifiError = null;
    this.wifiBase = null;
    await this.loadKvmWifi(node);
  }

  /** Close the Wi-Fi panel and drop its transient state. */
  closeKvmWifi(): void {
    this.wifiFor = null;
    this.wifiStatus = null;
    this.wifiScan = null;
    this.wifiError = null;
    this.wifiBase = null;
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
      const base = this.wifiBase ?? (await this.kvmConsoleBase(node));
      if (!base) {
        this.wifiError = "Couldn't reach this KVM's console.";
        return;
      }
      this.wifiBase = base;
      const rsp = await this.kvmApi<KvmWifiStatusRaw>(base, "/api/network/wifi");
      if (!rsp || rsp.code !== 0 || !rsp.data) {
        // Drop the cached tunnel so a retry re-maps a fresh one — the mapping
        // may have gone stale (the read is the first call on it each open).
        this.wifiBase = null;
        this.wifiError = "Couldn't read the KVM's Wi-Fi settings.";
        return;
      }
      this.wifiStatus = normalizeWifi(rsp.data);
      // Scan is Pro-only and purely additive: on a plain NanoKVM the route
      // 404s (kvmApi → null) and the picker just never shows.
      if (this.wifiStatus.supported) await this.scanKvmWifi(node);
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
    const base = this.wifiBase ?? (await this.kvmConsoleBase(node));
    if (!base) return;
    this.wifiBase = base;
    this.wifiScanning = true;
    try {
      const rsp = await this.kvmApi<{ wifiList?: KvmWifiNetwork[] }>(
        base,
        "/api/network/wifi/scan",
        { timeoutMs: 20000 },
      );
      const list = rsp?.data?.wifiList;
      if (rsp && rsp.code === 0 && Array.isArray(list)) {
        this.wifiScan = sortNetworks(list);
      }
      // Any other outcome (null = no scan route; code !== 0 = AP mode / busy)
      // leaves wifiScan as it was — the picker stays hidden and the form carries
      // the flow.
    } finally {
      this.wifiScanning = false;
    }
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
    const base = this.wifiBase ?? (await this.kvmConsoleBase(node));
    if (!base) {
      this.wifiError = "Couldn't reach this KVM's console.";
      return;
    }
    this.wifiBase = base;
    this.wifiBusy = true;
    try {
      const rsp = await this.kvmApi(base, "/api/network/wifi/connect", {
        method: "POST",
        body: { ssid: name, password },
        timeoutMs: 40000,
      });
      if (rsp && rsp.code === 0) {
        this.notify(`Connected the KVM to ${name}.`);
        await this.loadKvmWifi(node);
        void this.refreshKvms();
      } else if (rsp) {
        this.wifiError =
          "Couldn't connect — check the network name and password, then try again.";
      } else {
        // No reply: the request went out but nothing came back. Most often the
        // KVM has moved onto the new network and its mesh link is
        // re-establishing — not a failure we can be sure of.
        this.wifiError =
          "Sent the Wi-Fi details. If the KVM moves onto this network it may drop off for a minute — then reopen Wi-Fi to check.";
        void this.refreshKvms();
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
    const base = this.wifiBase ?? (await this.kvmConsoleBase(node));
    if (!base) {
      this.wifiError = "Couldn't reach this KVM's console.";
      return;
    }
    this.wifiBase = base;
    this.wifiBusy = true;
    try {
      const rsp = await this.kvmApi(base, "/api/network/wifi/disconnect", {
        method: "POST",
        timeoutMs: 20000,
      });
      if (rsp && rsp.code === 0) {
        this.notify("Disconnected the KVM's Wi-Fi.");
        await this.loadKvmWifi(node);
      } else if (rsp) {
        this.wifiError = "Couldn't disconnect the Wi-Fi. Try again.";
      } else {
        this.wifiError =
          "Sent the request. The KVM may drop off for a moment if Wi-Fi was its only connection.";
        void this.refreshKvms();
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
      body: `${label} will reset — it forgets this computer and offers itself for setup again.`,
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
      this.notify("Unclaimed — it's offering itself for setup again.");
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
          text: "Hi! I'm connected now — I'll take a look at that printer for you.",
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
      board: "ASUS PRIME B550-PLUS",
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
    // → "attached to this computer?" → Reboot / Unclaim is fully clickable via
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
