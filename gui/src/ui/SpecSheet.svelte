<script lang="ts">
  // The spec card — the machine's headline hardware between the front-door
  // buttons (or the number box) and the access list, so a technician on the
  // phone can ask "what does it say under CPU?" and the customer just reads
  // it. Titled with the computer's name. Data comes straight off the node's
  // local scan (`machine_specs`). Temps show only when the OS exposes
  // sensors — the card never invents numbers. Hidden entirely until the
  // scan lands (or on an older node without the command).
  import { store } from "../store.svelte";

  const specs = $derived(store.specs);
  /** "Name (HOSTNAME)" — the identity pair, spelled exactly like the
   *  technician's card so matching up is a read, not a guess. */
  const title = $derived(store.computerName || "This computer");

  /** "16 GB" / "512 GB" / "1.8 TB" — spec-sheet sizes, one decimal max. */
  function gb(bytes: number | null | undefined): string {
    if (!bytes || bytes <= 0) return "—";
    const g = bytes / 1024 ** 3;
    if (g >= 1024) return `${(g / 1024).toFixed(1)} TB`;
    return `${g >= 100 ? Math.round(g) : Math.round(g * 10) / 10} GB`;
  }

  /** The CPU line: brand plus cores when known. */
  const cpuLine = $derived.by(() => {
    const c = specs?.cpu;
    if (!c) return "";
    const cores = c.cores ? ` · ${c.cores} cores` : "";
    return `${c.brand}${cores}`;
  });

  /** RAM as "in use / total" — used is what a support call cares about. */
  const ramLine = $derived.by(() => {
    const m = specs?.memory;
    if (!m) return "";
    const used = Math.max(0, m.total_bytes - m.available_bytes);
    return `${gb(used)} used / ${gb(m.total_bytes)}`;
  });

  /** Fixed disks first; removables tag themselves. */
  const disks = $derived.by(() =>
    [...(specs?.disks ?? [])].sort((a, b) => Number(a.removable) - Number(b.removable)),
  );

  /** Sensor labels as the OS names them are driver-speak
   *  ("ACPI\ThermalZone\TZ00_0", "coretemp Package id 0"); shorten the noisy
   *  prefixes but never rename — a technician can still match what the
   *  customer reads to a vendor tool. */
  function tempLabel(label: string): string {
    const tail = label.split("\\").pop() ?? label;
    return tail.replace(/^coretemp\s+/i, "").trim() || label;
  }
  const temps = $derived(specs?.temps ?? []);
</script>

{#if !specs && store.specsPending}
  <!-- The scan takes a few seconds after boot; hold the card's spot with a
       quiet reading state instead of an empty rail. Resolves either into the
       real card or (older node, no command) into nothing. -->
  <section class="card sheet pending" role="status" aria-label="Reading this computer's specs">
    <span class="spin" aria-hidden="true"></span>
    <p class="pending-note">Reading this computer's specs…</p>
  </section>
{/if}
{#if specs}
  <section class="card sheet" aria-label="Computer specifications">
    <h3 class="title">{title}</h3>
    {#if store.grouped}
      <p class="idnum">CEC Support {store.grouped}</p>
    {/if}
    <div class="grid">
      <span class="k">CPU</span>
      <span class="v">{cpuLine}</span>
      <span class="k">RAM</span>
      <span class="v">{ramLine}</span>
      {#each specs.gpus as g, i (g.name + i)}
        <span class="k">{specs.gpus.length > 1 ? `GPU ${i + 1}` : "GPU"}</span>
        <span class="v">{g.name}{g.vram_bytes ? ` · ${gb(g.vram_bytes)}` : ""}</span>
      {/each}
    </div>
    {#if disks.length > 0}
      <div class="disks">
        {#each disks as d (d.name + (d.mount ?? ""))}
          {@const used = Math.max(0, d.total_bytes - d.available_bytes)}
          <div class="disk">
            <span class="disk-name" title={d.name}>
              {d.mount || d.name}{d.removable ? " (removable)" : ""}
            </span>
            <span class="disk-size">{gb(used)} / {gb(d.total_bytes)}</span>
            <span class="bar" aria-hidden="true">
              <span
                class="fill"
                style={`width: ${d.total_bytes > 0 ? Math.min(100, Math.round((used / d.total_bytes) * 100)) : 0}%`}
              ></span>
            </span>
          </div>
        {/each}
      </div>
    {/if}
    {#if temps.length > 0}
      <div class="temps" aria-label="Temperatures">
        {#each temps as t (t.label)}
          <span class="temp" title={t.label}>
            <span class="temp-label">{tempLabel(t.label)}</span>
            <span class="temp-val">{Math.round(t.celsius)}°C</span>
          </span>
        {/each}
      </div>
    {/if}
    <p class="os">{specs.os}</p>
  </section>
{/if}

<style>
  .sheet {
    width: 100%;
    max-width: 30rem;
    padding: 1.1rem 1.3rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .sheet.pending {
    align-items: center;
    justify-content: center;
    min-height: 9rem;
    gap: 0.8rem;
  }
  .spin {
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 50%;
    border: 3px solid var(--line);
    border-top-color: var(--accent);
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation: none;
    }
  }
  .pending-note {
    margin: 0;
    font-size: 0.8rem;
    color: var(--ink-faint);
  }
  .title {
    margin: 0;
    font-size: 1rem;
    font-weight: 700;
  }
  /* The Support Number, right under the machine name — the two things we
     match on, together at the top of the computer's own card. */
  .idnum {
    margin: -0.45rem 0 0;
    font-size: 0.75rem;
    color: var(--ink-faint);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.03em;
  }
  .grid {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 0.9rem;
    row-gap: 0.35rem;
    align-items: baseline;
  }
  .k {
    font-size: 0.7rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--ink-faint);
  }
  .v {
    font-size: 0.88rem;
    color: var(--ink);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .disks {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    border-top: 1px solid var(--line);
    padding-top: 0.6rem;
  }
  .disk {
    display: grid;
    grid-template-columns: 1fr auto;
    row-gap: 0.2rem;
    align-items: baseline;
  }
  .disk-name {
    font-size: 0.82rem;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .disk-size {
    font-size: 0.78rem;
    color: var(--ink-soft);
    font-variant-numeric: tabular-nums;
  }
  .bar {
    grid-column: 1 / -1;
    height: 0.35rem;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--line);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--accent);
    border-radius: inherit;
  }

  .temps {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 0.5rem;
    border-top: 1px solid var(--line);
    padding-top: 0.6rem;
  }
  .temp {
    display: inline-flex;
    align-items: baseline;
    gap: 0.35rem;
    padding: 0.15rem 0.55rem;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: var(--surface);
  }
  .temp-label {
    font-size: 0.72rem;
    color: var(--ink-soft);
    max-width: 11rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .temp-val {
    font-size: 0.78rem;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .os {
    margin: 0;
    font-size: 0.72rem;
    color: var(--ink-faint);
  }
</style>
