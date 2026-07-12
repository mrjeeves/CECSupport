# CEC Support

**One-tap remote help from Critical Error Computing** — a tiny, calm app a
customer runs so a CEC technician can view and control their screen to fix
things, like AnyDesk, but over a private peer-to-peer mesh with **no central
server** holding a directory or the session, and with access the customer
grants and can revoke at any time.

This repo is the **customer-facing client** (Windows-first). The remote-control
engine, the technician's app, and the shared CEC crates live in
[AllMyStuff](https://github.com/mrjeeves/AllMyStuff); the peer-to-peer substrate
is [MyOwnMesh](https://github.com/mrjeeves/MyOwnMesh). See
[`ARCHITECTURE.md`](ARCHITECTURE.md) and [`docs/`](docs/) for the full picture.

## What the customer sees

1. **Ask for help.** The headline button. One tap raises the customer's hand on
   the shared support area — a `SupportPresence` beacon a technician sees in
   their queue and answers by connecting to this device directly.
2. **Their support number.** Shown too (9 digits, e.g. `123 456 789`), with a
   copy button and "read this to your technician". It's derived from the
   device's key (`allmystuff_cec_protocol::support_id_from_device`) and serves
   as a display/verification label and a **fallback**: for a customer who'd
   rather read a number out, or for when the raised-hand queue is too crowded
   to pick them out, a technician can type the number and the node resolves it
   to this device on the area. It is **not** a mesh room.
3. **One shared support area.** On launch the app takes up residence on the one
   well-known MyOwnMesh network every CEC node shares
   (`HELP_NETWORK_ID` = `cecsupport-clients`), hub-shaped via `CEC_HELP_HUBS`.
   The customer connects only to CEC-operated infra hubs — never to other
   customers — and sees nobody; a technician's deliberate dial (device id,
   from the beacon) opens the one session. It connects to a technician only
   when asked, and only after the customer approves.
4. **Approve or deny.** When a technician dials in, a modal appears:
   "**‹Agent Name› is trying to connect to your computer**", with the 6-digit
   verification code to check against what the technician reads out, and three
   choices — **Approve Once**, **Auto-Approve for 3 hours**, **Auto-Approve
   Forever** — plus **Deny**. (These map to `ApprovalScope::Once` /
   `ThreeHours` / `Forever` in `allmystuff-cec-consent`.)
5. **While connected**, a small banner shows "‹Agent› is connected — viewing /
   controlling your screen" with a **Disconnect**, and a list of everyone with
   standing access (with a live countdown for the 3-hour ones) each with a
   **Forget** (revoke) button that bites immediately.
6. **If the technician asks for the $50 diagnostic session**, a purchase prompt
   appears — *optional, and only ever raised by the technician*, at any point
   in a help call: before they connect (quoting the work up front), while
   connected, or after disconnecting.
   One tap opens **our secure checkout in the customer's own browser**
   (`support.cec.direct/buy/diagnostic/` → the store's hosted checkout); the
   app never sees card details and the URL is built into the app, never taken
   from the wire. The customer taps "I've completed my purchase", the
   technician verifies the order actually landed in the store (it carries the
   customer's support number), confirms, and the prompt turns into "You're all
   set". Declining is always available until it's settled.
7. **Settings**: an **Install as a background service** toggle (so CEC Support
   reconnects after a reboot mid-repair), an Uninstall/Stop control, an "open at
   startup" toggle, and a friendly name for this computer.

It is **customer-only**. It never browses or dials anyone (that's the
technician's AllMyStuff app) — no graph, no fleets, no file browser, no
terminal. Just: ask for help, approve/deny, who's connected, revoke,
install-service.

## How it's built (reuse, don't clobber)

The client is a thin shell over the **AllMyStuff node engine**, embedded in "CEC
client (customer) mode" and driven over the node control socket — exactly how
AllMyStuff's own GUI is a thin client of its node. It reuses three things from
AllMyStuff (git dependencies):

- `allmystuff-node` — the node engine + control socket.
- `allmystuff-cec-protocol` — the CEC wire contract, `SupportId`,
  `HELP_NETWORK_ID` (the shared support area), `ApprovalScope`.
- `allmystuff-cec-consent` — the three-choice consent store.

Plus its **own** OS-service installer, `crates/cec-support-service` (in this
repo), under its own service identity so installing the background service never
touches an AllMyStuff install on the same machine.

The client rides the **same per-machine stack** an AllMyStuff install runs —
one `myownmesh` daemon, one `allmystuff-serve`, shared control sockets, one
machine identity. It reuses a running node or brings the stack up itself, so it
works solo or side by side with AllMyStuff; per-session privacy comes from each
support number's own room, not from siloing the apps.

## Install — a normal Windows app

CEC Support installs like any other app: the customer downloads the **installer
(`setup.exe`)** from [support.cec.direct](https://support.cec.direct) and
double-clicks it. **No terminal, no separate downloads, nothing to piece
together.** The installer is the Tauri bundle (`targets: all` → NSIS `setup.exe`
+ `.msi`), and it ships the `allmystuff-serve` node and the `myownmesh` daemon
**inside it** as sidecars (`externalBin` in `tauri.conf.json`) — one package with
everything the client needs.

**Reuse, don't clobber** happens at *runtime*, not install time: when the client
comes up it reuses an already-installed AllMyStuff node/daemon if one is present
and new enough, and falls back to its own bundled copies otherwise. The release
build stages the pinned versions into the bundle — `.myownmesh-rev` (`v0.2.32`)
and `.allmystuff-rev` (`v0.2.21`) are those pins, which also match the
`tag = "v0.2.21"` git deps in `gui/src-tauri/Cargo.toml`.

## The node-control contract the client drives

The client only ever sends these to the node's control socket (a sibling agent
implements them on the AllMyStuff node):

| Command | Args | Result |
|---|---|---|
| `cec_status` | `{}` | `{ number, network_id, role }` |
| `cec_online` | `{}` | `{ number }` (join the shared support area at bring-up) |
| `cec_ask_help` | `{ on }` | — (raise / lower the hand on the area) |
| `cec_pending` | `{}` | `[{ tech, agent_name, want_control, session_id, verification_code }]` |
| `cec_approve` | `{ tech, scope, session_id, want_control }` | — (`scope` ∈ `once`\|`three_hours`\|`forever`) |
| `cec_deny` | `{ tech, session_id }` | — |
| `cec_revoke` | `{ tech }` | — |
| `cec_grants` | `{}` | active grants |
| `cec_forget_node` | `{ node }` | — |
| `cec_set_label` | `{ label }` | — (friendly-name helper, beyond the core dial/approve contract) |
| `cec_purchase_status` | `{ purchase_id, state }` | — (`state` ∈ `seen`\|`opened`\|`claimed`\|`declined`) |
| `cec_purchases` | `{}` | tracked purchase asks (re-sync after a relaunch) |

Events re-emitted onto the Tauri bus for the UI:
`cec://request { tech, agent_name, want_control, session_id, verification_code }`
(drives the modal), `cec://session { session_id, state }` (the banner),
`cec://grants { grants }` (the access list), and `cec://purchase { purchase_id,
session_id, peer, agent_name, item, price, note, state, updated_at }` (the
technician-triggered diagnostic-purchase prompt; the technician-side verbs
`cec_purchase_request` / `_confirm` / `_cancel` live in the AllMyStuff app).

The background service is handled by `cec-support-service` directly (not the
node), via the `service_*` Tauri commands.

## Repository layout

```
CECSupport/
├── Justfile                        one-command ops (`just setup` / `dev` / `check` / `release`)
├── Cargo.toml                      light root workspace (service crate only)
├── crates/cec-support-service/     the client's OWN OS-service installer
├── gui/                            Tauri + Svelte 5 client (its own workspace)
│   ├── package.json, vite.config.ts, tsconfig.json, svelte.config.js
│   ├── src/                        App.svelte, tauri.ts bridge, store, components
│   └── src-tauri/                  Cargo.toml (v0.2.21 git deps), main.rs, tauri.conf.json
├── scripts/bump-version.sh         version bump used by `just release`
├── .allmystuff-rev / .myownmesh-rev   sidecar version pins (v0.2.21 / v0.2.32)
├── .github/workflows/             ci.yml (service crate + gui check) · release.yml (tag → Windows installer)
├── ARCHITECTURE.md · docs/         design + roadmap
```

## Build & run

The `Justfile` wraps all of this — the same recipe set as AllMyStuff, minus the
node-workspace bits it doesn't have. Install [`just`](https://just.systems) and:

```sh
just setup     # front-end deps (+ the Tauri prerequisites to install)
just dev       # run the app with hot reload
just check     # everything CI runs: fmt + clippy + test + gui typecheck/build
just release 0.2.0   # bump versions, commit, push, tag
```

The individual commands, if you'd rather not use `just`:

**Frontend (this repo's hard validation):**

```sh
cd gui
pnpm install
pnpm check     # svelte-check — 0 errors
pnpm build     # vite production build → gui/dist
pnpm dev       # browser preview (degrades gracefully with a demo, no backend)
```

**Service crate (compiles and tests here):**

```sh
cargo test         # from the repo root — the cec-support-service crate
```

**The full desktop app** (`gui/src-tauri`) needs the AllMyStuff node engine +
CEC crates, which are git dependencies on a sibling branch that isn't published
yet, plus the Tauri/media toolchain — so it does **not** build in this repo's
sandbox. Once those are on a published ref:

```sh
cd gui && pnpm tauri dev      # or: pnpm tauri build
```

The single `cec-support` binary is both the app and its CLI:

```sh
cec-support                   # open the app (what a customer runs)
cec-support run --service     # headless agent — what the OS service runs
cec-support service install   # keep it connected across reboots
cec-support id                # print this computer's support number
cec-support --version
```

## What builds where

| Piece | Builds in this sandbox? | Why |
|---|---|---|
| `gui/` frontend (`pnpm check`/`build`) | ✅ yes | pure Svelte/TS |
| `crates/cec-support-service` (`cargo test`) | ✅ yes | no webview/media deps |
| `gui/src-tauri` (`cargo build`) | ❌ no | AllMyStuff git deps not yet published + heavy media/webview toolchain |

## License

MIT — see [`LICENSE`](LICENSE).
