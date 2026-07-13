# CEC Support — Roadmap & build status

This file is the honest ledger: what is **compiled and tested here**, what is
**built and released but not compilable in this headless Linux CI sandbox**, and
what is **runtime-only** (needs a Windows machine + a live mesh to exercise).
CEC Support spans four repositories, so it also records the **cross-repo build
order** that got them there.

## Validated now (compiles + unit-tested)

These are green today (`cargo test` / `clippy -D warnings` / `fmt --check`):

- **`allmystuff-cec-protocol`** (in AllMyStuff) — constants (incl.
  `HELP_NETWORK_ID` = the shared support area), `SupportId` +
  `support_id_from_device` (the display/verification number),
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

## Shipped, not compilable in this sandbox (by design)

Real implementations that need a toolchain a headless Linux container doesn't
have (heavy media dev libs, a webview). The cross-repo git deps they pull are
published now and resolve; what this sandbox still lacks is the media/webview
toolchain. They are validated at the layer that *can* be checked here — the
frontend type-checker — and reviewed by inspection:

- **AllMyStuff node "CEC mode" + technician GUI** — the secret "CEC Support"
  settings tab (Agent Name + the raised-hand queue → answer; a Customer number
  entry as the fallback), revealed only by a hidden keyboard gesture. Every CEC
  node lives on the one shared support area (`cecsupport-clients`); a technician
  answers a raised hand by dialing that customer's device directly, and dialed
  customers show as ordinary graph peers. The tab lists them from CEC state via
  `cec_dialed`. The node-control command surface is `cec_online`, `cec_dial_node`
  (answer a hand / reconnect), `cec_dial` (number fallback), `cec_help_watch` /
  `cec_help_list`, `cec_pending`, `cec_approve`, `cec_deny`, `cec_revoke`,
  `cec_dialed`, the app-wide `forget_node` (on every node's gear), and the
  `cec://*` events. The Svelte frontend is type-checked (`pnpm check`); the node
  backend needs the Linux media stack **and** the MyOwnMesh hub/`Silent` API
  below.
- **CEC Support client GUI (`gui/`) + `cec-support` binary** — the Tauri + Svelte
  customer app (inline support number, three-choice approve modal, connected
  banner, access list, grant-scoped autostart settings). It installs like a
  **normal Windows app**: the Tauri bundle is an NSIS `setup.exe` + `.msi` that
  ships the `allmystuff-serve` node and the `myownmesh` daemon **inside it**
  (`externalBin`), so the customer double-clicks one file — no terminal, no
  one-liners. Frontend type-checked; the Tauri backend depends on the AllMyStuff
  node engine (git dep on a published AllMyStuff `tag`) + media stack.

## The one substrate change

- **MyOwnMesh `NetworkKind::Silent` + `JoinedNetwork::connect_peer`** — a Silent
  network auto-dials nobody and doesn't gossip; peers are visible via presence
  without connecting; `connect_peer` dials one specific peer; inbound offers are
  still answered. This is additive and backward-compatible. It is the only CEC
  change to MyOwnMesh.

## Cross-repo build order

Because the heavy pieces depend on each other across repos, they had to land in
this order for an end-to-end build — and now have, each layer merged, tagged,
and released:

1. **MyOwnMesh** — `NetworkKind::Silent` + `connect_peer` merged and tagged ✓.
2. **AllMyStuff** — `.myownmesh-rev` moved to that MyOwnMesh tag, workspace
   bumped, merged and tagged, and the tag published as a GitHub release. The
   node "CEC mode" builds against the `Silent`/`connect_peer` API over the daemon
   control socket.
3. **CECSupport** — `gui/src-tauri` git deps pin that AllMyStuff `tag`, and
   `.allmystuff-rev` / `.myownmesh-rev` hold the matching sidecar pins. `just
   release` tags the repo, and `.github/workflows/release.yml` builds the Windows
   Tauri bundle — the GUI's `build.rs` fetches those two pinned sidecars from
   their releases and bundles them into the `setup.exe` / `.msi`, which the
   workflow publishes to the GitHub release. (The AllMyStuff release is published
   first, so the `allmystuff-serve` sidecar resolves.)
4. **support.cec.direct** — published; its **Download for Windows** button points
   straight at the CEC Support `setup.exe` release. No install one-liners.

The light workspaces (the three tested crates above) build independently at
every step.

## Runtime verification still owed (needs Windows + a live mesh)

None of the following can be exercised in this headless environment; they are
the runtime acceptance tests, run on a Windows box against a live mesh:

- End-to-end (headline): customer launches → **Ask for help** → technician sees
  the raised hand and answers → customer sees "‹Agent› is trying to connect" →
  Approve Once/3h/Forever → screen appears → control works → Revoke stops it
  immediately.
- End-to-end (fallback): technician types the customer's number → node resolves
  it to that device on the shared area → same approve/connect flow.
- On the shared area, a customer connects only to the CEC infra hubs and forms
  **no** connection to another customer, and no session forms until the
  technician's deliberate `connect_peer`.
- Windows: grant-scoped autostart (start with Windows while a grant is live) and
  the `sc.exe` service installer both keeping the machine reachable across a
  reboot mid-repair, and that a 3-hour grant expires and re-prompts.
- Screen capture + input injection latency/quality on Windows (Media Foundation
  H.264 path) via the reused AllMyStuff node.
- Reuse-or-bundle: install on a machine with AllMyStuff present (reuse the
  daemon) and on one without (bundle), with no clobbering of an AllMyStuff
  service.

## Nice-to-haves (later)

- **A proper support queue.** The raised-hand list is the queue today; the
  number is the fallback for when that list gets too crowded to pick someone
  out. A real queue — ordering, claim/assignment across technicians, wait-time
  surfacing — is the near-term follow-up that lets the hand-raise path scale
  past the point where the number workaround is needed.
- A random (non-derived) number option for customers who want a fresh code per
  session rather than one stable to their device key.
- Session audit log on the customer side ("who connected, when, for how long").
