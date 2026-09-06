<script lang="ts">
  import { store } from "../store.svelte";
  import { copyToClipboard } from "../tauri";
  import ChatPanel from "./ChatPanel.svelte";

  let copied = $state(false);
  async function copyNumber(): Promise<void> {
    const number = store.status?.number;
    if (number && await copyToClipboard(number)) {
      copied = true;
      setTimeout(() => (copied = false), 1600);
    }
  }
</script>

{#if store.activeChatPeer}
  <ChatPanel peer={store.activeChatPeer} />
{:else}
  <section class="start card" aria-labelledby="support-number-title">
    <h2 id="support-number-title">Your support number</h2>
    <p>Share this number with your CEC technician to start a support session.</p>
    {#if store.grouped}
      <button class="number" onclick={copyNumber} title="Copy your support number"
        aria-label={`Support number ${store.grouped}. Tap to copy.`}>
        <span class="digits">{store.grouped}</span>
        <span class="copy" aria-live="polite">{copied ? "✓ Copied" : "Tap to copy"}</span>
      </button>
    {:else}
      <div class="starting" role="status">Getting your support number…</div>
    {/if}
    <p class="consent">When they connect, approve your technician by name. Nothing is shared until you approve.</p>
  </section>
{/if}

<style>
  .start { width: 100%; max-width: 30rem; padding: 2rem 1.6rem; text-align: center; }
  h2 { margin: 0 0 0.6rem; font-family: var(--font-display); font-size: 1.5rem; }
  p { margin: 0; color: var(--ink-soft); font-size: 0.9rem; line-height: 1.5; }
  .number { display: flex; flex-direction: column; align-items: center; gap: 0.5rem; width: 100%; margin: 1.4rem 0; padding: 1.3rem 0.5rem; border: 2px solid var(--accent); border-radius: var(--r-md); background: var(--surface); color: var(--ink); cursor: pointer; }
  .number:hover { background: var(--surface-2); }
  .number:focus-visible { outline: 3px solid var(--accent); outline-offset: 3px; }
  .digits { font-size: clamp(1.8rem, 5vw, 2.7rem); font-weight: 750; font-variant-numeric: tabular-nums; letter-spacing: 0.04em; white-space: nowrap; }
  .copy { font-size: 0.75rem; color: var(--ink-soft); }
  .starting { margin: 1.4rem 0; padding: 1.3rem 0; color: var(--ink-soft); }
  .consent { font-size: 0.8rem; }
</style>
