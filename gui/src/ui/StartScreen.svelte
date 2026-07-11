<script lang="ts">
  // The front door: two ways to get helped. "Ask for help" beacons this
  // machine onto the global help room — every CEC technician sees it in
  // their queue and one of them answers (the customer still approves them
  // before anything is shown). "Show Support Number" is the classic phone
  // flow: read the number out to the technician you're already talking to.
  import { store } from "../store.svelte";
  import { openTiktok } from "../tauri";

  /** "Name (HOSTNAME)" — the same pair the technician's card shows, so the
   *  customer can read it out and both sides match word for word. */
  const computerName = $derived.by(() => {
    const label = store.status?.label?.trim() ?? "";
    const host = store.specs?.hostname?.trim() ?? "";
    if (label && host && label.toLowerCase() !== host.toLowerCase()) {
      return `${label} (${host})`;
    }
    return label || host || "";
  });
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
      <!-- The link-up card: everything needed to connect this ask to a real
           conversation — how to reach us, and the two identifiers we'll match
           on our side. Numbers only, no jargon. -->
      <div class="linkup">
        <div class="li">
          <span class="k">Call us</span>
          <span class="v strong">713.309.6361</span>
        </div>
        <div class="li">
          <span class="k">Catch us live</span>
          <button class="link" onclick={() => openTiktok()}>
            tiktok.com/@criticalerrorcomputing
          </button>
        </div>
        <div class="li">
          <span class="k">Your Support Number</span>
          <span class="v strong mono">{store.grouped}</span>
        </div>
        {#if computerName}
          <div class="li">
            <span class="k">Your computer</span>
            <span class="v">{computerName}</span>
          </div>
        {/if}
      </div>
      <p class="sub tiny-note">
        Haven't talked to us yet? Call — or catch the live. Your number and
        your computer's name are how we find the right ask.
      </p>
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
  /* The link-up card — label left, value right, readable at phone-call pace. */
  .linkup {
    width: 100%;
    text-align: left;
    border: 1px solid var(--line);
    border-radius: 0.5rem;
    background: var(--surface);
    padding: 0.8rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .li {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 1rem;
  }
  .k {
    font-size: 0.7rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--ink-faint);
    white-space: nowrap;
  }
  .v {
    font-size: 0.9rem;
    overflow-wrap: anywhere;
  }
  .v.strong {
    font-weight: 700;
  }
  .v.mono {
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.04em;
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-size: 0.85rem;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
    overflow-wrap: anywhere;
    text-align: right;
  }
  .sub.tiny-note {
    margin: 0;
    font-size: 0.75rem;
    color: var(--ink-faint);
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
