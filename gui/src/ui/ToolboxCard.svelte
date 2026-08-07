<script lang="ts">
  import { openToolbox } from "../tauri";

  let opening = $state(false);

  async function open(): Promise<void> {
    if (opening) return;
    opening = true;
    try {
      await openToolbox();
    } finally {
      opening = false;
    }
  }
</script>

<section class="card toolbox" aria-labelledby="toolbox-card-title">
  <div class="copy">
    <span class="mark" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M14.7 6.3a4 4 0 0 0-5 5L3 18l3 3 6.7-6.7a4 4 0 0 0 5-5l-2.4 2.4-3-3 2.4-2.4z" />
      </svg>
    </span>
    <span>
      <h3 id="toolbox-card-title">Toolbox</h3>
      <p>Safe Windows checks and repair tools, gathered in one place.</p>
    </span>
  </div>
  <button class="btn toolbox-button" disabled={opening} onclick={() => void open()}>
    {opening ? "Opening…" : "Open Toolbox"}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M5 12h14" /><path d="m13 6 6 6-6 6" />
    </svg>
  </button>
</section>

<style>
  .toolbox {
    width: 100%;
    max-width: 30rem;
    padding: 0.9rem 1rem;
    display: grid;
    gap: 0.85rem;
  }
  .copy {
    display: flex;
    gap: 0.75rem;
    align-items: flex-start;
  }
  .mark {
    width: 2.25rem;
    height: 2.25rem;
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    border-radius: 0.75rem;
    color: var(--accent-ink);
    background: var(--accent-soft);
    border: 1px solid color-mix(in oklch, var(--accent) 34%, var(--line));
  }
  .mark svg {
    width: 1.25rem;
    height: 1.25rem;
  }
  h3 {
    margin: 0 0 0.18rem;
    font-size: 1rem;
  }
  p {
    margin: 0;
    color: var(--ink-soft);
    font-size: 0.8rem;
    line-height: 1.45;
  }
  .toolbox-button {
    width: 100%;
    justify-content: space-between;
    background: var(--surface-2);
  }
  .toolbox-button svg {
    width: 1.05rem;
    height: 1.05rem;
  }
</style>
