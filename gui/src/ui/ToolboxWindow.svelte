<script lang="ts">
  import { onMount } from "svelte";
  import {
    onToolboxProgress,
    runToolboxAction,
    type ToolboxAction,
    type ToolboxProgress,
  } from "../tauri";

  type Tool = {
    id: ToolboxAction;
    title: string;
    description: string;
    cta?: "Run" | "Open";
  };

  type Job = {
    id: string;
    action: ToolboxAction;
    title: string;
    output: string;
    started: number;
    status: "running" | "complete" | "failed";
  };

  const repairs: Tool[] = [
    { id: "sfc", title: "System File Checker", description: "Scan Windows system files and replace damaged copies." },
    { id: "dism", title: "Repair Windows image", description: "Check and repair the component store SFC relies on." },
    { id: "chkdsk", title: "Scan system drive", description: "Run the online, non-rebooting disk check." },
    { id: "flush_dns", title: "Flush DNS cache", description: "Clear stale local name-resolution entries." },
    { id: "disk_cleanup", title: "Disk Cleanup", description: "Open Windows Disk Cleanup to safely review removable files.", cta: "Open" },
  ];

  const consoles: Tool[] = [
    { id: "event_viewer", title: "Event Viewer", description: "Open Windows logs and recent errors." },
    { id: "device_manager", title: "Device Manager", description: "Inspect hardware and driver state." },
    { id: "services", title: "Services", description: "Inspect Windows background services." },
    { id: "system_information", title: "System Information", description: "Open the detailed hardware and OS summary." },
    { id: "task_manager", title: "Task Manager", description: "Open process and performance diagnostics." },
    { id: "windows_settings", title: "Windows Settings", description: "Open the main Windows Settings app." },
  ];

  const advancedTools: Tool[] = [
    { id: "control_panel", title: "Control Panel", description: "Open the original Windows Control Panel." },
    { id: "crucible_tests", title: "Crucible Tests", description: "Open the interactive administrator stress-test console." },
    { id: "registry_editor", title: "Registry Editor", description: "Inspect and edit the Windows registry." },
    { id: "disk_management", title: "Disk Management", description: "Manage disks, partitions, and drive letters." },
    { id: "computer_management", title: "Computer Management", description: "Open the combined Windows management console." },
    { id: "system_configuration", title: "System Configuration", description: "Change boot and service troubleshooting options." },
    { id: "windows_features", title: "Windows Features", description: "Turn optional Windows components on or off." },
    { id: "resource_monitor", title: "Resource Monitor", description: "Inspect detailed CPU, memory, disk, and network activity." },
    { id: "reliability_monitor", title: "Reliability Monitor", description: "Review the stability timeline, failures, and application crashes." },
  ];

  let advanced = $state(false);
  let jobs = $state<Job[]>([]);
  let now = $state(Date.now());
  let runSequence = 0;
  let progressReady: Promise<void> = Promise.resolve();
  const MAX_PROGRESS_CHARS = 24_000;

  function running(action: ToolboxAction): boolean {
    return jobs.some((job) => job.action === action && job.status === "running");
  }

  function appendOutput(current: string, chunk: string): string {
    const combined = current ? `${current}\n${chunk}` : chunk;
    if (combined.length <= MAX_PROGRESS_CHARS) return combined;
    return `... earlier output trimmed ...\n${combined.slice(-MAX_PROGRESS_CHARS)}`;
  }

  function updateJob(id: string, update: (job: Job) => Job): void {
    jobs = jobs.map((job) => (job.id === id ? update(job) : job));
  }

  function followOutput(node: HTMLElement, _value: string) {
    const scrollToEnd = () => {
      node.scrollTop = node.scrollHeight;
    };
    requestAnimationFrame(scrollToEnd);
    return {
      update(_next: string) {
        requestAnimationFrame(scrollToEnd);
      },
    };
  }

  function receiveProgress(progress: ToolboxProgress): void {
    updateJob(progress.runId, (job) => ({
      ...job,
      output: appendOutput(
        job.output,
        progress.stream === "stderr" ? `Warning: ${progress.chunk}` : progress.chunk,
      ),
    }));
  }

  function elapsed(job: Job): string {
    const seconds = Math.max(0, Math.floor((now - job.started) / 1000));
    if (seconds < 60) return `${seconds}s`;
    return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  }

  onMount(() => {
    let disposed = false;
    let stopProgress: (() => void) | null = null;
    progressReady = onToolboxProgress(receiveProgress)
      .then((stop) => {
        if (disposed) stop();
        else stopProgress = stop;
      })
      .catch(() => {});
    const ticker = window.setInterval(() => (now = Date.now()), 1000);
    return () => {
      disposed = true;
      stopProgress?.();
      window.clearInterval(ticker);
    };
  });

  async function run(tool: Tool): Promise<void> {
    if (running(tool.id)) return;
    const runId = `${tool.id}-${Date.now()}-${++runSequence}`;
    jobs = [
      {
        id: runId,
        action: tool.id,
        title: tool.title,
        output: "",
        started: Date.now(),
        status: "running",
      },
      ...jobs,
    ];
    await progressReady;
    try {
      const result = await runToolboxAction(tool.id, runId);
      updateJob(runId, (job) => ({
        ...job,
        status: "complete",
        output: job.output || result.output || `${tool.title} completed.`,
      }));
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      updateJob(runId, (job) => ({
        ...job,
        status: "failed",
        output: job.output || detail,
      }));
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

  <div class="toolbox-workspace" class:has-progress={jobs.length > 0}>
    <main>
      <section aria-labelledby="repair-title">
        <div class="section-heading">
          <h2 id="repair-title">Check & repair</h2>
          <span>Safe checks and Windows cleanup tools</span>
        </div>
        <div class="grid">
          {#each repairs as tool (tool.id)}
            <button class="tool repair" disabled={running(tool.id)} onclick={() => void run(tool)}>
              <span class="tool-icon" aria-hidden="true">&#10003;</span>
              <span class="tool-copy">
                <strong>{tool.title}</strong>
                <small>{tool.description}</small>
              </span>
              <span class="run-label">{running(tool.id) ? "Running…" : tool.cta ?? "Run"}</span>
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
            <button class="tool" disabled={running(tool.id)} onclick={() => void run(tool)}>
              <span class="tool-icon" aria-hidden="true">&#8599;</span>
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
              <button class="tool advanced-tool" disabled={running(tool.id)} onclick={() => void run(tool)}>
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

      <div class="mode-row">
        <div class="mode-group" role="group" aria-label="Toolbox mode">
          <button
            class:active={advanced}
            aria-pressed={advanced}
            onclick={() => (advanced = !advanced)}
          >{advanced ? "Hide Advanced" : "Show Advanced"}</button>
        </div>
      </div>
    </main>

    {#if jobs.length}
      <aside class="progress-panel" aria-label="Toolbox activity">
        <div class="progress-heading">
          <span>
            <h2>Activity</h2>
            <small>Live tool output</small>
          </span>
          <span class="job-count">{jobs.length}</span>
        </div>
        <div class="progress-list" aria-label="Toolbox task progress" aria-live="polite">
          {#each jobs as job (job.id)}
            <article class="status" class:running={job.status === "running"} class:failed={job.status === "failed"}>
              <span class="status-dot" aria-hidden="true"></span>
              <div class="status-body">
                <div class="status-head">
                  <strong>{job.title}</strong>
                  <span>{job.status === "running" ? `Running · ${elapsed(job)}` : job.status === "failed" ? "Failed" : "Complete"}</span>
                </div>
                <pre use:followOutput={`${job.status}:${job.output}`}>{job.output || `${job.title} is starting…`}</pre>
              </div>
              {#if job.status !== "running"}
                <button aria-label="Dismiss {job.title} progress" onclick={() => (jobs = jobs.filter((item) => item.id !== job.id))}>&times;</button>
              {/if}
            </article>
          {/each}
        </div>
      </aside>
    {/if}
  </div>
</div>

<style>
  .toolbox-shell {
    height: 100vh;
    height: 100dvh;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    overflow: hidden;
    background:
      radial-gradient(circle at 12% -8%, color-mix(in oklch, var(--accent) 12%, transparent), transparent 30rem),
      var(--bg);
  }
  header {
    display: flex;
    align-items: center;
    gap: 0.72rem;
    padding: 0.72rem 1rem;
    border-bottom: 1px solid var(--line);
    background: color-mix(in oklch, var(--surface) 94%, transparent);
  }
  .header-mark {
    width: 2.35rem;
    height: 2.35rem;
    display: grid;
    place-items: center;
    border-radius: 0.8rem;
    color: var(--accent-ink);
    background: var(--accent-soft);
    border: 1px solid color-mix(in oklch, var(--accent) 40%, var(--line));
  }
  .header-mark svg { width: 1.3rem; height: 1.3rem; }
  h1 { margin: 0; font-size: 1.15rem; }
  header p { margin: 0.1rem 0 0; color: var(--ink-soft); font-size: 0.78rem; }
  .toolbox-workspace {
    min-width: 0;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    overflow: hidden;
    transition: grid-template-columns 160ms ease-out;
  }
  .toolbox-workspace.has-progress {
    grid-template-columns: minmax(0, 1fr) clamp(20rem, 32vw, 25rem);
  }
  main {
    min-width: 0;
    width: min(100%, 76rem);
    height: 100%;
    box-sizing: border-box;
    margin: 0 auto;
    padding: 0.85rem 1rem;
    display: grid;
    align-content: start;
    gap: 0.9rem;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  section { display: grid; gap: 0.48rem; }
  .mode-row {
    display: flex;
    justify-content: flex-end;
    padding-top: 0.55rem;
    border-top: 1px solid var(--line);
  }
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
    padding: 0.48rem 0.74rem;
    border: 1px solid transparent;
    border-radius: 0.52rem;
    color: var(--ink-soft);
    background: transparent;
    font: inherit;
    font-size: 0.76rem;
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
  h2 { margin: 0; font-size: 0.88rem; text-transform: uppercase; letter-spacing: 0.055em; }
  .section-heading span { color: var(--ink-faint); font-size: 0.7rem; }
  .grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 0.52rem; }
  .tool {
    min-width: 0;
    min-height: 4.35rem;
    padding: 0.65rem;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.58rem;
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
    width: 1.8rem;
    height: 1.8rem;
    display: grid;
    place-items: center;
    border-radius: 0.6rem;
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
  .tool-copy { min-width: 0; display: grid; gap: 0.14rem; }
  .tool-copy strong { font-size: 0.84rem; }
  .tool-copy small { color: var(--ink-soft); line-height: 1.28; font-size: 0.68rem; }
  .run-label { color: var(--accent-ink); font-size: 0.7rem; font-weight: 700; }
  .progress-panel {
    min-width: 0;
    min-height: 0;
    padding: 0.85rem;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 0.72rem;
    overflow: hidden;
    border-left: 1px solid var(--line);
    background: color-mix(in oklch, var(--surface) 96%, var(--accent));
    box-shadow: -0.8rem 0 2rem color-mix(in oklch, var(--ink) 8%, transparent);
    animation: panel-in 160ms ease-out;
  }
  .progress-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.05rem 0.1rem 0.62rem;
    border-bottom: 1px solid var(--line);
  }
  .progress-heading > span:first-child { display: grid; gap: 0.12rem; }
  .progress-heading small { color: var(--ink-faint); font-size: 0.7rem; }
  .job-count {
    min-width: 1.6rem;
    height: 1.6rem;
    padding: 0 0.35rem;
    display: grid;
    place-items: center;
    box-sizing: border-box;
    border-radius: 999px;
    color: var(--accent-ink);
    background: var(--accent-soft);
    font-size: 0.7rem;
    font-weight: 800;
  }
  .progress-list {
    min-height: 0;
    display: grid;
    align-content: start;
    gap: 0.58rem;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 0.05rem 0.2rem 0.4rem 0.05rem;
    scrollbar-gutter: stable;
  }
  .status {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: start;
    gap: 0.7rem;
    padding: 0.68rem 0.8rem;
    border: 1px solid color-mix(in oklch, var(--ok) 45%, var(--line));
    border-radius: var(--r-md);
    background: color-mix(in oklch, var(--surface) 96%, var(--ok));
    box-shadow: var(--shadow-md);
  }
  .status.failed { border-color: color-mix(in oklch, var(--danger) 48%, var(--line)); }
  .status-dot { width: 0.55rem; height: 0.55rem; margin-top: 0.35rem; border-radius: 50%; background: var(--ok); }
  .status.running .status-dot { background: var(--accent); animation: progress-pulse 1.1s ease-in-out infinite; }
  .failed .status-dot { background: var(--danger); }
  .status-body { min-width: 0; display: grid; gap: 0.3rem; }
  .status-head { display: flex; justify-content: space-between; gap: 0.8rem; font-size: 0.77rem; }
  .status-head span { color: var(--ink-faint); font-size: 0.7rem; white-space: nowrap; }
  pre {
    margin: 0;
    max-height: clamp(9rem, 26vh, 15rem);
    padding: 0.62rem 0.7rem;
    box-sizing: border-box;
    overflow: auto;
    overscroll-behavior: contain;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    border: 1px solid var(--line);
    border-radius: 0.55rem;
    background: var(--bg);
    font: 0.72rem/1.52 var(--mono);
    color: var(--ink-soft);
    scrollbar-gutter: stable;
  }
  .status button { border: 0; background: transparent; color: var(--ink-faint); font-size: 1.2rem; padding: 0 0.2rem; }
  @keyframes panel-in { from { opacity: 0; transform: translateX(1rem); } }
  @keyframes progress-pulse { 50% { opacity: 0.38; transform: scale(0.78); } }
  @media (max-width: 1100px) {
    .toolbox-workspace.has-progress .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  @media (max-width: 850px) {
    .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .toolbox-workspace.has-progress {
      grid-template-columns: minmax(0, 1fr) minmax(17rem, 42vw);
    }
    .toolbox-workspace.has-progress .grid { grid-template-columns: 1fr; }
  }
  @media (max-width: 620px) {
    .grid { grid-template-columns: 1fr; }
    .section-heading { align-items: flex-start; flex-direction: column; gap: 0.2rem; }
  }
</style>
