<script lang="ts">
  import { onMount } from "svelte";
  import { fileDownload, fileSend, watchFiles } from "../tauri";
  import type { FileEntry, FileEvent } from "../types";

  let {
    route,
    label,
    source,
    close,
  }: { route: string; label: string; source: string; close: () => void } = $props();

  let path = $state("~");
  let entries = $state<FileEntry[]>([]);
  let loading = $state(true);
  let message = $state("");
  let nextReq = 1;
  let listReq = 0;
  let uploadInput = $state<HTMLInputElement | null>(null);
  let stopWatch: (() => void) | null = null;
  let disposed = false;

  const sorted = $derived(
    [...entries].sort((a, b) => Number(b.dir) - Number(a.dir) || a.name.localeCompare(b.name)),
  );

  function child(name: string): string {
    return path === "/" ? `/${name}` : `${path}/${name}`;
  }
  function parent(): string {
    if (path === "/" || path === "~") return "/";
    const up = path.slice(0, path.lastIndexOf("/"));
    return up || "/";
  }
  function send(event: FileEvent) {
    void fileSend(route, event).catch((e) => (message = String(e)));
  }
  function list(next = path) {
    loading = true;
    const req = nextReq++;
    listReq = req;
    send({ kind: "list", req, path: next });
  }
  function onEvent(event: FileEvent) {
    if (event.kind === "entries" && event.req === listReq) {
      path = event.path;
      entries = event.entries;
      loading = false;
      message = "";
    } else if (event.kind === "err") {
      loading = false;
      message = event.reason;
    } else if (event.kind === "ok") {
      message = "Saved";
      list(path);
    }
  }
  function open(entry: FileEntry) {
    if (entry.dir) list(child(entry.name));
    else void download(entry);
  }
  async function download(entry: FileEntry) {
    const req = nextReq++;
    try {
      const destination = await fileDownload(route, req, entry.name);
      send({ kind: "read", req, path: child(entry.name) });
      message = `Downloading to ${destination}`;
    } catch (e) {
      message = String(e);
    }
  }
  function b64(bytes: Uint8Array): string {
    let raw = "";
    for (let i = 0; i < bytes.length; i++) raw += String.fromCharCode(bytes[i]);
    return btoa(raw);
  }
  async function upload(files: FileList | null) {
    if (!files) return;
    for (const file of Array.from(files)) {
      const req = nextReq++;
      const bytes = new Uint8Array(await file.arrayBuffer());
      const chunk = 40 * 1024;
      if (bytes.length === 0) {
        await fileSend(route, { kind: "write", req, path: child(file.name), data: "", append: false, eof: true });
      }
      for (let offset = 0; offset < bytes.length; offset += chunk) {
        await fileSend(route, {
          kind: "write",
          req,
          path: child(file.name),
          data: b64(bytes.subarray(offset, Math.min(offset + chunk, bytes.length))),
          append: offset > 0,
          eof: offset + chunk >= bytes.length,
        });
      }
    }
    if (uploadInput) uploadInput.value = "";
  }
  function makeFolder() {
    const name = window.prompt("Folder name")?.trim();
    if (!name || name.includes("/") || name.includes("\\")) return;
    send({ kind: "mkdir", req: nextReq++, path: child(name) });
  }
  function remove(entry: FileEntry) {
    if (!window.confirm(`Delete ${entry.name}?`)) return;
    send({ kind: "delete", req: nextReq++, path: child(entry.name) });
  }

  onMount(() => {
    void watchFiles(route, onEvent).then((stop) => {
      if (disposed) {
        stop();
        return;
      }
      stopWatch = stop;
      list("~");
    });
    return () => {
      disposed = true;
      stopWatch?.();
    };
  });
</script>

<div class="scrim" role="presentation">
  <button class="backdrop" aria-label="Close mapped drive" onclick={close}></button>
  <div class="browser" role="dialog" aria-modal="true" aria-label={`Mapped drive ${label}`}>
    <header>
      <div>
        <h2>{label}</h2>
        <p>Mapped from {source}</p>
      </div>
      <button class="x" aria-label="Close" onclick={close}>×</button>
    </header>
    <div class="toolbar">
      <button class="btn" disabled={path === "/"} onclick={() => list(parent())}>↑ Up</button>
      <button class="btn" onclick={() => list("/")}>⌂ Drive</button>
      <span class="path">{path}</span>
      <button class="btn" onclick={() => list(path)}>↻</button>
      <button class="btn" onclick={makeFolder}>New folder</button>
      <button class="btn primary" onclick={() => uploadInput?.click()}>Upload</button>
      <input bind:this={uploadInput} class="hidden" type="file" multiple onchange={(e) => void upload(e.currentTarget.files)} />
    </div>
    {#if message}<div class="message">{message}</div>{/if}
    <div class="list">
      {#if loading}
        <p class="empty">Loading…</p>
      {:else if sorted.length === 0}
        <p class="empty">This folder is empty.</p>
      {:else}
        {#each sorted as entry (entry.name)}
          <div class="entry">
            <button class="open" onclick={() => open(entry)}>
              <span>{entry.dir ? "📁" : "📄"}</span>
              <b>{entry.name}</b>
              <small>{entry.dir ? "Folder" : `${Math.max(1, Math.round(entry.size / 1024))} KB`}</small>
            </button>
            <button class="delete" aria-label={`Delete ${entry.name}`} onclick={() => remove(entry)}>Delete</button>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 80; display: grid; place-items: center; padding: 1rem; }
  .backdrop { position: absolute; inset: 0; border: 0; background: rgba(7, 8, 16, 0.72); }
  .browser { position: relative; width: min(58rem, 96vw); height: min(40rem, 90vh); display: flex; flex-direction: column; background: var(--surface); border: 1px solid var(--line-strong); border-radius: 1rem; overflow: hidden; box-shadow: 0 1.5rem 5rem rgba(0,0,0,.45); }
  header { display: flex; justify-content: space-between; align-items: center; padding: 1rem 1.15rem; border-bottom: 1px solid var(--line); }
  h2, header p { margin: 0; } header p { color: var(--ink-soft); font-size: .78rem; margin-top: .15rem; }
  .x { border: 0; background: transparent; color: var(--ink-soft); font-size: 1.6rem; cursor: pointer; }
  .toolbar { display: flex; gap: .45rem; align-items: center; padding: .65rem; border-bottom: 1px solid var(--line); flex-wrap: wrap; }
  .path { min-width: 8rem; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: .45rem .6rem; border: 1px solid var(--line); border-radius: .45rem; font-family: var(--mono); font-size: .78rem; }
  .hidden { display: none; }
  .message { padding: .45rem .8rem; background: color-mix(in oklch, var(--accent), transparent 88%); color: var(--ink-soft); font-size: .78rem; }
  .list { min-height: 0; flex: 1; overflow: auto; padding: .6rem; }
  .entry { display: flex; align-items: center; gap: .4rem; border-bottom: 1px solid var(--line); }
  .open { min-width: 0; flex: 1; display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: .7rem; align-items: center; border: 0; background: transparent; color: inherit; text-align: left; padding: .7rem; cursor: pointer; }
  .open:hover { background: color-mix(in oklch, var(--accent), transparent 92%); }
  .open b { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; } .open small { color: var(--ink-soft); }
  .delete { border: 0; background: transparent; color: var(--danger); cursor: pointer; padding: .5rem; }
  .empty { color: var(--ink-soft); text-align: center; margin: 3rem 0; }
</style>
