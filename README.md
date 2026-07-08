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

1. **Their support number.** On launch the app shows a short code (e.g.
   `XY40-0SHD`), big and clear, with a copy button and "read this to your
   technician". The number is derived from the device's key
   (`allmystuff_cec_protocol::support_id_from_device`), so no directory is
   needed — the technician derives the same number-to-room mapping locally.
2. **It waits.** The app joins a MyOwnMesh network of type **Silent** whose
   `network_id` is derived from the number
   (`network_id_for_number` → `cec-xy400shd`). A Silent mesh auto-dials nobody
   and never gossips a roster, so the customer is only *discoverable* to someone
   who already knows the number. It connects to no one on its own.
3. **Approve or deny.** When a technician dials in, a modal appears:
   "**‹Agent Name› is trying to connect to your computer**", with the 6-char
   verification code to check against what the technician reads out, and three
   choices — **Approve Once**, **Auto-Approve for 3 hours**, **Auto-Approve
   Forever** — plus **Deny**. (These map to `ApprovalScope::Once` /
   `ThreeHours` / `Forever` in `allmystuff-cec-consent`.)
4. **While connected**, a small banner shows "‹Agent› is connected — viewing /
   controlling your screen" with a **Disconnect**, and a list of everyone with
   standing access (with a live countdown for the 3-hour ones) each with a
   **Forget** (revoke) button that bites immediately.
5. **Settings**: an **Install as a background service** toggle (so CEC Support
   reconnects after a reboot mid-repair), an Uninstall/Stop control, an "open at
   startup" toggle, and a friendly name for this computer.

It is **customer-only**. It never browses or dials anyone (that's the
technician's AllMyStuff app) — no graph, no fleets, no file browser, no
terminal. Just: my number, approve/deny, who's connected, revoke,
install-service.

## How it's built (reuse, don't clobber)

The client is a thin shell over the **AllMyStuff node engine**, embedded in "CEC
client (customer) mode" and driven over the node control socket — exactly how
AllMyStuff's own GUI is a thin client of its node. It reuses three things from
AllMyStuff (git dependencies):

- `allmystuff-node` — the node engine + control socket.
- `allmystuff-cec-protocol` — the CEC wire contract, `SupportId`,
  `network_id_for_number`, `ApprovalScope`.
- `allmystuff-cec-consent` — the three-choice consent store.

Plus its **own** OS-service installer, `crates/cec-support-service` (in this
repo), under its own service identity so installing the background service never
touches an AllMyStuff install on the same machine.

The client sets a CEC-specific home (`CEC_SUPPORT_HOME` → `MYOWNMESH_HOME`) and
CEC's forked signaling app-id before bringing the node up, so its node socket,
identity, and traffic are isolated from any AllMyStuff/MyOwnMesh install.

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
and `.allmystuff-rev` (`v0.2.20`) are those pins, which also match the
`tag = "v0.2.20"` git deps in `gui/src-tauri/Cargo.toml`.

## The node-control contract the client drives

The client only ever sends these to the node's control socket (a sibling agent
implements them on the AllMyStuff node):

| Command | Args | Result |
|---|---|---|
| `cec_status` | `{}` | `{ number, network_id, role, hosting }` |
| `cec_start_hosting` | `{}` | `{ number }` |
| `cec_stop_hosting` | `{}` | — |
| `cec_pending` | `{}` | `[{ tech, agent_name, want_control, session_id, verification_code }]` |
| `cec_approve` | `{ tech, scope, session_id, want_control }` | — (`scope` ∈ `once`\|`three_hours`\|`forever`) |
| `cec_deny` | `{ tech, session_id }` | — |
| `cec_revoke` | `{ tech }` | — |
| `cec_grants` | `{}` | active grants |
| `cec_forget_node` | `{ node }` | — |
| `cec_set_label` | `{ label }` | — (friendly-name helper, beyond the core dial/approve contract) |

Events re-emitted onto the Tauri bus for the UI:
`cec://request { tech, agent_name, want_control, session_id, verification_code }`
(drives the modal), `cec://session { session_id, state }` (the banner), and
`cec://grants { grants }` (the access list).

The background service is handled by `cec-support-service` directly (not the
node), via the `service_*` Tauri commands.

## Repository layout

```
CECSupport/
├── Cargo.toml                      light root workspace (service crate only)
├── crates/cec-support-service/     the client's OWN OS-service installer
├── gui/                            Tauri + Svelte 5 client (its own workspace)
│   ├── package.json, vite.config.ts, tsconfig.json, svelte.config.js
│   ├── src/                        App.svelte, tauri.ts bridge, store, components
│   └── src-tauri/                  Cargo.toml (v0.2.20 git deps), main.rs, tauri.conf.json
├── .allmystuff-rev / .myownmesh-rev   sidecar version pins (v0.2.20 / v0.2.32)
├── .github/workflows/ci.yml        service-crate CI + gui check/build
├── ARCHITECTURE.md · docs/         design + roadmap
```

## Build & run

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
