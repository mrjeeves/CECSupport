<script lang="ts">
  // A small in-app confirmation popup — the calm alternative to the webview's
  // native confirm(), which a customer's WebView may block or style oddly.
  // Driven entirely by store.confirmDialog: any action that wants a
  // "are you sure?" gate calls store.askConfirm({...}) and this renders it,
  // then calls runConfirm / cancelConfirm. Uses the same scrim + card modal as
  // the approval prompt.
  import { store } from "../store.svelte";

  const d = $derived(store.confirmDialog);
</script>

{#if d}
  <div class="scrim">
    <div class="card modal confirm" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
      <h2 id="confirm-title">{d.title}</h2>
      <p class="body">{d.body}</p>
      <div class="actions">
        <button class="btn ghost" disabled={store.busy} onclick={() => store.cancelConfirm()}>
          Cancel
        </button>
        <button
          class="btn {d.danger ? 'danger' : 'primary'}"
          disabled={store.busy}
          onclick={() => void store.runConfirm()}
        >
          {d.confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .confirm {
    width: min(24rem, 100%);
    padding: 1.4rem 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
  }
  h2 {
    margin: 0;
    font-size: 1.15rem;
    line-height: 1.3;
    color: var(--ink);
  }
  .body {
    margin: 0;
    font-size: 0.92rem;
    line-height: 1.5;
    color: var(--ink-soft);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
    margin-top: 0.2rem;
  }
</style>
