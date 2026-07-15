<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { store } from "../store.svelte";
  import { isTauri } from "../tauri";
  import SpecSheet from "./SpecSheet.svelte";
  import StartScreen from "./StartScreen.svelte";
  import ApproveModal from "./ApproveModal.svelte";
  import ContactCard from "./ContactCard.svelte";
  import ConnectedBanner from "./ConnectedBanner.svelte";
  import AccessList from "./AccessList.svelte";
  import KvmClaimCard from "./KvmClaimCard.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
  import cecLogo from "../assets/cec-logo.png";

  let contentEl = $state<HTMLElement | null>(null);
  let flowEl = $state<HTMLElement | null>(null);

  // Grow the window (never shrink it) when the vertical flow outgrows the
  // viewport — a spec card landing, a session banner appearing — so the
  // customer isn't handed a scrollbar the screen has room to absorb. Clamped
  // to the monitor's work area, and grow-only so it never fights a size the
  // user chose. The content column stays a scroll container for whatever the
  // work area can't absorb.
  let growTimer: ReturnType<typeof setTimeout> | undefined;
  let growObserver: ResizeObserver | undefined;

  async function growToFit(): Promise<void> {
    if (!contentEl) return;
    const overflow = contentEl.scrollHeight - contentEl.clientHeight;
    if (overflow <= 1) return;
    try {
      const { getCurrentWindow, LogicalSize, currentMonitor } = await import(
        "@tauri-apps/api/window"
      );
      const win = getCurrentWindow();
      const factor = await win.scaleFactor();
      // setSize sets the *inner* size; the decorations (title bar, borders)
      // ride on top of it, so the work-area clamp must subtract them.
      const inner = (await win.innerSize()).toLogical(factor);
      const outer = (await win.outerSize()).toLogical(factor);
      const decorations = Math.max(0, outer.height - inner.height);
      let target = inner.height + overflow;
      const monitor = await currentMonitor();
      if (monitor) {
        // Grow downward from where the window sits, but never past the work
        // area's bottom edge (the taskbar keeps its ground).
        const area = monitor.workArea ?? { position: monitor.position, size: monitor.size };
        const top = (await win.outerPosition()).toLogical(monitor.scaleFactor).y;
        const bottom = (area.position.y + area.size.height) / monitor.scaleFactor;
        target = Math.min(target, bottom - top - decorations);
      }
      if (target > inner.height + 1) {
        await win.setSize(new LogicalSize(inner.width, target));
      }
    } catch (e) {
      // Web mode has no window to grow; anything else deserves a trace —
      // a silent catch here once hid a missing capabilities grant.
      console.warn("grow-to-fit:", e);
    }
  }

  onMount(() => {
    void store.init();
    if (isTauri() && flowEl) {
      // Observe the inner flow, not the scroll container: the flow's box is
      // its content height, so it fires exactly when the page gets taller.
      growObserver = new ResizeObserver(() => {
        clearTimeout(growTimer);
        growTimer = setTimeout(() => void growToFit(), 120);
      });
      growObserver.observe(flowEl);
    }
  });
  onDestroy(() => {
    store.destroy();
    growObserver?.disconnect();
    clearTimeout(growTimer);
  });

  // Narrow the current request to a plain variable so it can be passed to the
  // modal as a non-null prop (a getter can't be narrowed by `{#if}`).
  let request = $derived(store.request);
</script>

<div class="shell">
  <header class="topbar">
    <div class="brand">
      <img class="mark" src={cecLogo} alt="" aria-hidden="true" />
      <span class="name">CEC <span class="name-dim">Support</span></span>
    </div>
    <!-- The customer's Support number, centered — the one they read to their
         technician. Always the middle grid cell (even before the node answers)
         so the brand stays left and Settings stays right. -->
    <div class="support" aria-live="polite">
      {#if store.status?.number}
        <span class="support-label">Support&nbsp;#</span>
        <span class="support-num">{store.grouped}</span>
      {/if}
    </div>
    <button
      class="btn ghost small header-action"
      aria-label={store.view === "settings" ? "Back" : "Settings"}
      onclick={() => (store.view = store.view === "settings" ? "start" : "settings")}
    >
      {#if store.view === "settings"}
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="19" y1="12" x2="5" y2="12" /><polyline points="12 19 5 12 12 5" />
        </svg>
        Back
      {:else}
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        Settings
      {/if}
    </button>
  </header>

  <main class="content" bind:this={contentEl}>
    <div class="flow" bind:this={flowEl}>
    {#if store.view === "settings"}
      <SettingsPanel />
    {:else}
      <!-- Same two-column shape as the number view. The rail has the room,
           so CEC's contact points simply live there — no appearing act. Live
           status shows on the front door too — a customer who asked for help
           never opens the number screen, but "X is viewing your screen" and
           "who can connect to your computer" must never be hidden behind a
           navigation step while they're true. -->
      <div class="cols">
        <div class="col">
          <StartScreen />
          <ConnectedBanner />
          <AccessList />
        </div>
        <aside class="col">
          <ContactCard />
          <SpecSheet />
          <KvmClaimCard />
        </aside>
      </div>
    {/if}
    </div>
  </main>

  {#if request}
    <ApproveModal {request} />
  {/if}

  <ConfirmDialog />

  {#if store.toast}
    <button class="toast" onclick={() => store.dismissToast()}>{store.toast}</button>
  {/if}
</div>

<style>
  .shell {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }

  .topbar {
    flex: 0 0 auto;
    /* Three cells — brand left, Support number dead-centre, Settings right —
       so the number stays centred regardless of the side widths. */
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 0.6rem;
    padding: 0.7rem 1.1rem;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-weight: 700;
    justify-self: start;
  }

  /* Centre cell. */
  .support {
    display: inline-flex;
    align-items: baseline;
    gap: 0.15rem;
    justify-self: center;
    min-width: 0;
    white-space: nowrap;
  }
  .support-label {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--ink-soft);
  }
  .support-num {
    font-family: var(--mono);
    font-size: 1.05rem;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--ink);
  }
  .header-action {
    justify-self: end;
  }
  .brand .mark {
    display: block;
    height: 2rem;
    width: auto;
  }
  .brand .name {
    font-family: var(--font-display);
    font-size: 1.25rem;
    font-weight: 700;
    letter-spacing: 0.01em;
  }
  .brand .name-dim {
    color: var(--dim);
    font-weight: 500;
  }
  .ico {
    width: 1rem;
    height: 1rem;
  }

  .content {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 1.4rem;
  }
  /* The measured column: its box height IS the content height, which is what
     the grow-to-fit observer watches (the scroll container's own box never
     changes when content does). */
  .flow {
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
    align-items: center;
  }
  /* Side-by-side when the window affords it: the tallest column sets the
     height the grow logic asks for; narrow windows stack back to one. */
  .cols {
    width: 100%;
    display: grid;
    grid-template-columns: minmax(0, 30rem) minmax(0, 30rem);
    justify-content: center;
    gap: 1.1rem;
    align-items: start;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
    align-items: center;
    min-width: 0;
  }
  @media (max-width: 880px) {
    .cols {
      grid-template-columns: minmax(0, 30rem);
    }
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: 1.3rem;
    transform: translateX(-50%);
    max-width: min(30rem, 90vw);
    text-align: left;
    border: 1px solid var(--line-strong);
    background: var(--ink);
    color: var(--bg);
    padding: 0.7rem 1rem;
    border-radius: var(--r-md);
    box-shadow: var(--shadow-md);
    font-size: 0.92rem;
    font-weight: 500;
    z-index: 60;
    animation: fade 0.14s ease;
  }
</style>
