<script lang="ts">
  // The customer's "KVM and Claiming" area — sits under the stats block on the
  // right. A CEC KVM is a NanoKVM-class appliance plugged into this machine; it
  // shows up here (discovered from the node's mesh snapshot) as something the
  // customer can claim, mark as attached to this computer, then reboot.
  //
  // One item per relevant KVM, each rendering exactly one lifecycle state:
  //   available   → Claim
  //   claimed     → "is it on this computer?"  (Yes / Not this computer)
  //   attached    → KVM info + Wi-Fi + Reboot + Unclaim
  //   claimed-but-not-here → offer to link it here
  // The store owns the transitions; this file is a dumb view of `store.cecKvms`.
  // The Wi-Fi button opens KvmWifiModal (rendered below while `store.wifiFor`
  // is set), which reads/sets the KVM's own Wi-Fi over the reboot tunnel.
  import { store } from "../store.svelte";
  import KvmWifiModal from "./KvmWifiModal.svelte";

  const kvms = $derived(store.cecKvms);
</script>

{#if kvms.length > 0}
  <section class="card kvm" aria-label="KVM and claiming">
    <header class="head">
      <h3 class="title">KVM &amp; Claiming</h3>
      <button
        class="refresh"
        title="Look again for KVMs"
        aria-label="Refresh"
        onclick={() => store.refreshKvms()}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M21 12a9 9 0 1 1-2.64-6.36" /><path d="M21 3v6h-6" />
        </svg>
      </button>
    </header>

    {#each kvms as k (k.node)}
      <div class="item">
        {#if !k.mine && k.claimable}
          <!-- Available to set up. -->
          <div class="row">
            <span class="dot" aria-hidden="true"></span>
            <div class="who">
              <div class="name">{k.label}</div>
              <div class="sub">A KVM is ready to set up.</div>
            </div>
          </div>
          <div class="actions">
            <button class="btn primary" disabled={store.busy} onclick={() => store.claimKvm(k.node)}>
              Claim this KVM
            </button>
          </div>
        {:else if k.promptAttach}
          <!-- Just claimed — ask where it lives. -->
          <div class="row">
            <span class="dot ok" aria-hidden="true"></span>
            <div class="who">
              <div class="name">{k.label}</div>
              <div class="sub">Claimed. Is this KVM plugged into <strong>this computer</strong>?</div>
            </div>
          </div>
          <div class="actions">
            <button class="btn primary" disabled={store.busy} onclick={() => store.attachKvmHere(k.node)}>
              Yes, it's on this computer
            </button>
            <button class="btn ghost" disabled={store.busy} onclick={() => store.dismissAttachPrompt(k.node)}>
              Not this computer
            </button>
          </div>
        {:else if k.mine && k.attachedHere}
          <!-- Set up and controlling this machine. -->
          <div class="row">
            <span class="dot ok" aria-hidden="true"></span>
            <div class="who">
              <div class="name">{k.label}</div>
              <div class="sub">Linked to this computer</div>
            </div>
          </div>
          <div class="actions">
            <button
              class="btn"
              disabled={store.busy || !k.hasWeb}
              title={k.hasWeb ? "Set up this KVM's Wi-Fi" : "This KVM hasn't published a console yet"}
              onclick={() => void store.openKvmWifi(k.node)}
            >
              Wi-Fi
            </button>
            <button
              class="btn"
              disabled={store.busy || !k.hasWeb}
              title={k.hasWeb ? "Reboot the machine this KVM controls" : "This KVM hasn't published a console yet"}
              onclick={() => store.rebootKvm(k.node)}
            >
              Reboot
            </button>
            <button
              class="btn danger"
              disabled={store.busy}
              title="Release this KVM — resets it and offers it for setup again"
              onclick={() => store.promptUnclaim(k.node, k.label)}
            >
              Unclaim
            </button>
          </div>
        {:else}
          <!-- Ours, but the customer said it's not on this computer. -->
          <div class="row">
            <span class="dot" aria-hidden="true"></span>
            <div class="who">
              <div class="name">{k.label}</div>
              <div class="sub">Claimed — not linked to this computer.</div>
            </div>
          </div>
          <div class="actions">
            <button class="btn" disabled={store.busy} onclick={() => store.attachKvmHere(k.node)}>
              It's on this computer
            </button>
            <button
              class="btn danger"
              disabled={store.busy}
              title="Release this KVM — resets it and offers it for setup again"
              onclick={() => store.promptUnclaim(k.node, k.label)}
            >
              Unclaim
            </button>
          </div>
        {/if}
      </div>
    {/each}
  </section>
{/if}

<!-- The Wi-Fi panel is a full-screen overlay, so it lives outside the card and
     renders only while a KVM's Wi-Fi is open. Conditionally mounted so its
     SSID/password fields start empty each time it opens. -->
{#if store.wifiFor}
  <KvmWifiModal node={store.wifiFor} />
{/if}

<style>
  .kvm {
    width: 100%;
    max-width: 30rem;
    padding: 1.1rem 1.3rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .title {
    flex: 1 1 auto;
    margin: 0;
    font-weight: 700;
    font-size: 1rem;
    line-height: 1.2;
    color: var(--ink);
  }
  .refresh {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.9rem;
    height: 1.9rem;
    padding: 0;
    color: var(--ink-soft);
    background: transparent;
    border: 1px solid var(--line);
    border-radius: var(--r-pill);
    cursor: pointer;
  }
  .refresh:hover {
    color: var(--ink);
    border-color: var(--line-strong);
  }
  .refresh svg {
    width: 1rem;
    height: 1rem;
  }

  .item {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
    padding: 0.8rem 0.85rem;
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r-md);
  }

  .row {
    display: flex;
    align-items: flex-start;
    gap: 0.6rem;
  }
  .dot {
    flex: 0 0 auto;
    width: 0.7rem;
    height: 0.7rem;
    margin-top: 0.25rem;
    border-radius: 50%;
    background: var(--ink-faint);
  }
  .dot.ok {
    background: var(--ok);
    box-shadow: 0 0 0 3px var(--ok-soft);
  }
  .who {
    flex: 1 1 auto;
    min-width: 0;
  }
  .name {
    font-weight: 700;
    font-size: 0.95rem;
    line-height: 1.2;
    color: var(--ink);
    overflow-wrap: anywhere;
  }
  .sub {
    margin-top: 0.15rem;
    font-size: 0.82rem;
    line-height: 1.4;
    color: var(--ink-soft);
  }
  .sub strong {
    color: var(--ink);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
</style>
