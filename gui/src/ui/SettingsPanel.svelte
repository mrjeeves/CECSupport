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
</script>

<div class="settings">
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
      needs the computer to reboot. You stay in control — every connection still needs your
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

  <section class="card block">
    <h3>Startup</h3>
    <p class="desc">
      CEC Support opens with this computer by default, so your technician can
      reach you after a restart. Closing the window quits the app unless you
      choose to keep it waiting in the background.
    </p>
    <label class="toggle">
      <input
        type="checkbox"
        checked={store.autostart}
        onchange={(e) => void store.setAutostart(e.currentTarget.checked)}
      />
      <span>Open CEC Support when this computer starts</span>
    </label>
    <label class="toggle">
      <input
        type="checkbox"
        checked={store.keepBackground}
        onchange={(e) => void store.setKeepBackground(e.currentTarget.checked)}
      />
      <span>Keep running in the background when the window is closed</span>
    </label>
  </section>

  <footer class="ver">
    CEC Support{store.version ? ` v${store.version}` : ""}
    · by Critical Error Computing
  </footer>
</div>

<style>
  .settings {
    width: 100%;
    max-width: 30rem;
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

  .ver {
    text-align: center;
    color: var(--ink-faint);
    font-size: 0.82rem;
    padding: 0.5rem 0 1rem;
  }
</style>
