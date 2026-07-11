<script lang="ts">
  // CEC's contact points — how an ask becomes a conversation. Appears the
  // moment the customer asks for help and stays up for the rest of the run
  // (help arriving withdraws the ask, not the phone number). The identity
  // pair we match on (Support Number + computer name) lives in the app's
  // permanent bottom bar, so this card is purely "how to reach us".
  import { openTiktok } from "../tauri";
</script>

<section class="card contact" aria-label="How to reach CEC">
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
  <p class="note">
    Haven't talked to us yet? Call, or catch the live — we'll match you by
    the number and computer name at the bottom of this window.
  </p>
</section>

<style>
  .contact {
    width: 100%;
    max-width: 30rem;
    text-align: left;
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
  .v.strong {
    font-size: 0.95rem;
    font-weight: 700;
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
  .note {
    margin: 0;
    font-size: 0.75rem;
    color: var(--ink-faint);
    line-height: 1.45;
  }
</style>
