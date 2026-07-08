<script lang="ts">
  import { store } from "../store.svelte";
  import type { LiveSession } from "../types";

  function what(s: LiveSession): string {
    if (s.state === "connecting") return "is connecting…";
    return s.want_control ? "is connected — viewing and controlling your screen" : "is connected — viewing your screen";
  }
</script>

{#if store.liveSessions.length > 0}
  <section class="banners">
    {#each store.liveSessions as s (s.session_id)}
      <div class="banner card">
        <span class="live" aria-hidden="true"></span>
        <p class="text"><strong>{s.agent_name}</strong> {what(s)}</p>
        <button class="btn danger small" onclick={() => void store.disconnect(s)}>Disconnect</button>
      </div>
    {/each}
  </section>
{/if}

<style>
  .banners {
    width: 100%;
    max-width: 30rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 0.7rem;
    padding: 0.75rem 0.95rem;
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .live {
    flex: 0 0 auto;
    width: 0.7rem;
    height: 0.7rem;
    border-radius: 50%;
    background: var(--accent);
    animation: blink 1.4s ease-in-out infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.35;
    }
  }
  .text {
    margin: 0;
    flex: 1 1 auto;
    font-size: 0.95rem;
    line-height: 1.35;
    color: var(--ink);
  }
</style>
