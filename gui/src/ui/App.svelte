<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { store } from "../store.svelte";
  import NumberScreen from "./NumberScreen.svelte";
  import ApproveModal from "./ApproveModal.svelte";
  import ConnectedBanner from "./ConnectedBanner.svelte";
  import AccessList from "./AccessList.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
  import cecLogo from "../assets/cec-logo.png";

  onMount(() => {
    void store.init();
  });
  onDestroy(() => store.destroy());

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
    <button
      class="btn ghost small"
      aria-label={store.view === "settings" ? "Back" : "Settings"}
      onclick={() => (store.view = store.view === "settings" ? "home" : "settings")}
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

  <main class="content">
    {#if store.view === "settings"}
      <SettingsPanel />
    {:else}
      <NumberScreen />
      <ConnectedBanner />
      <AccessList />
    {/if}
  </main>

  {#if request}
    <ApproveModal {request} />
  {/if}

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
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.7rem 1.1rem;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-weight: 700;
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
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
    align-items: center;
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
