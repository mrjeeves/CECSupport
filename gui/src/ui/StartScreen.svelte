<script lang="ts">
  // The front door: raise your hand and a technician connects. The support
  // number is the quiet fallback — for a technician who asks for it, or to
  // confirm you're you — so it just sits here, on the card, labelled and
  // one tap to copy. (There's no separate "number screen" any more: the
  // number was never worth a whole view of its own.)
  import { store } from "../store.svelte";
  import { copyToClipboard } from "../tauri";
  import ChatPanel from "./ChatPanel.svelte";

  let copied = $state(false);

  async function copyNumber(): Promise<void> {
    const n = store.status?.number ?? "";
    if (!n) return;
    if (await copyToClipboard(n)) {
      copied = true;
      setTimeout(() => (copied = false), 1600);
    }
  }
</script>

{#if store.activeChatPeer}
  <!-- Connected (or reviewing an earlier conversation): the front door becomes
       the chat with that technician. The store swaps this in on connect and back
       out to the Ask-for-help card on disconnect. -->
  <ChatPanel peer={store.activeChatPeer} />
{:else}
<section class="start card">
  {#if store.askingHelp}
    <div class="waiting" role="status" aria-live="polite">
      <!-- Two honest phases, keyed to the node's dispatched-to count: the
           hand is either still going up (no beacon has reached a watcher
           yet — spinner) or provably seen (a live watcher received it —
           breathing dot). No decorative waiting. -->
      {#if (store.helpWatchers ?? 0) > 0}
        <span class="pulse" aria-hidden="true"></span>
        <h2>Help is on the way</h2>
        <p class="sub">
          CEC can see your hand — a technician will connect shortly. Keep
          this window open. Nothing is shared until you approve them by name.
        </p>
      {:else}
        <span class="hand-raise" aria-hidden="true">✋</span>
        <h2>Raising your hand…</h2>
        <p class="sub">
          Connecting to CEC — this usually takes a few seconds. Keep this
          window open. Nothing is shared until you approve someone by name.
        </p>
      {/if}
      <button class="btn danger" disabled={store.busy} onclick={() => void store.cancelHelp()}>
        Stop asking
      </button>
    </div>
  {:else}
    <h2>How can we help?</h2>
    <div class="choices">
      <button class="btn primary big" disabled={store.busy} onclick={() => void store.askHelp()}>
        Ask for help
      </button>
      <p class="sub">
        Raise your hand — a CEC technician sees your request and connects.
        You approve them before they can see anything.
      </p>
      <!-- The number is the quiet fallback, not a second front door: it only
           matters when a technician asks for it, or to confirm you're you.
           So it's shown right here — labelled, and one tap to copy — instead
           of hidden behind a "show number" button and its own screen. -->
      {#if store.grouped}
        <div class="alt">
          <button
            class="numbtn"
            onclick={copyNumber}
            title="Copy your support number"
            aria-label={`Support number ${store.grouped}. Tap to copy.`}
          >
            <span class="numlabel">Your support number</span>
            <span class="num">{store.grouped}</span>
            <span class="copyhint">{copied ? "✓ Copied" : "Tap to copy"}</span>
          </button>
          <p class="sub tiny">If we ask for your number — or you'd like to confirm it.</p>
        </div>
      {/if}
    </div>
  {/if}
</section>
{/if}

<style>
  .start {
    width: 100%;
    max-width: 30rem;
    padding: 2rem 1.6rem 1.8rem;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.6rem;
  }
  h2 {
    margin: 0 0 0.4rem;
    font-family: var(--font-display);
    font-size: 1.5rem;
  }
  .choices {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0.45rem;
  }
  .btn.big {
    font-size: 1.05rem;
    padding: 0.85rem 1rem;
  }
  .sub {
    margin: 0 0 0.9rem;
    font-size: 0.85rem;
    color: var(--ink-soft);
    line-height: 1.45;
  }
  /* The demoted fallback: a slim button and a whispered caption, clearly
     subordinate to the Ask button above. */
  .alt {
    margin-top: 0.4rem;
    padding-top: 0.9rem;
    border-top: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.35rem;
  }
  /* The number, presented as the card's quiet identifier: a tap-to-copy
     block, not a call-to-action button. Labelled above, big tabular digits,
     a copy hint below. */
  .numbtn {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.15rem;
    width: 100%;
    padding: 0.55rem 0.9rem;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--r-pill, 0.7rem);
    cursor: pointer;
    color: inherit;
  }
  .numbtn:hover {
    border-color: var(--accent);
  }
  .numlabel {
    font-size: 0.7rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--ink-faint);
  }
  .num {
    font-size: 1.35rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.06em;
    color: var(--ink);
  }
  .copyhint {
    font-size: 0.7rem;
    color: var(--ink-soft);
  }
  .sub.tiny {
    margin: 0;
    font-size: 0.75rem;
    color: var(--ink-faint);
  }

  .waiting {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.7rem;
  }
  /* The "you're live" beacon — a soft breathing dot, calm rather than urgent. */
  .pulse {
    width: 0.9rem;
    height: 0.9rem;
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
  /* The hand-going-up phase: an actual hand that rises into view and gives a
     small friendly wave — so the tap lands on a warm "we see you" beat rather
     than a bare loader while the ask travels to CEC. */
  .hand-raise {
    font-size: 2.6rem;
    line-height: 1;
    display: inline-block;
    transform-origin: 70% 90%; /* pivot near the wrist for a natural wave */
    animation:
      hand-rise 0.5s cubic-bezier(0.34, 1.56, 0.64, 1) both,
      hand-wave 1.5s ease-in-out 0.55s infinite;
  }
  @keyframes hand-rise {
    from {
      transform: translateY(0.5rem) scale(0.8);
      opacity: 0;
    }
    to {
      transform: translateY(0) scale(1);
      opacity: 1;
    }
  }
  @keyframes hand-wave {
    0%,
    100% {
      transform: rotate(0deg);
    }
    30% {
      transform: rotate(-13deg);
    }
    70% {
      transform: rotate(13deg);
    }
  }
  /* Reduced motion: still show the hand (it's the reassurance), just settled
     and still — no rise, no wave. */
  @media (prefers-reduced-motion: reduce) {
    .hand-raise {
      animation: none;
    }
  }
</style>
