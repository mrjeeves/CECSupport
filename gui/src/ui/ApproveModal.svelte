<script lang="ts">
  import { store } from "../store.svelte";
  import type { ApprovalScope, ConnectRequest } from "../types";

  let { request }: { request: ConnectRequest } = $props();

  // "Approve once" and the 3-hour window are the two choices a customer gets.
  // The "forever" / always-on grant is intentionally omitted for now — it's
  // reserved for a future premier-subscriber tier that doesn't exist yet, so we
  // don't offer it. (The wire still understands `forever`; we just don't show
  // it.)
  const choices: { scope: ApprovalScope; label: string; hint: string; cls: string }[] = [
    { scope: "once", label: "Approve once", hint: "Just for right now", cls: "primary" },
    {
      scope: "three_hours",
      label: "Auto-approve for 3 hours",
      hint: "Handy for a repair that may need a restart",
      cls: "",
    },
  ];
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
      Only continue if you're expecting <strong>{request.agent_name}</strong> to connect —
      that's the name your technician will have told you to look for.
    </p>

    <div class="choices">
      {#each choices as c (c.scope)}
        <button
          class={`btn ${c.cls}`}
          disabled={store.busy}
          onclick={() => void store.approve(request, c.scope)}
        >
          <span class="choice-main">{c.label}</span>
          <span class="choice-hint">{c.hint}</span>
        </button>
      {/each}
    </div>

    <button
      class="btn danger deny"
      disabled={store.busy}
      onclick={() => void store.deny(request)}
    >
      No thanks — don't connect
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
    gap: 0.6rem;
  }
  .choices .btn {
    flex-direction: column;
    align-items: stretch;
    gap: 0.1rem;
    padding: 0.75rem 1rem;
    text-align: center;
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

  .deny {
    margin-top: 0.15rem;
  }
</style>
