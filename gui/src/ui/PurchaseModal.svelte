<script lang="ts">
  // The diagnostic-purchase prompt — raised ONLY by the technician, mid-repair
  // ("before we continue, there's a $50 diagnostic session"). One prompt,
  // four beats, all driven by `purchase.state` from the store:
  //
  //   requested/seen  → the ask: what it is, what it costs, one honest button
  //   opened          → "finish up in your browser", with the claim button
  //   claimed         → "confirming with your technician…" (they check the
  //                     store for the order — human confirmation, no webhooks)
  //   confirmed       → "you're all set", then it gets out of the way
  //
  // Payment never happens in here: the button hands off to the customer's own
  // browser (our purchase page → the store's hosted checkout), so card details
  // stay where they belong. Declining is always available until it's settled.
  import { store } from "../store.svelte";
  import type { Purchase } from "../types";

  let { purchase }: { purchase: Purchase } = $props();

  const agent = $derived(purchase.agent_name || "Your technician");
  const item = $derived(purchase.item || "CEC Diagnostic Session");
  const price = $derived(purchase.price || "$50");
  const stage = $derived(
    purchase.state === "confirmed"
      ? "confirmed"
      : purchase.state === "claimed"
        ? "claimed"
        : purchase.state === "opened"
          ? "opened"
          : "asked",
  );
</script>

<div class="scrim">
  <div class="card modal" role="dialog" aria-modal="true" aria-labelledby="purchase-title">
    {#if stage === "confirmed"}
      <div class="settled">
        <span class="check" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="20 6 9 17 4 12" />
          </svg>
        </span>
        <h2 id="purchase-title">You're all set</h2>
        <p class="sub">
          {agent} confirmed your purchase — your repair carries on. A receipt is
          on its way to your email.
        </p>
        <button class="btn primary big" onclick={() => store.dismissPurchase()}>Done</button>
      </div>
    {:else if stage === "claimed"}
      <div class="settled">
        <span class="wait-dot" aria-hidden="true"></span>
        <h2 id="purchase-title">Thanks — confirming with {agent}</h2>
        <p class="sub">
          They're just checking your order came through. This usually takes a
          moment; you can stay on the line with them.
        </p>
        <p class="verify">
          Already paid but stuck here? Tell your technician — they can confirm
          it from their side.
        </p>
      </div>
    {:else}
      <div class="who">
        <span class="avatar" aria-hidden="true">{agent.slice(0, 1).toUpperCase()}</span>
        <div>
          <h2 id="purchase-title">
            {agent} is asking you to complete a purchase
          </h2>
          <p class="sub">
            To continue with your repair, they're asking for the diagnostic
            session below.
          </p>
        </div>
      </div>

      <div class="order" aria-label={`${item}, ${price}`}>
        <div class="order-what">
          <b>{item}</b>
          {#if purchase.note}
            <span class="order-note">{purchase.note}</span>
          {:else}
            <span class="order-note">A deep look at what's wrong, by a real CEC technician.</span>
          {/if}
        </div>
        <span class="order-price">{price}</span>
      </div>

      {#if stage === "opened"}
        <p class="verify">
          We opened our secure checkout in your web browser — finish the
          purchase there, then come back and tap
          <strong>I've completed my purchase</strong>.
        </p>
        <div class="choices">
          <button class="btn primary" disabled={store.busy} onclick={() => store.claimPurchase()}>
            <span class="choice-main">I've completed my purchase</span>
            <span class="choice-hint">{agent} will confirm it on their side</span>
          </button>
          <button class="btn" disabled={store.busy} onclick={() => void store.openCheckout()}>
            <span class="choice-main">Reopen the checkout</span>
            <span class="choice-hint">If the browser window got lost</span>
          </button>
        </div>
      {:else}
        <p class="verify">
          You'll pay on our <strong>secure checkout</strong> in your own web
          browser — CEC never sees your card details, and your receipt arrives
          by email.
        </p>
        <div class="choices">
          <button class="btn primary" disabled={store.busy} onclick={() => void store.openCheckout()}>
            <span class="choice-main">Open secure checkout · {price}</span>
            <span class="choice-hint">Opens in your web browser</span>
          </button>
        </div>
      {/if}

      <button class="btn danger deny" disabled={store.busy} onclick={() => store.declinePurchase()}>
        No thanks — not right now
      </button>
    {/if}
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

  /* The order line — what you're buying and what it costs, no fine print. */
  .order {
    display: flex;
    align-items: center;
    gap: 0.9rem;
    padding: 0.85rem 1rem;
    border-radius: var(--r-md);
    background: var(--surface-2);
    border: 1px solid var(--line);
  }
  .order-what {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
    flex: 1;
  }
  .order-what b {
    font-size: 0.98rem;
    font-weight: 650;
  }
  .order-note {
    font-size: 0.82rem;
    color: var(--ink-soft);
    line-height: 1.35;
  }
  .order-price {
    font-family: var(--mono);
    font-size: 1.35rem;
    font-weight: 700;
    color: var(--ink);
    flex-shrink: 0;
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

  /* The two settling beats — waiting on the technician, then the good news. */
  .settled {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 0.9rem;
    padding: 0.5rem 0.25rem 0.25rem;
  }
  .settled h2 {
    margin: 0;
  }
  .settled .sub {
    max-width: 22rem;
  }
  .settled .btn {
    margin-top: 0.35rem;
  }
  .check {
    display: inline-grid;
    place-items: center;
    width: 3.2rem;
    height: 3.2rem;
    border-radius: 50%;
    background: var(--ok-soft);
    color: var(--ok);
  }
  .check svg {
    width: 1.7rem;
    height: 1.7rem;
  }
  .wait-dot {
    width: 0.85rem;
    height: 0.85rem;
    border-radius: 50%;
    background: var(--accent);
    animation: breathe 1.6s ease-in-out infinite;
  }
  @keyframes breathe {
    0%,
    100% {
      transform: scale(1);
      opacity: 1;
    }
    50% {
      transform: scale(1.35);
      opacity: 0.55;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wait-dot {
      animation: none;
    }
  }
</style>
