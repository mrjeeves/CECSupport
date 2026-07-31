<script lang="ts">
  // The customer's "KVM and Claiming" area — sits under the stats block on the
  // right. A CEC KVM is a NanoKVM-class appliance plugged into this machine; it
  // shows up here (discovered from the node's mesh snapshot) as something the
  // customer can claim, mark as attached to this computer, then keep updated.
  //
  // One item per relevant KVM, each rendering exactly one lifecycle state:
  //   available   → Claim
  //   claimed     → "is it on this computer?"  (Yes / Not this computer)
  //   attached    → KVM info + Wi-Fi + Update + Unclaim
  //   claimed-but-not-here → offer to link it here
  // The store owns the transitions; this file is a dumb view of `store.cecKvms`.
  // The Wi-Fi button opens KvmWifiModal (rendered below while `store.wifiFor`
  // is set), which reads/sets the KVM's own Wi-Fi over that same tunnel.
  import { store } from "../store.svelte";
  import { openKvmStore } from "../tauri";
  import KvmWifiModal from "./KvmWifiModal.svelte";

  const kvms = $derived(store.cecKvms);

  /** Dismiss the Open menu on any click that didn't land inside it, and on
   *  Escape — the ordinary expectations of a dropdown. The menu's own buttons
   *  stop propagation implicitly by closing themselves after they act. */
  function onDocClick(e: MouseEvent): void {
    if (!store.linksFor) return;
    const el = e.target as HTMLElement | null;
    if (el?.closest(".menu-wrap")) return;
    store.closeKvmLinks();
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === "Escape" && store.linksFor) store.closeKvmLinks();
  }

  /** Group the 9-digit support number as "123 456 789" so it can be read out
   *  over the phone without losing your place. */
  function formatSupportId(id: string): string {
    return /^\d{9}$/.test(id) ? `${id.slice(0, 3)} ${id.slice(3, 6)} ${id.slice(6)}` : id;
  }
</script>

<svelte:window onclick={onDocClick} onkeydown={onKeydown} />

<!-- The card stays whether or not a KVM is present. It used to disappear
     entirely with nothing to show, which quietly hid the feature from everyone
     who doesn't already own one — and took the Refresh with it, so the control
     for finding a KVM only existed once one had been found. Refresh now lives
     in the header; the empty state below simply mentions that KVMs are a thing.
     Gently: a sentence, no price, no button. -->
<section class="card kvm" aria-label="KVM and claiming">
  <header class="head">
    <h3 class="title">KVM &amp; Claiming</h3>
  </header>

  {#if kvms.length === 0}
    <p class="empty">
      No KVM connected. One lets us help even when this computer won&#39;t start.
      <button class="link" onclick={() => openKvmStore()}>Get one</button>
    </p>
  {:else}
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
            <!-- Reach the KVM's own web UI. A KVM can sit on two networks at
                 once and either address may be the reachable one, so this
                 offers each it found plus the tunnel, which always works. -->
            <div class="menu-wrap">
              <button
                class="btn"
                disabled={store.busy || !k.hasWeb}
                aria-haspopup="menu"
                aria-expanded={store.linksOpenFor(k.node)}
                title={k.hasWeb ? "Open this KVM's own page" : "This KVM hasn't published a console yet"}
                onclick={() => void store.toggleKvmLinks(k.node)}
              >
                Open
                <svg class="caret" class:open={store.linksOpenFor(k.node)} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="m6 9 6 6 6-6" />
                </svg>
              </button>

              {#if store.linksOpenFor(k.node)}
                <div class="menu" role="menu">
                  {#if store.linksLoading && store.kvmLinks.length === 0}
                    <p class="menu-note">Looking for addresses…</p>
                  {/if}
                  {#each store.kvmLinks as link (link.kind + link.host)}
                    <button class="menu-item" role="menuitem" onclick={() => void store.openKvmLink(link)}>
                      <span class="ico" aria-hidden="true">
                        {#if link.kind === "wired"}
                          <!-- ethernet port -->
                          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <rect x="3" y="8" width="18" height="12" rx="2" /><path d="M8 8V5h8v3M8 20v-4M12 20v-4M16 20v-4" />
                          </svg>
                        {:else if link.kind === "wireless"}
                          <!-- wi-fi waves -->
                          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M5 13a10 10 0 0 1 14 0M8.5 16.5a5 5 0 0 1 7 0" /><circle cx="12" cy="20" r="1" fill="currentColor" />
                          </svg>
                        {:else if link.kind === "usb"}
                          <!-- usb trident: the cable the customer is holding -->
                          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M12 21V5" /><path d="m9 8 3-5 3 5" fill="currentColor" />
                            <path d="M12 15l4-3V9" /><circle cx="16" cy="8" r="1.4" fill="currentColor" />
                            <path d="M12 12l-4-3V7" /><rect x="6.6" y="4.6" width="2.8" height="2.8" rx="0.4" fill="currentColor" />
                          </svg>
                        {:else}
                          <!-- linked nodes -->
                          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <circle cx="12" cy="5" r="2.5" /><circle cx="5" cy="19" r="2.5" /><circle cx="19" cy="19" r="2.5" /><path d="M10.5 7 6.5 16.5M13.5 7l4 9.5" />
                          </svg>
                        {/if}
                      </span>
                      <span class="menu-text">
                        <span class="menu-label">{link.label}</span>
                        <span class="menu-detail">{link.detail}</span>
                      </span>
                    </button>
                  {/each}
                  {#if store.linksError}
                    <p class="menu-note err">{store.linksError}</p>
                  {/if}
                </div>
              {/if}
            </div>

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
              title={k.hasWeb
                ? "Update this KVM's firmware and restart it"
                : "This KVM hasn't published a console yet"}
              onclick={() => void store.updateKvm(k.node)}
            >
              Update
            </button>
            <button
              class="btn danger"
              disabled={store.busy}
              title="Release this KVM. It resets and offers itself for setup again"
              onclick={() => store.promptUnclaim(k.node, k.label)}
            >
              Unclaim
            </button>
          </div>

          <!-- Ask for help. The KVM raises its own hand on the CEC support
               area; its physical button does the same thing, so this reflects
               whichever was used. A technician who answers is authorised for a
               bounded window, and that deadline is shown rather than left as an
               open-ended "someone may be connected".

               Whether access is live is `authorised`, NOT whether a countdown
               could be computed: the KVM has no RTC, and a grant held while its
               clock is still unset is real access the device can't yet put a
               time on. Branching on the countdown would tell the customer
               nobody was connected at the one moment that's least true. -->
          {@const help = store.helpFor(k.node)}
          {#if help?.enabled}
            {@const left = store.helpTimeLeft(k.node)}
            <div class="help" class:up={help.asking} class:granted={help.authorised}>
              <div class="help-state">
                {#if help.authorised}
                  <span class="dot ok" aria-hidden="true"></span>
                  {#if left}
                    <span>A technician has access — <strong>{left}</strong> left</span>
                  {:else}
                    <span>A technician has access</span>
                  {/if}
                {:else if help.asking}
                  <span class="hand" aria-hidden="true">✋</span>
                  <span>Hand up — waiting for a technician</span>
                {:else}
                  <span class="dot" aria-hidden="true"></span>
                  <span>Support number <strong>{formatSupportId(help.supportId)}</strong></span>
                {/if}
              </div>
              <button
                class="btn small"
                class:primary={!help.asking && !help.authorised}
                disabled={store.helpPending(k.node)}
                title={help.asking
                  ? "Take the hand down and leave the queue"
                  : `Raise this KVM's hand. A technician who answers gets ${store.helpWindowLabel(k.node)} of access.`}
                onclick={() => void store.toggleKvmHelp(k.node)}
              >
                {#if store.helpPending(k.node)}
                  Working…
                {:else if help.asking}
                  Cancel
                {:else}
                  Ask for help
                {/if}
              </button>
            </div>
          {/if}
        {:else}
          <!-- Ours, but the customer said it's not on this computer. -->
          <div class="row">
            <span class="dot" aria-hidden="true"></span>
            <div class="who">
              <div class="name">{k.label}</div>
              <div class="sub">Claimed, not linked to this computer.</div>
            </div>
          </div>
          <div class="actions">
            <button class="btn" disabled={store.busy} onclick={() => store.attachKvmHere(k.node)}>
              It's on this computer
            </button>
            <button
              class="btn danger"
              disabled={store.busy}
              title="Release this KVM. It resets and offers itself for setup again"
              onclick={() => store.promptUnclaim(k.node, k.label)}
            >
              Unclaim
            </button>
          </div>
        {/if}
      </div>
    {/each}
  {/if}
</section>

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

  /* The empty state. Reads as information, not a pitch: same muted tone as the
     card's other secondary text, no accent colour, no call-to-action styling. */
  .empty {
    margin: 0;
    padding: 0.15rem 0 0.2rem;
    color: var(--ink-soft);
    font-size: 0.86rem;
    line-height: 1.5;
  }

  /* Inline text, not a button: this is a remark with a way to follow it up, and
     anything with a border or a fill would turn the line into a pitch. */
  .link {
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    color: var(--ink-soft);
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }
  .link:hover,
  .link:focus-visible {
    color: var(--ink);
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

  /* Ask-for-help row — hand-raise state and any live support authorisation. */
  .help {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
    padding: 0.55rem 0.7rem;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--r-md);
  }
  .help.up {
    border-color: var(--accent);
  }
  .help.granted {
    border-color: var(--ok);
  }
  .help-state {
    flex: 1 1 10rem;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.85rem;
    line-height: 1.35;
    color: var(--ink-soft);
  }
  .help-state strong {
    color: var(--ink);
    font-weight: 650;
    white-space: nowrap;
  }
  .hand {
    flex: 0 0 auto;
    font-size: 0.95rem;
    line-height: 1;
  }

  /* "Open" menu — the ways to reach the KVM's own web UI. */
  .menu-wrap {
    position: relative;
    display: inline-flex;
  }
  .menu-wrap :global(.btn) {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }
  /* Points the way the menu will go — up, since that's where it opens — and
     turns back over once it's open, so the chevron always indicates the action
     the click performs rather than the state it's in. */
  .caret {
    width: 0.8rem;
    height: 0.8rem;
    opacity: 0.7;
    transform: rotate(180deg);
    transition: transform 0.15s ease;
  }
  .caret.open {
    transform: rotate(0deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .caret {
      transition: none;
    }
  }

  /* Opens UPWARD. The KVM card sits at the bottom of its column and the button
     is on its last row, so there is nothing below to grow into — anchored
     downward the menu ran off the card and, with a KVM reporting several
     addresses, off the window. Above it has the whole card to overlay. */
  .menu {
    position: absolute;
    bottom: calc(100% + 0.35rem);
    left: 0;
    z-index: 20;
    min-width: 13rem;
    padding: 0.3rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    background: var(--surface);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-md);
    /* Cast the shadow upward too, or the menu reads as sitting behind the row
       it just rose out of. */
    box-shadow: 0 -10px 28px rgb(0 0 0 / 0.18);
  }

  .menu-item {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    width: 100%;
    padding: 0.5rem 0.6rem;
    background: transparent;
    border: 0;
    border-radius: var(--r-sm);
    color: var(--ink);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .menu-item:hover,
  .menu-item:focus-visible {
    background: var(--surface-2);
  }
  .ico {
    flex: 0 0 auto;
    display: inline-flex;
    color: var(--ink-soft);
  }
  .ico :global(svg) {
    width: 1.05rem;
    height: 1.05rem;
  }
  .menu-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    line-height: 1.25;
  }
  .menu-label {
    font-size: 0.9rem;
    font-weight: 600;
  }
  .menu-detail {
    font-size: 0.78rem;
    color: var(--ink-soft);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu-note {
    margin: 0;
    padding: 0.45rem 0.6rem;
    font-size: 0.8rem;
    line-height: 1.4;
    color: var(--ink-soft);
  }
  .menu-note.err {
    color: var(--danger);
  }
</style>
