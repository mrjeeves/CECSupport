<script lang="ts">
  // CEC's contact points — how an ask becomes a conversation. Lives at the
  // top of the right-hand rail, always: the space is there, and the phone
  // number is useful before, during, and after an ask. The identity pair we
  // match on (Support Number + computer name) heads the spec card beneath,
  // so this card is purely "how to reach us".
  import { openTiktok, openAllmystuffWorks } from "../tauri";
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
  <!-- One short line each; the fuller explanation rides the tooltip. -->
  <p
    class="note"
    title="We'll match you by the name and Support Number shown on your computer's card."
  >
    New to CEC? Call us or hop on the live.
  </p>

  <div class="promo">
    <button
      class="promo-link"
      title="CEC Support runs on the AllMyStuff system to connect and show your screen. AllMyStuff.works gives you that same access to your own machines, anywhere."
      onclick={() => openAllmystuffWorks()}
    >
      Powered by <strong>AllMyStuff</strong>. Get it at AllMyStuff.works →
    </button>
  </div>
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

  .promo {
    margin-top: 0.3rem;
    padding-top: 0.7rem;
    border-top: 1px solid var(--line, rgba(128, 128, 128, 0.25));
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .promo-link strong {
    color: var(--ink, inherit);
    font-weight: 700;
  }
  .promo-link {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-size: 0.82rem;
    font-weight: 600;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
    text-align: left;
  }
</style>
