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
  backgroundGet,
  backgroundSet,
  cecApprove,
  cecAskHelp,
  cecDeny,
  cecForgetNode,
  cecGrants,
  cecPending,
  cecRevoke,
  cecSetLabel,
  machineSpecs,
  machineTemps,
  cecStartHosting,
  cecStatus,
  cecStopHosting,
  isTauri,
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
  CecStatus,
  ConnectRequest,
  Grant,
  LiveSession,
  MachineSpecs,
  ServiceStatus,
  SessionEvent,
} from "./types";

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
  /** Opt-in: closing the window keeps the app in the tray. Off by default —
   *  the close button really quits. */
  keepBackground = $state(false);
  /** Unix seconds, re-read each second so expiry countdowns tick. */
  now = $state(Math.floor(Date.now() / 1000));
  /** Which screen is showing. `start` is the front door (Ask for help / Show
   *  Support Number); `number` is the classic number screen behind it. */
  view = $state<"start" | "number" | "settings">("start");
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

  private unlisteners: Array<() => void> = [];
  private timer: ReturnType<typeof setInterval> | undefined;
  private toastTimer: ReturnType<typeof setTimeout> | undefined;
  private tempsTimer: ReturnType<typeof setInterval> | undefined;

  /** The connect request to prompt about (first pending), or null. */
  get request(): ConnectRequest | null {
    return this.pending[0] ?? null;
  }

  /** Sessions that haven't ended — the Connected banner list. */
  get liveSessions(): LiveSession[] {
    return Object.values(this.sessions).filter((s) => s.state !== "ended");
  }

  get hosting(): boolean {
    return this.status?.hosting ?? false;
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

  /** Set on destroy so the bring-up retry loop ends with the store. */
  private stopped = false;

  async init(): Promise<void> {
    this.version = await appVersion();

    // Listeners first, so nothing the node emits during bring-up is missed.
    this.unlisteners.push(await onCecRequest((r) => this.onRequest(r)));
    this.unlisteners.push(await onCecSession((s) => this.onSession(s)));
    this.unlisteners.push(await onCecGrants((g) => (this.grants = g)));
    this.unlisteners.push(
      await onCecHelp((e) => {
        // The node withdraws the ask itself when a session is approved (help
        // arrived) — the waiting card must follow without a manual refresh.
        if (e.asking === false) this.askingHelp = false;
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
      // Idempotent join of our own Silent mesh; a null (node still starting)
      // is retried on the next round.
      await cecStartHosting();
      await this.refresh();
      if (this.status?.number) {
        // Node's up — pull the spec card's scan once. Not part of refresh():
        // a full hardware scan per event would be waste for numbers that
        // barely move. Temps are the exception — the card's one moving
        // number — so they get their own lazy poll against the node's
        // scan-free machine_temps.
        this.specs = await machineSpecs();
        this.specsPending = false;
        if (this.specs) {
          this.tempsTimer = setInterval(() => void this.pollTemps(), 30_000);
        }
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
    if (this.tempsTimer) clearInterval(this.tempsTimer);
  }

  /** Refresh just the spec card's temps. A null (older node, node briefly
   *  down) keeps the last reading rather than blanking the row. */
  private async pollTemps(): Promise<void> {
    if (!this.specs) return;
    const t = await machineTemps();
    if (t?.temps) this.specs.temps = t.temps;
  }

  async refresh(): Promise<void> {
    this.status = await cecStatus();
    this.pending = await cecPending();
    this.grants = await cecGrants();
    // The node is the truth for the ask (it withdraws it itself on approval,
    // and a restart drops it) — mirror it whenever the status lands.
    if (this.status) this.askingHelp = this.status.asking_help === true;
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
      return;
    }
    const prev = this.sessions[s.session_id];
    this.sessions = {
      ...this.sessions,
      [s.session_id]: {
        session_id: s.session_id,
        state: s.state,
        tech: prev?.tech ?? "",
        agent_name: prev?.agent_name ?? "Your technician",
        want_control: prev?.want_control ?? false,
      },
    };
    // Once a session is connecting/active the decision has been made, so drop
    // any lingering prompt for it.
    if (s.state === "active" || s.state === "connecting") {
      this.pending = this.pending.filter((p) => p.session_id !== s.session_id);
    }
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

  async setHosting(on: boolean): Promise<void> {
    if (on) await cecStartHosting();
    else await cecStopHosting();
    this.status = await cecStatus();
  }

  /** "Ask for help": beacon this machine onto the global help room until a
   *  technician connects or the customer cancels. The node brings hosting up
   *  as part of the ask, so a tap on a fresh launch still just works. */
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
    this.busy = true;
    try {
      await cecAskHelp(true);
      this.askingHelp = true;
    } catch (e) {
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
      network_id: "cec-123456789",
      role: "client",
      hosting: true,
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
