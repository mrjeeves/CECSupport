<script lang="ts">
  import { runToolboxAction, type ToolboxAction } from "../tauri";

  type Tool = {
    id: ToolboxAction;
    title: string;
    description: string;
  };

  const repairs: Tool[] = [
    { id: "sfc", title: "System File Checker", description: "Scan Windows system files and replace damaged copies." },
    { id: "dism", title: "Repair Windows image", description: "Check and repair the component store SFC relies on." },
    { id: "chkdsk", title: "Scan system drive", description: "Run the online, non-rebooting disk check." },
    { id: "flush_dns", title: "Flush DNS cache", description: "Clear stale local name-resolution entries." },
  ];

  const consoles: Tool[] = [
    { id: "event_viewer", title: "Event Viewer", description: "Open Windows logs and recent errors." },
    { id: "device_manager", title: "Device Manager", description: "Inspect hardware and driver state." },
    { id: "services", title: "Services", description: "Inspect Windows background services." },
    { id: "system_information", title: "System Information", description: "Open the detailed hardware and OS summary." },
    { id: "task_manager", title: "Task Manager", description: "Open process and performance diagnostics." },
  ];

  const advancedTools: Tool[] = [
    { id: "control_panel", title: "Control Panel", description: "Open the original Windows Control Panel." },
    { id: "windows_settings", title: "Windows Settings", description: "Open the main Windows Settings app." },
    { id: "registry_editor", title: "Registry Editor", description: "Inspect and edit the Windows registry." },
    { id: "disk_management", title: "Disk Management", description: "Manage disks, partitions, and drive letters." },
    { id: "computer_management", title: "Computer Management", description: "Open the combined Windows management console." },
    { id: "system_configuration", title: "System Configuration", description: "Change boot and service troubleshooting options." },
    { id: "windows_features", title: "Windows Features", description: "Turn optional Windows components on or off." },
    { id: "resource_monitor", title: "Resource Monitor", description: "Inspect detailed CPU, memory, disk, and network activity." },
  ];

  let running = $state<ToolboxAction | null>(null);
  let advanced = $state(false);
  let message = $state("");
  let failed = $state(false);

  async function run(tool: Tool): Promise<void> {
    if (running) return;
    running = tool.id;
    failed = false;
    message = `${tool.title} is starting…`;
    try {
      const result = await runToolboxAction(tool.id);
      message = result.output || `${tool.title} completed.`;
    } catch (error) {
      failed = true;
      message = error instanceof Error ? error.message : String(error);
    } finally {
      running = null;
    }
  }
</script>

<div class="toolbox-shell">
  <header>
    <span class="header-mark" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M14.7 6.3a4 4 0 0 0-5 5L3 18l3 3 6.7-6.7a4 4 0 0 0 5-5l-2.4 2.4-3-3 2.4-2.4z" />
      </svg>
    </span>
    <span>
      <h1>Toolbox</h1>
      <p>Useful, repeatable Windows maintenance—without hunting through menus.</p>
    </span>
  </header>

  <main>
    <div class="mode-row">
      <div class="mode-group" role="group" aria-label="Toolbox mode">
        <button
          class:active={advanced}
          aria-pressed={advanced}
          onclick={() => (advanced = !advanced)}
        >{advanced ? "Hide Advanced" : "Show Advanced"}</button>
      </div>
    </div>

    <section aria-labelledby="repair-title">
      <div class="section-heading">
        <h2 id="repair-title">Check & repair</h2>
        <span>Runs through the attached administrator terminal</span>
      </div>
      <div class="grid">
        {#each repairs as tool (tool.id)}
          <button class="tool repair" disabled={running !== null} onclick={() => void run(tool)}>
            <span class="tool-icon" aria-hidden="true">✓</span>
            <span class="tool-copy">
              <strong>{tool.title}</strong>
              <small>{tool.description}</small>
            </span>
            <span class="run-label">{running === tool.id ? "Running…" : "Run"}</span>
          </button>
        {/each}
      </div>
    </section>

    <section aria-labelledby="console-title">
      <div class="section-heading">
        <h2 id="console-title">Windows tools</h2>
        <span>Open familiar diagnostics directly</span>
      </div>
      <div class="grid">
        {#each consoles as tool (tool.id)}
          <button class="tool" disabled={running !== null} onclick={() => void run(tool)}>
            <span class="tool-icon" aria-hidden="true">↗</span>
            <span class="tool-copy">
              <strong>{tool.title}</strong>
              <small>{tool.description}</small>
            </span>
            <span class="run-label">Open</span>
          </button>
        {/each}
      </div>
    </section>

    {#if advanced}
      <section class="advanced" aria-labelledby="advanced-title">
        <div class="section-heading">
          <h2 id="advanced-title">Advanced Windows tools</h2>
          <span>These tools can change system configuration</span>
        </div>
        <div class="grid">
          {#each advancedTools as tool (tool.id)}
            <button class="tool advanced-tool" disabled={running !== null} onclick={() => void run(tool)}>
              <span class="tool-icon" aria-hidden="true">!</span>
              <span class="tool-copy">
                <strong>{tool.title}</strong>
                <small>{tool.description}</small>
              </span>
              <span class="run-label">Open</span>
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#if message}
      <div class:failed class="status" role="status" aria-live="polite">
        <span class="status-dot" aria-hidden="true"></span>
        <pre>{message}</pre>
        <button aria-label="Dismiss status" onclick={() => (message = "")}>×</button>
      </div>
    {/if}
  </main>
</div>

<style>
  .toolbox-shell {
    min-height: 100vh;
    background:
      radial-gradient(circle at 12% -8%, color-mix(in oklch, var(--accent) 12%, transparent), transparent 30rem),
      var(--bg);
  }
  header {
    display: flex;
    align-items: center;
    gap: 0.9rem;
    padding: 1rem 1.35rem;
    border-bottom: 1px solid var(--line);
    background: color-mix(in oklch, var(--surface) 94%, transparent);
  }
  .header-mark {
    width: 2.75rem;
    height: 2.75rem;
    display: grid;
    place-items: center;
    border-radius: 0.9rem;
    color: var(--accent-ink);
    background: var(--accent-soft);
    border: 1px solid color-mix(in oklch, var(--accent) 40%, var(--line));
  }
  .header-mark svg { width: 1.5rem; height: 1.5rem; }
  h1 { margin: 0; font-size: 1.25rem; }
  header p { margin: 0.16rem 0 0; color: var(--ink-soft); font-size: 0.84rem; }
  main {
    width: min(100%, 68rem);
    margin: 0 auto;
    padding: 1.25rem;
    display: grid;
    gap: 1.45rem;
  }
  section { display: grid; gap: 0.7rem; }
  .mode-row { display: flex; justify-content: flex-end; }
  .mode-group {
    display: inline-flex;
    padding: 0.22rem;
    border: 1px solid var(--line);
    border-radius: 0.72rem;
    background: var(--surface);
    box-shadow: var(--shadow-sm);
  }
  .mode-group button {
    min-width: 8.3rem;
    padding: 0.52rem 0.78rem;
    border: 1px solid transparent;
    border-radius: 0.52rem;
    color: var(--ink-soft);
    background: transparent;
    font: inherit;
    font-size: 0.78rem;
    font-weight: 750;
  }
  .mode-group button:hover { color: var(--ink); background: var(--surface-2); }
  .mode-group button.active {
    color: var(--danger);
    border-color: color-mix(in oklch, var(--danger) 35%, var(--line));
    background: color-mix(in oklch, var(--danger) 10%, var(--surface));
  }
  .section-heading {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
    padding: 0 0.15rem;
  }
  h2 { margin: 0; font-size: 0.95rem; text-transform: uppercase; letter-spacing: 0.055em; }
  .section-heading span { color: var(--ink-faint); font-size: 0.74rem; }
  .grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 0.7rem; }
  .tool {
    min-width: 0;
    min-height: 5.2rem;
    padding: 0.85rem;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.75rem;
    text-align: left;
    color: var(--ink);
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    box-shadow: var(--shadow-sm);
    transition: transform 0.08s ease, border-color 0.12s ease, background 0.12s ease;
  }
  .tool:hover:not(:disabled) { background: var(--surface-2); border-color: var(--line-strong); transform: translateY(-1px); }
  .tool:active:not(:disabled) { transform: translateY(1px); }
  .tool:disabled { cursor: default; opacity: 0.58; }
  .tool-icon {
    width: 2rem;
    height: 2rem;
    display: grid;
    place-items: center;
    border-radius: 0.65rem;
    background: var(--surface-2);
    color: var(--ink-soft);
    font-weight: 800;
  }
  .repair .tool-icon { color: var(--ok); background: var(--ok-soft); }
  .advanced { padding-top: 0.2rem; border-top: 1px solid var(--line); }
  .advanced-tool .tool-icon {
    color: var(--danger);
    background: color-mix(in oklch, var(--danger) 12%, var(--surface-2));
  }
  .tool-copy { min-width: 0; display: grid; gap: 0.18rem; }
  .tool-copy strong { font-size: 0.9rem; }
  .tool-copy small { color: var(--ink-soft); line-height: 1.35; font-size: 0.72rem; }
  .run-label { color: var(--accent-ink); font-size: 0.74rem; font-weight: 700; }
  .status {
    position: sticky;
    bottom: 1rem;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: start;
    gap: 0.7rem;
    padding: 0.85rem 0.95rem;
    border: 1px solid color-mix(in oklch, var(--ok) 45%, var(--line));
    border-radius: var(--r-md);
    background: color-mix(in oklch, var(--surface) 96%, var(--ok));
    box-shadow: var(--shadow-md);
  }
  .status.failed { border-color: color-mix(in oklch, var(--danger) 48%, var(--line)); }
  .status-dot { width: 0.55rem; height: 0.55rem; margin-top: 0.35rem; border-radius: 50%; background: var(--ok); }
  .failed .status-dot { background: var(--danger); }
  pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 10rem; overflow: auto; font: 0.74rem/1.5 var(--mono); color: var(--ink-soft); }
  .status button { border: 0; background: transparent; color: var(--ink-faint); font-size: 1.2rem; padding: 0 0.2rem; }
  @media (max-width: 700px) {
    .grid { grid-template-columns: 1fr; }
    .section-heading { align-items: flex-start; flex-direction: column; gap: 0.2rem; }
  }
</style>
