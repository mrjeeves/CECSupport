<script lang="ts">
  // The customer's side of the live support chat, sitting in the top-left card
  // slot. While a technician is connected this replaces the "Ask for help"
  // front door (the store swaps it in on connect and back out on disconnect);
  // it also opens on its own when the customer taps a technician's name in the
  // access list, to read back an earlier conversation.
  //
  // One render path, keyed by the technician's canonical device id: the store's
  // `chatThread(peer)` holds both sent and received lines (a sent line is
  // echoed back over `cec://chat`), so the bubbles here are simply that thread —
  // the customer's own lines ("client") on the right, the technician's on the
  // left.
  import { tick } from "svelte";
  import { store } from "../store.svelte";

  let { peer }: { peer: string } = $props();

  const thread = $derived(store.chatThread(peer));
  const name = $derived(store.chatPeerName(peer));
  const connected = $derived(store.isConnectedTo(peer));
  // A close affordance only when this isn't the technician who's actively on the
  // session — you can't dismiss the chat with whoever is on your screen (it's
  // the only way to talk to them), but you can step back out of an earlier
  // conversation you opened to read.
  const canClose = $derived(store.connectedTech !== peer);

  let draft = $state("");
  let threadEl = $state<HTMLDivElement | null>(null);

  async function scrollToEnd(): Promise<void> {
    await tick();
    if (threadEl) threadEl.scrollTop = threadEl.scrollHeight;
  }

  // Stick to the newest line as the thread grows, and keep it marked read while
  // it's the chat on screen.
  $effect(() => {
    void thread.length;
    store.markChatRead(peer);
    void scrollToEnd();
  });

  function send(): void {
    const body = draft.trim();
    if (!body) return;
    void store.sendChat(peer, body);
    draft = "";
  }

  function onKey(e: KeyboardEvent): void {
    // Enter sends; Shift+Enter keeps a newline for a longer note.
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  function fmtTime(ts: number): string {
    try {
      // `ts` is UNIX **seconds** — to milliseconds for the Date.
      return new Date(ts * 1000).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return "";
    }
  }
</script>

<section class="chat card" aria-label={`Chat with ${name}`}>
  <header class="head">
    <span class="dot" class:online={connected} aria-hidden="true"></span>
    <div class="who">
      <div class="name">{name}</div>
      <div class="sub">{connected ? "Connected — you can chat now" : "Not connected right now"}</div>
    </div>
    {#if canClose}
      <button class="close" aria-label="Close chat" title="Close chat" onclick={() => store.closeChat()}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
        </svg>
      </button>
    {/if}
  </header>

  <div class="thread" bind:this={threadEl}>
    {#if thread.length === 0}
      <div class="empty">
        <p>No messages yet.</p>
        <p class="hint">
          {connected
            ? "Say hello, or ask your technician anything while they're connected."
            : "Your conversation will appear here when a technician connects."}
        </p>
      </div>
    {:else}
      {#each thread as m (m.id)}
        <div class="row" class:mine={m.from === "client"}>
          <div class="bubble">
            <span class="text">{m.text}</span>
            <span class="ts">{fmtTime(m.ts)}</span>
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <footer class="composer">
    <textarea
      class="input"
      rows="1"
      placeholder={connected ? "Message your technician…  (Enter to send)" : "Chat opens when a technician connects"}
      disabled={!connected}
      bind:value={draft}
      onkeydown={onKey}
    ></textarea>
    <button class="send" onclick={send} disabled={!connected || draft.trim().length === 0}>
      Send
    </button>
  </footer>
</section>

<style>
  .chat {
    width: 100%;
    max-width: 30rem;
    display: flex;
    flex-direction: column;
    /* Tall enough to feel like a conversation, short enough to leave the
       connected banner + access list visible below it. */
    height: 26rem;
    max-height: 70vh;
    overflow: hidden;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.75rem 0.95rem;
    border-bottom: 1px solid var(--line);
    flex: 0 0 auto;
  }
  .dot {
    flex: 0 0 auto;
    width: 0.7rem;
    height: 0.7rem;
    border-radius: 50%;
    background: var(--ink-faint);
  }
  .dot.online {
    background: var(--ok);
    box-shadow: 0 0 0 3px var(--ok-soft);
  }
  .who {
    flex: 1 1 auto;
    min-width: 0;
    text-align: left;
  }
  .name {
    font-weight: 700;
    font-size: 1rem;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: 0.78rem;
    color: var(--ink-soft);
  }
  .close {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    padding: 0;
    color: var(--ink-soft);
    background: transparent;
    border: 1px solid var(--line);
    border-radius: var(--r-pill);
    cursor: pointer;
  }
  .close:hover {
    color: var(--ink);
    border-color: var(--line-strong);
  }
  .close svg {
    width: 1rem;
    height: 1rem;
  }

  .thread {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.9rem;
  }
  .empty {
    margin: auto;
    max-width: 22rem;
    text-align: center;
    color: var(--ink-faint);
  }
  .empty p {
    margin: 0 0 0.4rem;
  }
  .empty .hint {
    font-size: 0.82rem;
    line-height: 1.45;
  }

  .row {
    display: flex;
    justify-content: flex-start;
  }
  /* The customer's own lines sit on the right; the technician's on the left. */
  .row.mine {
    justify-content: flex-end;
  }
  .bubble {
    max-width: 80%;
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    padding: 0.45rem 0.65rem;
    border-radius: var(--r-md);
    background: var(--surface-2);
    border: 1px solid var(--line);
    font-size: 0.92rem;
    line-height: 1.4;
  }
  .row.mine .bubble {
    background: var(--accent-soft);
    border-color: var(--accent);
  }
  .text {
    white-space: pre-wrap;
    word-break: break-word;
    min-width: 0;
    color: var(--ink);
  }
  .ts {
    flex: 0 0 auto;
    font-size: 0.66rem;
    color: var(--ink-faint);
    align-self: flex-end;
  }

  .composer {
    display: flex;
    gap: 0.5rem;
    align-items: flex-end;
    padding: 0.7rem 0.75rem;
    border-top: 1px solid var(--line);
    flex: 0 0 auto;
  }
  .input {
    flex: 1 1 auto;
    resize: none;
    max-height: 7rem;
    font: inherit;
    font-size: 0.92rem;
    line-height: 1.4;
    padding: 0.55rem 0.65rem;
    color: var(--ink);
    background: var(--surface);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-md);
  }
  .input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .input:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .send {
    flex: 0 0 auto;
    font: inherit;
    font-weight: 700;
    font-size: 0.9rem;
    padding: 0.55rem 1rem;
    color: var(--accent-ink);
    background: var(--accent);
    border: none;
    border-radius: var(--r-pill);
    cursor: pointer;
  }
  .send:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
