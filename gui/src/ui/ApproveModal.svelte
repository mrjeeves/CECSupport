<script lang="ts">
  import { store } from "../store.svelte";
  import type { ApprovalScope, ConnectRequest } from "../types";

  let { request }: { request: ConnectRequest } = $props();

  // The 3-hour window is the DEFAULT, prominent choice: it's what lets a
  // technician restart the machine mid-repair and carry on once you've
  // logged back in, without you being re-asked. "Approve once" is the
  // smaller fallback for a one-off look. The "forever" / always-on grant is
  // intentionally omitted for now — reserved for a future premier tier that
  // doesn't exist yet. (The wire still understands `forever`; we just don't
  // show it.)
  const primary = {
    scope: "three_hours" as ApprovalScope,
    label: "Approve for 3 hours",
    hint: "Best for a repair. Stays connected through a restart, no need to re-approve",
  };
  const minor = {
    scope: "once" as ApprovalScope,
    label: "Just this once",
    hint: "A single connection, ends when they disconnect",
  };
</script>

<div class="scrim">
  <div class="card modal" role="dialog" aria-modal="true" aria-labelledby="approve-title">
    <div class="who">
      <span class="avatar" aria-hidden="true">{request.agent_name.slice(0, 1).toUpperCase()}</span>
      <div>
        <h2 id="approve-title">
          {request.agent_name} is trying to connect to your computer
        </h2>
        <p class="sub">
          They want to {request.want_control
            ? "see and control your screen"
            : "see your screen"} so they can help you.
        </p>
      </div>
    </div>

    <p class="verify">
      Only continue if you're expecting <strong>{request.agent_name}</strong> to connect.
      That's the name your technician will have told you to look for.
    </p>

    <div class="choices">
      <!-- Default + prominent: the 3-hour window that survives a restart. -->
      <button
        class="btn primary big"
        disabled={store.busy}
        onclick={() => void store.approve(request, primary.scope)}
      >
        <span class="choice-main">{primary.label}</span>
        <span class="choice-hint">{primary.hint}</span>
      </button>

      <!-- Smaller fallback: a single one-off connection. -->
      <button
        class="btn minor"
        disabled={store.busy}
        onclick={() => void store.approve(request, minor.scope)}
      >
        <span class="choice-main">{minor.label}</span>
        <span class="choice-hint">{minor.hint}</span>
      </button>
    </div>

    <button
      class="btn danger deny"
      disabled={store.busy}
      onclick={() => void store.deny(request)}
    >
      No thanks, don't connect
    </button>
  </div>
</div>

<style>
  .modal {
    width: min(28rem, 100%);
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1.15rem;
  }

  .who {
    display: flex;
    gap: 0.9rem;
    align-items: flex-start;
  }
  .avatar {
    flex: 0 0 auto;
    display: inline-grid;
    place-items: center;
    width: 2.6rem;
    height: 2.6rem;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent-ink);
    font-weight: 700;
    font-size: 1.1rem;
  }
  h2 {
    margin: 0 0 0.25rem;
    font-size: 1.2rem;
    line-height: 1.3;
  }
  .sub {
    margin: 0;
    color: var(--ink-soft);
    font-size: 0.95rem;
    line-height: 1.4;
  }

  .verify {
    margin: 0;
    padding: 0.8rem 0.95rem;
    border-radius: var(--r-md);
    background: var(--surface-2);
    border: 1px solid var(--line);
    font-size: 0.9rem;
    color: var(--ink-soft);
    text-align: center;
    line-height: 1.4;
  }
  .verify strong {
    color: var(--ink);
    font-weight: 650;
  }

  .choices {
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
  }
  .choices .btn {
    flex-direction: column;
    align-items: stretch;
    gap: 0.1rem;
    padding: 0.75rem 1rem;
    text-align: center;
  }
  /* The default choice reads as the obvious one — larger, full-weight. */
  .choices .btn.big {
    padding: 0.95rem 1rem;
  }
  .choices .btn.big .choice-main {
    font-size: 1.08rem;
    font-weight: 700;
  }
  /* The fallback is deliberately quieter: compact, ghost-styled, so the eye
     lands on the 3-hour default first. */
  .choices .btn.minor {
    padding: 0.55rem 1rem;
    background: transparent;
    border: 1px solid var(--line-strong, var(--line));
    color: var(--ink-soft);
  }
  .choices .btn.minor:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--ink);
  }
  .choices .btn.minor .choice-main {
    font-size: 0.92rem;
    font-weight: 600;
  }
  .choice-main {
    font-size: 1rem;
    font-weight: 650;
  }
  .choice-hint {
    font-size: 0.8rem;
    font-weight: 500;
    opacity: 0.85;
  }
  .choices .btn.minor .choice-hint {
    opacity: 0.75;
  }

  .deny {
    margin-top: 0.15rem;
  }
</style>
