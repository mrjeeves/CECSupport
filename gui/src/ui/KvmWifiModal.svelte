<script lang="ts">
  // The Wi-Fi panel for a claimed KVM. Reads the appliance's own Wi-Fi state
  // and sets it — SSID + password, or a tap on a scanned network — over the
  // same mesh tunnel the Reboot uses (see store `connectKvmWifi`). The store
  // owns every backend call and all the transient state; this is a dumb view.
  //
  // Rendered by KvmClaimCard only while `store.wifiFor` is set, so mounting
  // clears the local SSID/password fields for each fresh open.
  import { store } from "../store.svelte";
  import type { KvmWifiNetwork } from "../types";

  let { node }: { node: string } = $props();

  let ssid = $state("");
  let password = $state("");

  const status = $derived(store.wifiStatus);
  // Always an array so the template never has to narrow a nullable.
  const nets = $derived(store.wifiScan ?? []);

  function isSecured(net: KvmWifiNetwork): boolean {
    return !!net.security && net.security.toLowerCase() !== "open";
  }

  /** 1–3 bars from a dBm reading (unknown → middle). */
  function signalStrength(signal?: number): number {
    if (typeof signal !== "number") return 2;
    if (signal >= -60) return 3;
    if (signal >= -72) return 2;
    return 1;
  }

  function pick(net: KvmWifiNetwork): void {
    ssid = net.ssid;
    // Clear any half-typed password; an open network needs none.
    password = "";
  }

  function connect(): void {
    void store.connectKvmWifi(node, ssid, password);
  }
</script>

<div class="scrim">
  <div class="card modal wifi" role="dialog" aria-modal="true" aria-labelledby="wifi-title">
    <header class="whead">
      <h2 id="wifi-title">Wi-Fi · {store.wifiKvmLabel}</h2>
      <button class="btn ghost small" onclick={() => store.closeKvmWifi()}>Close</button>
    </header>

    {#if store.wifiLoading && !status}
      <p class="muted">Checking the KVM's Wi-Fi…</p>
    {:else if !status}
      <p class="err">{store.wifiError ?? "Couldn't read the KVM's Wi-Fi settings."}</p>
      <button class="btn" disabled={store.wifiLoading} onclick={() => void store.loadKvmWifi(node)}>
        Try again
      </button>
    {:else if !status.supported}
      <p class="muted">This KVM doesn't have Wi-Fi.</p>
    {:else}
      <!-- Current state -->
      <div class="state">
        {#if status.connected}
          <span class="dot ok" aria-hidden="true"></span>
          <span>Connected to <strong>{status.ssid ?? "Wi-Fi"}</strong></span>
        {:else if status.apMode}
          <span class="dot" aria-hidden="true"></span>
          <span>In setup (hotspot) mode — not on a network yet</span>
        {:else}
          <span class="dot" aria-hidden="true"></span>
          <span>Not connected to Wi-Fi</span>
        {/if}
      </div>

      <!-- Scanned networks (NanoKVM-Pro only; hidden on a plain NanoKVM) -->
      {#if nets.length > 0}
        <div class="pick">
          <div class="pick-head">
            <span class="lbl">Networks nearby</span>
            <button
              class="btn ghost small"
              disabled={store.wifiScanning || store.wifiBusy}
              onclick={() => void store.scanKvmWifi(node)}
            >
              {store.wifiScanning ? "Scanning…" : "Rescan"}
            </button>
          </div>
          <ul class="nets">
            {#each nets as net (net.bssid ?? net.ssid)}
              {@const strength = signalStrength(net.signal)}
              <li>
                <button
                  class="net"
                  class:sel={ssid === net.ssid}
                  disabled={store.wifiBusy}
                  onclick={() => pick(net)}
                >
                  <span class="net-name">{net.ssid}</span>
                  <span class="net-meta">
                    {#if isSecured(net)}
                      <svg class="lock" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <rect x="3" y="11" width="18" height="11" rx="2" /><path d="M7 11V7a5 5 0 0 1 10 0v4" />
                      </svg>
                    {/if}
                    <span class="bars" data-strength={strength} aria-hidden="true">
                      <i></i><i></i><i></i>
                    </span>
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      <!-- Enter / confirm the network -->
      <div class="form">
        <label class="fld">
          <span class="lbl">Network name (SSID)</span>
          <input
            class="input"
            type="text"
            bind:value={ssid}
            placeholder="Wi-Fi name"
            autocomplete="off"
            autocapitalize="none"
            spellcheck="false"
            disabled={store.wifiBusy}
          />
        </label>
        <label class="fld">
          <span class="lbl">Password</span>
          <input
            class="input"
            type="password"
            bind:value={password}
            placeholder="Leave blank for an open network"
            autocomplete="off"
            disabled={store.wifiBusy}
          />
        </label>

        {#if store.wifiError}
          <p class="err">{store.wifiError}</p>
        {/if}

        <button
          class="btn primary big"
          disabled={store.wifiBusy || !ssid.trim()}
          onclick={connect}
        >
          {store.wifiBusy ? "Connecting…" : "Connect"}
        </button>

        {#if status.connected}
          <button
            class="btn danger"
            disabled={store.wifiBusy}
            onclick={() => store.promptDisconnectKvmWifi(node, status?.ssid ?? null)}
          >
            Disconnect
          </button>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .wifi {
    width: min(26rem, 100%);
    padding: 1.35rem 1.45rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    max-height: min(90vh, 44rem);
    overflow-y: auto;
  }

  .whead {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .whead h2 {
    flex: 1 1 auto;
    margin: 0;
    font-size: 1.15rem;
    line-height: 1.3;
    color: var(--ink);
    overflow-wrap: anywhere;
  }

  .muted {
    margin: 0;
    color: var(--ink-soft);
    font-size: 0.95rem;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 0.88rem;
    line-height: 1.45;
  }

  .state {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    padding: 0.7rem 0.85rem;
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    font-size: 0.92rem;
    color: var(--ink-soft);
  }
  .state strong {
    color: var(--ink);
    font-weight: 650;
    overflow-wrap: anywhere;
  }
  .dot {
    flex: 0 0 auto;
    width: 0.65rem;
    height: 0.65rem;
    border-radius: 50%;
    background: var(--ink-faint);
  }
  .dot.ok {
    background: var(--ok);
    box-shadow: 0 0 0 3px var(--ok-soft);
  }

  .pick {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }
  .pick-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
  }
  .lbl {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--ink-soft);
  }
  .nets {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    max-height: 12rem;
    overflow-y: auto;
  }
  .net {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
    padding: 0.55rem 0.7rem;
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    color: var(--ink);
    font-size: 0.92rem;
    text-align: left;
  }
  .net:hover:not(:disabled) {
    border-color: var(--line-strong);
    background: var(--surface);
  }
  .net.sel {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .net-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .net-meta {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    color: var(--ink-soft);
  }
  .lock {
    width: 0.85rem;
    height: 0.85rem;
  }
  .bars {
    display: inline-flex;
    align-items: flex-end;
    gap: 2px;
    height: 0.85rem;
  }
  .bars i {
    width: 3px;
    border-radius: 1px;
    background: var(--ink-faint);
  }
  .bars i:nth-child(1) {
    height: 40%;
  }
  .bars i:nth-child(2) {
    height: 70%;
  }
  .bars i:nth-child(3) {
    height: 100%;
  }
  .bars[data-strength="1"] i:nth-child(-n + 1),
  .bars[data-strength="2"] i:nth-child(-n + 2),
  .bars[data-strength="3"] i:nth-child(-n + 3) {
    background: var(--ink);
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .input {
    font-family: inherit;
    font-size: 1rem;
    padding: 0.6rem 0.8rem;
    border-radius: var(--r-md);
    border: 1px solid var(--line-strong);
    background: var(--surface);
    color: var(--ink);
  }
  .input:focus {
    outline: 2px solid var(--accent-soft);
    border-color: var(--accent);
  }
  .form .btn.big {
    margin-top: 0.15rem;
  }
</style>
