# CEC Support — Roadmap & build status

This file is the honest ledger: what is **compiled and tested**, what is
**written but not compilable in a headless Linux CI sandbox**, and what is
**runtime-only** (needs a Windows machine + a live mesh). CEC Support spans four
repositories, so it also records the **cross-repo build order**.

## Validated now (compiles + unit-tested)

These are green today (`cargo test` / `clippy -D warnings` / `fmt --check`):

- **`allmystuff-cec-protocol`** (in AllMyStuff) — constants, `SupportId` +
  `support_id_from_device`, `network_id_for_number` / `network_id_for_device`,
  `ApprovalScope` (Once / ThreeHours / Forever), `SupportPresence`,
  `ControlMessage` / `ConnectControl` / `AppControl`, the media-frame codec.
  **16 unit tests.**
- **`allmystuff-cec-consent`** (in AllMyStuff) — the three-choice grant store:
  in-memory `Once`, persisted+expiring `ThreeHours`, persisted `Forever`;
  per-frame `is_allowed`; `revoke` ("Forget this technician"); expiry pruning;
  crash-safe atomic persistence with corrupt-file quarantine; pubkey-suffix
  canonicalisation. **10 unit tests.**
- **`cec-support-service`** (this repo) — the client's own systemd / launchd /
  Windows-SCM service installer, with pure, tested unit/plist/`sc` renderers.
  **5 unit tests.**

## Written, not compilable in this sandbox (by design)

Real implementations that need a toolchain a headless Linux container doesn't
have (heavy media dev libs, a webview, cross-repo git deps that must be pushed
first). They are validated at the layer that *can* be checked — the frontend
type-checker — and reviewed by inspection:

- **AllMyStuff node "CEC mode" + technician GUI** — the secret "CEC Support"
  settings tab (Agent Name + Customer number → dial), revealed only by a hidden
  keyboard gesture. Dialed customers show as ordinary graph peers (no "fleet
  group" — the CEC mesh is Silent, with no roster); the tab lists them from CEC
  state via `cec_dialed`. The node-control command surface is `cec_start_hosting`,
  `cec_dial`, `cec_pending`, `cec_approve`, `cec_deny`, `cec_revoke`, `cec_dialed`,
  the app-wide `forget_node` (on every node's gear), and the `cec://*` events. The
  Svelte frontend is type-checked (`pnpm check`); the node backend needs the
  Linux media stack **and** the MyOwnMesh `Silent` API below.
- **CEC Support client GUI (`gui/`) + `cec-support` binary** — the Tauri + Svelte
  customer app (number screen, three-choice approve modal, connected banner,
  access list, service toggle). It installs like a **normal Windows app**: the
  Tauri bundle is an NSIS `setup.exe` + `.msi` that ships the `allmystuff-serve`
  node and the `myownmesh` daemon **inside it** (`externalBin`), so the customer
  double-clicks one file — no terminal, no one-liners. Frontend type-checked; the
  Tauri backend depends on the AllMyStuff node engine (git dep, `tag = "v0.2.21"`)
  + media stack.

## The one substrate change

- **MyOwnMesh `NetworkKind::Silent` + `JoinedNetwork::connect_peer`** — a Silent
  network auto-dials nobody and doesn't gossip; peers are visible via presence
  without connecting; `connect_peer` dials one specific peer; inbound offers are
  still answered. This is additive and backward-compatible. It is the only CEC
  change to MyOwnMesh.

## Cross-repo build order

Because the heavy pieces depend on each other across repos, they must land in
this order for an end-to-end build:

1. **MyOwnMesh** — `NetworkKind::Silent` + `connect_peer` merged and tagged
   **`v0.2.32`** ✓.
2. **AllMyStuff** — `.myownmesh-rev` moved to `v0.2.32`, workspace bumped to
   **`0.2.21`**; merge and tag `v0.2.21`. The node "CEC mode" builds against the
   `Silent`/`connect_peer` API over the daemon control socket.
3. **CECSupport** — `gui/src-tauri` git deps pin `tag = "v0.2.21"`;
   `.allmystuff-rev` = `v0.2.21`, `.myownmesh-rev` = `v0.2.32`. `just release`
   tags the repo, and `.github/workflows/release.yml` builds the Windows Tauri
   bundle — the GUI's `build.rs` fetches those two pinned sidecars from their
   releases and bundles them into the `setup.exe` / `.msi`, which the workflow
   publishes to the GitHub release. (Cut this after AllMyStuff `v0.2.21` is
   published, so the `allmystuff-serve` sidecar resolves.)
4. **support.cec.direct** — published; its **Download for Windows** button points
   straight at the CEC Support `setup.exe` release. No install one-liners.

The light workspaces (the three tested crates above) build independently at
every step.

## Runtime verification still owed (needs Windows + a live mesh)

None of the following can be exercised in this environment; they are the
acceptance tests for the first real build:

- End-to-end: customer launches → reads number → technician dials by number →
  customer sees "‹Agent› is trying to connect" → Approve Once/3h/Forever →
  screen appears → control works → Revoke stops it immediately.
- The Silent mesh actually forms **no** connection until `connect_peer`, and the
  per-number room is unreachable without the number.
- Windows: `sc.exe` service install/uninstall, unattended reconnect after
  reboot, and that a 3-hour grant expires and re-prompts.
- Screen capture + input injection latency/quality on Windows (Media Foundation
  H.264 path) via the reused AllMyStuff node.
- Reuse-or-bundle: install on a machine with AllMyStuff present (reuse the
  daemon) and on one without (bundle), with no clobbering of an AllMyStuff
  service.

## Nice-to-haves (later)

- Optional: a future MyOwnMesh could let a Silent node suppress even the
  transport handshake for un-dialed peers entirely (today a dialed peer's offer
  is answered; discovery is presence-only). Not needed for the per-number-room
  model, where rooms are tiny.
- A random (non-derived) number option for customers who want a fresh code per
  session rather than one stable to their device key.
- Session audit log on the customer side ("who connected, when, for how long").
