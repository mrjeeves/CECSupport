<script lang="ts">
  import { store } from "../store.svelte";

  let name = $state("");
  let seeded = $state(false);

  // Seed the name field once the status (with any saved label) arrives.
  $effect(() => {
    if (!seeded && store.status) {
      name = store.status.label ?? "";
      seeded = true;
    }
  });

  // The background-service install is parked for now: run-on-boot (below) is
  // the default way CEC Support survives a restart, and the service returns
  // when its story (reboot-and-reconnect mid-repair) is ready. Flip this to
  // bring the whole card back — the commands and the service crate stay wired.
  const SHOW_SERVICE = false;

  let serviceSupported = $derived(store.service?.supported ?? false);
  let serviceInstalled = $derived(store.service?.installed ?? false);

  // ---- updates ----
  const update = $derived(store.updateInfo);
  // An install that can't swap its own binary: a package manager owns it, or
  // it's a per-machine install this process can't write to. It still checks —
  // it just can't install what it finds.
  const managed = $derived(update?.install_kind === "package_manager");
  const checkResult = $derived(store.checkOutcomeText(store.updateOutcome));

  const lastChecked = $derived(
    update?.last_check_at
      ? new Date(update.last_check_at * 1000).toLocaleString()
      : "not yet",
  );

  type SettingsTab = "general" | "startup" | "updates";
  let tab = $state<SettingsTab>("general");

  function cleanVersion(v: string | null | undefined): string {
    return v?.replace(/^v/, "") || "Unknown";
  }

  function behind(current: string | null, pinned: string | null): boolean {
    if (!current || !pinned) return true;
    const have = cleanVersion(current).split(/[.-]/).slice(0, 3).map(Number);
    const want = cleanVersion(pinned).split(/[.-]/).slice(0, 3).map(Number);
    for (let i = 0; i < 3; i += 1) {
      if ((have[i] || 0) < (want[i] || 0)) return true;
      if ((have[i] || 0) > (want[i] || 0)) return false;
    }
    return false;
  }
</script>

<div class="settings-shell">
  <nav class="tabs" aria-label="Settings sections">
    <button class:active={tab === "general"} onclick={() => (tab = "general")}>General</button>
    <button class:active={tab === "startup"} onclick={() => (tab = "startup")}>Startup</button>
    <button class:active={tab === "updates"} onclick={() => { tab = "updates"; void store.loadUpdateStatus(); }}>Updates</button>
  </nav>

  <div class="settings">
  {#if tab === "general"}
  <section class="card block">
    <h3>This computer's name</h3>
    <p class="desc">A friendly name your technician will see, so they know it's you.</p>
    <div class="name-row">
      <input
        class="input"
        type="text"
        placeholder="e.g. Reception PC"
        maxlength="40"
        bind:value={name}
      />
      <button class="btn" onclick={() => void store.setLabel(name.trim())}>Save</button>
    </div>
  </section>

  {#if SHOW_SERVICE}
  <section class="card block">
    <h3>Background service</h3>
    <p class="desc">
      Keeps CEC Support connected after a restart, so your technician can finish a repair that
      needs the computer to reboot. You stay in control: every connection still needs your
      approval.
    </p>

    {#if !serviceSupported}
      <p class="muted">Not available on this system.</p>
    {:else if serviceInstalled}
      <div class="chip ok">Installed and running</div>
      <div class="btn-row">
        <button class="btn" disabled={store.busy} onclick={() => void store.stopService()}>
          Stop
        </button>
        <button class="btn danger" disabled={store.busy} onclick={() => void store.uninstallService()}>
          Uninstall
        </button>
      </div>
    {:else}
      <button class="btn primary big" disabled={store.busy} onclick={() => void store.installService()}>
        Install as a background service
      </button>
    {/if}
  </section>
  {/if}

  {:else if tab === "startup"}

  <section class="card block">
    <h3>Startup</h3>
    <p class="desc">
      When CEC Support opens with this computer. The default keeps a repair
      going across a restart: while your technician still has access, the app
      comes back on its own after you log in.
    </p>

    <div class="modes">
      <label class="mode primary" class:on={store.autostartMode === "while_granted"}>
        <input
          type="radio"
          name="autostart-mode"
          value="while_granted"
          checked={store.autostartMode === "while_granted"}
          onchange={() => void store.setAutostartMode("while_granted")}
        />
        <span class="mode-body">
          <span class="mode-title">Open with Windows while a technician has access</span>
          <span class="mode-hint">
            Recommended. Only starts on boot while a technician grant is active,
            so a repair survives a restart, then stops once their access ends.
          </span>
        </span>
      </label>

      <label class="mode" class:on={store.autostartMode === "always"}>
        <input
          type="radio"
          name="autostart-mode"
          value="always"
          checked={store.autostartMode === "always"}
          onchange={() => void store.setAutostartMode("always")}
        />
        <span class="mode-body">
          <span class="mode-title">Always open with Windows</span>
        </span>
      </label>

      <label class="mode" class:on={store.autostartMode === "off"}>
        <input
          type="radio"
          name="autostart-mode"
          value="off"
          checked={store.autostartMode === "off"}
          onchange={() => void store.setAutostartMode("off")}
        />
        <span class="mode-body">
          <span class="mode-title">Don't open with Windows</span>
        </span>
      </label>
    </div>

    <label class="toggle">
      <input
        type="checkbox"
        checked={store.keepBackground}
        onchange={(e) => void store.setKeepBackground(e.currentTarget.checked)}
      />
      <span>Keep running in the background when the window is closed</span>
    </label>
  </section>

  {:else if tab === "updates"}

  <section class="card block">
    <h3>Updates</h3>
    <p class="desc">
      CEC Support keeps itself up to date so your technician is always working with a version
      that matches theirs. It checks quietly in the background — you don't have to do anything.
    </p>

    {#if store.demo}
      <p class="muted">Updates are handled by the installed app.</p>
    {:else if !update}
      <p class="muted">Checking your version…</p>
    {:else}
      <div class="upd-head">
        <span class="muted">Version {update.current_version}</span>
        <button class="btn" disabled={store.updateBusy} onclick={() => void store.checkUpdates()}>
          {store.updateBusy ? "Checking…" : "Check now"}
        </button>
      </div>

      {#if checkResult && !store.updateBusy}
        <p class="muted">{checkResult}</p>
      {/if}

      <div class="component-list">
        {#if store.componentVersions.length === 0}
          <p class="muted">Reading component versions…</p>
        {:else}
          {#each store.componentVersions as row (row.id)}
            <div class="component-row" class:stale={behind(row.current, row.pinned)}>
              <div class="component-copy">
                <b>{row.label}</b>
                <span>{row.detail}</span>
              </div>
              <div class="versions">
                <span><small>Current</small><b>{cleanVersion(row.current)}</b></span>
                <span><small>Pinned</small><b>{cleanVersion(row.pinned)}</b></span>
              </div>
              <button class="btn repair" disabled={store.componentBusy !== null} onclick={() => void store.repairComponent(row.id)}>
                {store.componentBusy === row.id ? "Working…" : behind(row.current, row.pinned) ? "Update" : "Repair"}
              </button>
            </div>
          {/each}
        {/if}
      </div>

      {#if update.staged_version}
        <!-- Something is downloaded and verified, waiting on a restart. This is
             the whole point of the notification: it must be actionable right
             here, not buried behind a menu. -->
        <div class="chip ok">Version {update.staged_version} is ready to install</div>
        <div class="btn-row">
          <button
            class="btn primary"
            disabled={store.updateBusy}
            onclick={() => void store.applyUpdateAndRestart()}
          >
            Restart and update
          </button>
        </div>
      {/if}

      {#if managed}
        <p class="muted">
          This copy of CEC Support can't replace its own files — it was installed for all users,
          or through a package manager. It still checks for new versions and will tell you when
          one is out; installing it means running the installer again.
        </p>
      {:else}
        <label class="toggle">
          <input
            type="checkbox"
            checked={update.enabled}
            onchange={(e) => void store.setUpdatePrefs({ enabled: e.currentTarget.checked })}
          />
          <span>Keep CEC Support up to date automatically</span>
        </label>
      {/if}

      <p class="muted">Last checked: {lastChecked}</p>
    {/if}
  </section>

  <footer class="ver">
    CEC Support{store.version ? ` v${store.version}` : ""}
    · by Critical Error Computing
  </footer>
  {/if}
  </div>
</div>

<style>
  .settings-shell {
    width: 100%;
    max-width: 38rem;
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
  }
  .tabs {
    display: flex;
    gap: 0.25rem;
    padding: 0.25rem;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--surface-2);
    align-self: center;
  }
  .tabs button {
    border: 0;
    border-radius: calc(var(--r-md) - 0.18rem);
    padding: 0.5rem 0.9rem;
    background: transparent;
    color: var(--ink-soft);
    font: inherit;
    font-weight: 650;
    cursor: pointer;
  }
  .tabs button.active {
    color: var(--ink);
    background: var(--surface);
    box-shadow: var(--shadow-sm);
  }
  .settings {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .block {
    padding: 1.2rem;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  h3 {
    margin: 0;
    font-size: 1.05rem;
  }
  .desc {
    margin: 0;
    color: var(--ink-soft);
    font-size: 0.92rem;
    line-height: 1.45;
  }
  .muted {
    margin: 0;
    color: var(--ink-faint);
    font-size: 0.92rem;
  }

  .name-row {
    display: flex;
    gap: 0.5rem;
  }
  /* Version on the left, "Check now" on the right. */
  .upd-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .component-list {
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    overflow: hidden;
  }
  .component-row {
    display: grid;
    grid-template-columns: minmax(9rem, 1fr) auto auto;
    align-items: center;
    gap: 0.7rem;
    padding: 0.65rem 0.7rem;
    border-top: 1px solid var(--line);
  }
  .component-row:first-child { border-top: 0; }
  .component-row.stale { background: color-mix(in srgb, var(--danger-soft) 45%, transparent); }
  .component-copy { display: flex; flex-direction: column; min-width: 0; }
  .component-copy b { font-size: 0.84rem; }
  .component-copy span { color: var(--ink-faint); font-size: 0.7rem; }
  .versions { display: flex; gap: 0.6rem; }
  .versions span { min-width: 3.8rem; display: flex; flex-direction: column; }
  .versions small { color: var(--ink-faint); text-transform: uppercase; font-size: 0.6rem; letter-spacing: 0.04em; }
  .versions b { font-size: 0.76rem; }
  .repair { min-width: 4.2rem; }
  .input {
    flex: 1 1 auto;
    font-family: inherit;
    font-size: 1rem;
    padding: 0.6rem 0.8rem;
    border-radius: var(--r-md);
    border: 1px solid var(--line-strong);
    background: var(--surface);
    color: var(--ink);
  }
  .input:focus {
    outline: 2px solid var(--accent-soft);
    border-color: var(--accent);
  }

  .btn-row {
    display: flex;
    gap: 0.5rem;
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 0.95rem;
    cursor: pointer;
  }
  .toggle input {
    width: 1.15rem;
    height: 1.15rem;
    accent-color: var(--accent);
  }

  .modes {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-bottom: 0.9rem;
  }
  .mode {
    display: flex;
    align-items: flex-start;
    gap: 0.6rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    cursor: pointer;
  }
  .mode input {
    margin-top: 0.15rem;
    accent-color: var(--accent);
  }
  .mode.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  /* The default choice is the prominent one — bigger, leads the list. */
  .mode.primary {
    padding: 0.8rem 0.85rem;
  }
  .mode-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .mode-title {
    font-size: 0.95rem;
    font-weight: 600;
  }
  .mode.primary .mode-title {
    font-size: 1.02rem;
    font-weight: 700;
  }
  .mode-hint {
    font-size: 0.82rem;
    color: var(--ink-soft);
    line-height: 1.4;
  }

  .ver {
    text-align: center;
    color: var(--ink-faint);
    font-size: 0.82rem;
    padding: 0.5rem 0 1rem;
  }
  @media (max-width: 560px) {
    .component-row { grid-template-columns: 1fr auto; }
    .versions { grid-column: 1; }
    .repair { grid-column: 2; grid-row: 1 / span 2; }
  }
</style>
