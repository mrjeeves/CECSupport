<script lang="ts">
  import { store } from "../store.svelte";
  import { copyToClipboard } from "../tauri";

  let copied = $state(false);

  async function copy(): Promise<void> {
    const n = store.status?.number ?? "";
    if (!n) return;
    if (await copyToClipboard(n)) {
      copied = true;
      setTimeout(() => (copied = false), 1800);
    }
  }
</script>

<section class="hero card">
  <p class="eyebrow">Your support number</p>

  {#if store.status}
    <p class="number" aria-label={`Support number ${store.grouped}`}>{store.grouped}</p>

    <div class="row">
      <button class="btn primary" onclick={copy}>
        {copied ? "✓ Copied" : "Copy number"}
      </button>
    </div>

    <p class="hint">
      Read this number to your CEC technician so they can connect to help you.
    </p>

    <div class="status" class:waiting={store.hosting}>
      {#if store.hosting}
        <span class="dot" aria-hidden="true"></span>
        <span>Waiting for your technician to connect…</span>
      {:else}
        <span>Sharing is off — no one can connect right now.</span>
        <button class="btn small" onclick={() => store.setHosting(true)}>Turn on</button>
      {/if}
    </div>
  {:else}
    <p class="number placeholder">— — — —</p>
    <p class="hint">Starting up…</p>
  {/if}

  {#if store.demo}
    <button class="btn ghost small demo" onclick={() => store.simulateDial()}>
      Preview the approval prompt
    </button>
  {/if}
</section>

<style>
  .hero {
    width: 100%;
    max-width: 30rem;
    padding: 2rem 1.6rem 1.6rem;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.9rem;
  }

  .eyebrow {
    margin: 0;
    font-size: 0.9rem;
    font-weight: 600;
    color: var(--ink-soft);
    letter-spacing: 0.02em;
  }

  .number {
    margin: 0;
    font-family: var(--mono);
    font-size: clamp(2.6rem, 12vw, 3.8rem);
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--ink);
    line-height: 1.05;
  }
  .number.placeholder {
    color: var(--ink-faint);
  }

  .row {
    display: flex;
    gap: 0.6rem;
  }

  .hint {
    margin: 0;
    max-width: 24rem;
    color: var(--ink-soft);
    font-size: 1rem;
    line-height: 1.45;
  }

  .status {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-top: 0.4rem;
    padding: 0.55rem 0.9rem;
    border-radius: var(--r-pill);
    background: var(--surface-2);
    color: var(--ink-soft);
    font-size: 0.9rem;
    font-weight: 500;
  }
  .status.waiting {
    background: var(--ok-soft);
    color: var(--ok);
  }
  .dot {
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 50%;
    background: var(--ok);
    box-shadow: 0 0 0 0 var(--ok);
    animation: pulse 1.8s ease-out infinite;
  }
  @keyframes pulse {
    0% {
      box-shadow: 0 0 0 0 rgba(26, 143, 76, 0.5);
    }
    70% {
      box-shadow: 0 0 0 0.5rem rgba(26, 143, 76, 0);
    }
    100% {
      box-shadow: 0 0 0 0 rgba(26, 143, 76, 0);
    }
  }

  .demo {
    margin-top: 0.4rem;
    color: var(--ink-faint);
  }
</style>
