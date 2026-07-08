<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { store } from "../store.svelte";
  import NumberScreen from "./NumberScreen.svelte";
  import ApproveModal from "./ApproveModal.svelte";
  import ConnectedBanner from "./ConnectedBanner.svelte";
  import AccessList from "./AccessList.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

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
      <span class="mark" aria-hidden="true">CEC</span>
      <span class="name">CEC Support</span>
    </div>
    <button
      class="btn ghost small"
      onclick={() => (store.view = store.view === "settings" ? "home" : "settings")}
    >
      {store.view === "settings" ? "← Back" : "⚙ Settings"}
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
    display: inline-grid;
    place-items: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--r-sm);
    background: var(--accent);
    color: #fff;
    font-size: 0.72rem;
    font-weight: 800;
    letter-spacing: 0.02em;
  }
  .brand .name {
    font-size: 1.02rem;
    letter-spacing: 0.01em;
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
