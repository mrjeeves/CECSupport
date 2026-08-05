<script lang="ts">
  import { onMount } from "svelte";
  import { driveMap, driveScan, driveUnmap, sessionSnapshot } from "../tauri";
  import type { DriveCapability, DriveScan, SessionRoute, SessionSnapshot } from "../types";
  import DriveBrowser from "./DriveBrowser.svelte";

  let local = $state<DriveScan | null>(null);
  let session = $state<SessionSnapshot | null>(null);
  let targetFor = $state<Record<string, string>>({});
  let openRoute = $state<{ route: string; label: string; source: string } | null>(null);
  let busy = $state(false);
  let note = $state("");

  const drives = $derived(
    (local?.capabilities ?? []).filter((c) => c.media === "storage" && c.origin === "storage"),
  );
  const targets = $derived.by(() =>
    (session?.peers ?? [])
      .map((peer) => ({
        peer,
        sink: peer.capabilities?.find((c) => c.media === "storage" && c.origin === "storage-in"),
      }))
      .filter((x): x is { peer: typeof x.peer; sink: DriveCapability } => !!x.sink),
  );
  const mappings = $derived(
    (session?.routes ?? []).filter(
      (r) => r.route.media === "storage" && r.route.to.endsWith(":storage-in") && r.state.state !== "torn_down",
    ),
  );

  function nodeOf(capability: string): string { return capability.split(":", 1)[0]; }
  function peerName(node: string): string {
    const peer = session?.peers?.find((p) => p.node === node);
    return peer?.label || peer?.hostname || "support machine";
  }
  function driveName(route: SessionRoute): string {
    const caps = nodeOf(route.route.from) === local?.node_id
      ? local?.capabilities
      : session?.peers?.find((p) => p.node === nodeOf(route.route.from))?.capabilities;
    return caps?.find((c) => c.id === route.route.from)?.label ?? "Mapped drive";
  }
  async function refresh() {
    const [scan, snap] = await Promise.all([driveScan(), sessionSnapshot()]);
    if (scan) local = scan;
    if (snap) session = snap;
  }
  async function map(capability: DriveCapability) {
    const targetNode = targetFor[capability.id] ?? targets[0]?.peer.node;
    const target = targets.find((x) => x.peer.node === targetNode);
    if (!target) return;
    busy = true;
    note = "";
    try {
      await driveMap(capability.id, target.sink.id);
      await refresh();
    } catch (e) {
      note = String(e);
    } finally {
      busy = false;
    }
  }
  async function unmap(route: string) {
    busy = true;
    try {
      await driveUnmap(route);
      if (openRoute?.route === route) openRoute = null;
      await refresh();
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 1500);
    return () => clearInterval(timer);
  });
</script>

<section class="card drives">
  <div class="heading">
    <div><h2>Drive mapping</h2><p>Use a plugged-in drive on the other support machine.</p></div>
    <span>KVM-free</span>
  </div>

  {#if mappings.length > 0}
    <div class="mapped">
      {#each mappings as mapping (mapping.route.id)}
        {@const incoming = nodeOf(mapping.route.to) === local?.node_id}
        <div class="mapping">
          <div class="meta">
            <b>{driveName(mapping)}</b>
            <small>{incoming ? `From ${peerName(nodeOf(mapping.route.from))}` : `Mapped to ${peerName(nodeOf(mapping.route.to))}`}</small>
          </div>
          {#if incoming}
            <button class="btn primary" onclick={() => (openRoute = { route: mapping.route.id, label: driveName(mapping), source: peerName(nodeOf(mapping.route.from)) })}>Open</button>
          {/if}
          <button class="btn ghost" disabled={busy} onclick={() => void unmap(mapping.route.id)}>Unmap</button>
        </div>
      {/each}
    </div>
  {/if}

  {#if drives.length > 0 && targets.length > 0}
    <details>
      <summary>Map a drive from this computer</summary>
      <div class="local-list">
        {#each drives as drive (drive.id)}
          <div class="local-row">
            <b>{drive.label}</b>
            <select bind:value={targetFor[drive.id]} aria-label={`Map ${drive.label} to`}>
              {#each targets as target (target.peer.node)}
                <option value={target.peer.node}>{target.peer.label || target.peer.hostname || "Support machine"}</option>
              {/each}
            </select>
            <button class="btn" disabled={busy} onclick={() => void map(drive)}>Map</button>
          </div>
        {/each}
      </div>
    </details>
  {:else if mappings.length === 0}
    <p class="empty">Connect to a support machine to map a drive in either direction.</p>
  {/if}
  {#if note}<p class="error">{note}</p>{/if}
</section>

{#if openRoute}
  <DriveBrowser route={openRoute.route} label={openRoute.label} source={openRoute.source} close={() => (openRoute = null)} />
{/if}

<style>
  .drives { width: 100%; max-width: 30rem; padding: 1.1rem; display: flex; flex-direction: column; gap: .8rem; }
  .heading { display: flex; justify-content: space-between; gap: .8rem; align-items: flex-start; }
  h2, p { margin: 0; } h2 { font-family: var(--font-display); font-size: 1.05rem; }
  .heading p, .empty, small { color: var(--ink-soft); font-size: .76rem; margin-top: .2rem; }
  .heading > span { font-size: .65rem; font-weight: 800; color: var(--ok); border: 1px solid var(--ok); border-radius: 999px; padding: .18rem .38rem; }
  .mapped, .local-list { display: flex; flex-direction: column; gap: .45rem; }
  .mapping, .local-row { display: flex; align-items: center; gap: .45rem; padding: .55rem; border: 1px solid var(--line); border-radius: .6rem; }
  .meta { min-width: 0; flex: 1; display: flex; flex-direction: column; } .meta b { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  details { border-top: 1px solid var(--line); padding-top: .65rem; } summary { cursor: pointer; font-size: .78rem; font-weight: 700; }
  .local-list { margin-top: .55rem; } .local-row b { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  select { min-width: 7rem; max-width: 10rem; border: 1px solid var(--line-strong); border-radius: .45rem; background: var(--surface); color: var(--ink); padding: .35rem; }
  .empty { text-align: center; padding: .7rem; } .error { color: var(--danger); font-size: .76rem; }
</style>
