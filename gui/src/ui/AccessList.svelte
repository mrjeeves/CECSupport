<script lang="ts">
  import { store } from "../store.svelte";
  import { grantScope } from "../types";
  import type { ApprovalScope, Grant } from "../types";

  const scopeLabel: Record<ApprovalScope, string> = {
    once: "This session only",
    three_hours: "Auto-approve for 3 hours",
    forever: "Auto-approve",
  };

  function remaining(g: Grant): string | null {
    if (g.expires_at == null) return null;
    const s = g.expires_at - store.now;
    if (s <= 0) return "expiring…";
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    if (h > 0) return `${h}h ${m}m left`;
    if (m > 0) return `${m}m left`;
    return `${s}s left`;
  }

  function access(g: Grant): string {
    return g.control ? "View and control" : "View only";
  }
</script>

{#if store.grants.length > 0}
  <section class="access card">
    <h3>Who can connect to your computer</h3>
    <ul>
      {#each store.grants as g (g.technician)}
        <li>
          <div class="info">
            <span class="agent">{g.agent_name || "A CEC technician"}</span>
            <span class="meta">
              {scopeLabel[grantScope(g)]} · {access(g)}
              {#if remaining(g)}<span class="chip warn">{remaining(g)}</span>{/if}
            </span>
          </div>
          <button class="btn small danger" onclick={() => void store.forget(g.technician)}>
            Forget
          </button>
        </li>
      {/each}
    </ul>
    <p class="foot">“Forget” removes their standing access right away. They'll have to ask you again next time.</p>
  </section>
{/if}

<style>
  .access {
    width: 100%;
    max-width: 30rem;
    padding: 1.1rem 1.2rem;
  }
  h3 {
    margin: 0 0 0.7rem;
    font-size: 1rem;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  li {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 0.7rem;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--surface-2);
  }
  .info {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    min-width: 0;
  }
  .agent {
    font-weight: 600;
    font-size: 0.98rem;
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem;
    font-size: 0.85rem;
    color: var(--ink-soft);
  }
  .foot {
    margin: 0.9rem 0 0;
    font-size: 0.85rem;
    color: var(--ink-faint);
    line-height: 1.4;
  }
</style>
