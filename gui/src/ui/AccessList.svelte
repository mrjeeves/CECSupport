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
            <!-- The name is the door into that technician's chat: tap it to open
                 the conversation (live while they're connected, a read-back of
                 the transcript otherwise). -->
            <button
              class="agent"
              onclick={() => store.openChat(g.technician)}
              title={`Open chat with ${g.agent_name || "this technician"}`}
            >
              <span class="agent-name">{g.agent_name || "A CEC technician"}</span>
              <svg class="chat-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z" />
              </svg>
              {#if store.chatUnreadFor(g.technician) > 0}
                <span class="unread" aria-label={`${store.chatUnreadFor(g.technician)} unread`}>
                  {store.chatUnreadFor(g.technician)}
                </span>
              {/if}
            </button>
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
    <p class="foot">Tap a technician's name to chat with them. “Forget” removes their standing access right away — they'll have to ask you again next time.</p>
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
  /* The name doubles as the chat door: a borderless button that reads as a
     name, with a speech-bubble hint and an unread count. */
  .agent {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    align-self: flex-start;
    max-width: 100%;
    padding: 0;
    font: inherit;
    font-weight: 600;
    font-size: 0.98rem;
    color: var(--ink);
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
  }
  .agent:hover {
    color: var(--accent);
  }
  .agent-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chat-ico {
    flex: 0 0 auto;
    width: 0.85rem;
    height: 0.85rem;
    color: var(--ink-faint);
  }
  .agent:hover .chat-ico {
    color: var(--accent);
  }
  .unread {
    flex: 0 0 auto;
    min-width: 1.1rem;
    height: 1.1rem;
    padding: 0 0.3rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 0.7rem;
    font-weight: 700;
    line-height: 1;
    color: var(--accent-ink);
    background: var(--accent);
    border-radius: var(--r-pill);
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
