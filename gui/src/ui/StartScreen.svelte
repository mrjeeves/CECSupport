<script lang="ts">
  // The front door: two ways to get helped. "Ask for help" beacons this
  // machine onto the global help room — every CEC technician sees it in
  // their queue and one of them answers (the customer still approves them
  // before anything is shown). "Show Support Number" is the classic phone
  // flow: read the number out to the technician you're already talking to.
  import { store } from "../store.svelte";
</script>

<section class="start card">
  {#if store.askingHelp}
    <div class="waiting" role="status" aria-live="polite">
      <span class="pulse" aria-hidden="true"></span>
      <h2>Help is on the way</h2>
      <p class="sub">
        You're in line — a CEC technician will connect shortly. Keep this
        window open. Nothing is shared until you approve them by name.
      </p>
      <!-- CEC's contact card renders just below this card (App mounts it
           whenever the ask is live or was ever raised); the identity pair we
           match on lives in the app's permanent bottom bar. -->
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
           matters when a technician asks for it, or to confirm you're you. -->
      <div class="alt">
        <button class="btn ghost small" onclick={() => (store.view = "number")}>
          Show Support Number
        </button>
        <p class="sub tiny">If we ask for your number — or you'd like to confirm it.</p>
      </div>
    </div>
  {/if}
</section>

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
  .alt .btn {
    align-self: center;
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
</style>
