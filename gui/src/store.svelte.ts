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
  machineSpecs,
  cecOnline,
  cecStatus,
  isTauri,
  onCecChat,
  onCecGrants,
  onCecHelp,
  onCecRequest,
  onCecSession,
  serviceInstall,
  serviceStatus,
  serviceStop,
  serviceUninstall,
} from "./tauri";
import type {
  ApprovalScope,
  CecChatMsg,
  CecStatus,
  ConnectRequest,
  Grant,
  LiveSession,
  MachineSpecs,
  ServiceStatus,
  SessionEvent,
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
