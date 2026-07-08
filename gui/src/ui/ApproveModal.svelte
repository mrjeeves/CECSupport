<script lang="ts">
  import { store } from "../store.svelte";
  import type { ApprovalScope, ConnectRequest } from "../types";

  let { request }: { request: ConnectRequest } = $props();

  const choices: { scope: ApprovalScope; label: string; hint: string; cls: string }[] = [
    { scope: "once", label: "Approve once", hint: "Just for right now", cls: "primary" },
    {
      scope: "three_hours",
      label: "Auto-approve for 3 hours",
      hint: "Handy for a repair that may need a restart",
      cls: "",
    },
    {
      scope: "forever",
      label: "Auto-approve from now on",
      hint: "Until you remove them in Settings",
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

    <div class="code">
      <span class="code-label">Verification code</span>
      <span class="code-value">{request.verification_code}</span>
      <span class="code-hint">Only continue if your technician reads out this same code.</span>
    </div>

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

  .code {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.3rem;
    padding: 0.9rem;
    border-radius: var(--r-md);
    background: var(--surface-2);
    border: 1px dashed var(--line-strong);
  }
  .code-label {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--ink-faint);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .code-value {
    font-family: var(--mono);
    font-size: 1.9rem;
    font-weight: 700;
    letter-spacing: 0.24em;
    color: var(--ink);
  }
  .code-hint {
    font-size: 0.82rem;
    color: var(--ink-soft);
    text-align: center;
    line-height: 1.35;
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
