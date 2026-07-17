<script lang="ts">
  // "Who can connect to your computer" — the single place standing access AND
  // live connection state show. Each row is a technician: their grant, a
  // status dot (above Forget) for whether they're connected right now, and a
  // green "Viewing your screen" chip that breathes while a session is live.
  // Forget is the one kill-switch: it disconnects them and removes their
  // access in the same tap (the old separate banner + Disconnect button are
  // gone — this list is where the truth lives now).
  import { store } from "../store.svelte";
  import { grantScope } from "../types";
  import type { AccessRow, ApprovalScope, Grant } from "../types";

  const scopeLabel: Record<ApprovalScope, string> = {
    once: "This session only",
    three_hours: "Auto-approve for 3 hours",
    forever: "Auto-approve",
  };

  // The old footer paragraph, now behind the ? next to the title — hover for
  // the tooltip, tap to pin it open (touch has no hover).
  const HELP_TEXT =
    "Tap Chat to message a technician. Forget disconnects them and removes " +
    "their access right away. They'll have to ask you again next time.";
  let helpOpen = $state(false);

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

  function access(r: AccessRow): string {
    const control = r.grant ? r.grant.control : (r.live?.want_control ?? false);
    return control ? "View and control" : "View only";
  }

  type DotState = "live" | "connecting" | "off";
  function dotState(r: AccessRow): DotState {
    if (r.live?.state === "active") return "live";
    if (r.live?.state === "connecting") return "connecting";
    return "off";
  }
  const dotTitle: Record<DotState, string> = {
    live: "Connected right now",
    connecting: "Connecting…",
    off: "Not connected",
  };
</script>

{#if store.accessRows.length > 0}
  <section class="access card">
    <header class="head">
      <h3>Who can connect to your computer</h3>
      <button
        class="help"
        title={HELP_TEXT}
        aria-label="How this list works"
        aria-expanded={helpOpen}
        onclick={() => (helpOpen = !helpOpen)}
      >?</button>
    </header>
    {#if helpOpen}
      <p class="help-pop">{HELP_TEXT}</p>
    {/if}
    <ul>
      {#each store.accessRows as r (r.key)}
        {@const dot = dotState(r)}
        <li>
          <div class="info">
            <div class="who">
              <span class="agent-name">{r.agent_name}</span>
              <button
                class="btn small chat"
                onclick={() => store.openChat(r.technician)}
                title={`Chat with ${r.agent_name}`}
              >
                <svg class="chat-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z" />
                </svg>
                Chat
                {#if store.chatUnreadFor(r.technician) > 0}
                  <span class="unread" aria-label={`${store.chatUnreadFor(r.technician)} unread`}>
                    {store.chatUnreadFor(r.technician)}
                  </span>
                {/if}
              </button>
            </div>
            <span class="meta">
              {r.grant ? scopeLabel[grantScope(r.grant)] : scopeLabel.once} · {access(r)}
              {#if r.grant && remaining(r.grant)}
                <span class="chip warn">{remaining(r.grant)}</span>
              {/if}
              {#if dot === "live"}
                <span class="chip ok viewing">
                  {r.live?.want_control ? "Controlling your screen" : "Viewing your screen"}
                </span>
              {/if}
            </span>
          </div>
          <div class="side">
            <span class="dot {dot}" role="img" title={dotTitle[dot]} aria-label={dotTitle[dot]}></span>
            <button class="btn small danger" onclick={() => void store.forget(r.technician)}>
              Forget
            </button>
          </div>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .access {
    width: 100%;
    max-width: 30rem;
    padding: 1.1rem 1.2rem;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 0.7rem;
  }
  h3 {
    margin: 0;
    font-size: 1rem;
    flex: 1 1 auto;
  }
  /* The ? — a small round button that carries the old footer text as its
     tooltip, and pins it open on tap. */
  .help {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    padding: 0;
    font-size: 0.85rem;
    font-weight: 700;
    color: var(--ink-soft);
    background: transparent;
    border: 1px solid var(--line);
    border-radius: var(--r-pill);
    cursor: pointer;
  }
  .help:hover,
  .help[aria-expanded="true"] {
    color: var(--ink);
    border-color: var(--line-strong);
    background: var(--surface-2);
  }
  .help-pop {
    margin: 0 0 0.7rem;
    padding: 0.55rem 0.7rem;
    font-size: 0.82rem;
    line-height: 1.45;
    color: var(--ink-soft);
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r-md);
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
    gap: 0.3rem;
    min-width: 0;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    min-width: 0;
  }
  .agent-name {
    font-weight: 600;
    font-size: 0.98rem;
    color: var(--ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* An actual button that says Chat — the speech bubble alone was invisible. */
  .btn.chat {
    flex: 0 0 auto;
    gap: 0.35rem;
    padding: 0.25rem 0.6rem;
    font-size: 0.8rem;
  }
  .chat-ico {
    width: 0.85rem;
    height: 0.85rem;
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
  /* The live chip: green, breathing slowly while they're on your screen. */
  .chip.viewing {
    animation: viewing-pulse 2.8s ease-in-out infinite;
  }
  @keyframes viewing-pulse {
    50% {
      opacity: 0.6;
    }
  }
  /* The connection column: status dot sitting above the one kill-switch. */
  .side {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.4rem;
  }
  .dot {
    width: 0.7rem;
    height: 0.7rem;
    border-radius: 50%;
    background: transparent;
    border: 1px solid var(--line-strong);
  }
  .dot.live {
    background: var(--ok);
    border-color: var(--ok);
    box-shadow: 0 0 0 3px var(--ok-soft);
  }
  .dot.connecting {
    background: var(--warn);
    border-color: var(--warn);
    box-shadow: 0 0 0 3px var(--warn-soft);
    animation: connecting-pulse 1.4s ease-in-out infinite;
  }
  @keyframes connecting-pulse {
    50% {
      opacity: 0.45;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .chip.viewing,
    .dot.connecting {
      animation: none;
    }
  }
</style>
