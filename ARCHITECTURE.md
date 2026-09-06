# CEC Support — Architecture

CEC Support is the customer app for Critical Error Computing. The customer shares
its nine-digit support number; a technician enters it in AllMyStuff and requests
access. The customer approves the named technician before screen or control access
starts. Both apps use the AllMyStuff node engine and MyOwnMesh transport.

## Discovery and connection

The public `cecsupport-clients` room is a Silent, admission-disabled directory.
Customers announce their presence; technicians listen without announcing themselves.
A support number derives from the device's public key and resolves against this
live directory. Discovery provides no access to a computer or appliance.

A deliberate dial joins the customer's `cec-<number>` session room and opens a
WebRTC transport to that device. The technician sends a connect request containing
an agent name, session id, and requested capabilities. The customer checks the
name and six-digit verification code, then approves or declines. Every privileged
operation remains subject to the customer's consent grant.

Existing saved customer connections can reconnect through their known identity.
The help queue and its presence beacons are retired. Updated engines remove any
persisted `cecsupport-asking` membership on startup or reconnect. CECSupport also
withdraws an old ask when it announces through an older shared engine.

## KVM approval

NanoKVM and NanoKVM-Pro display their own support numbers. Incoming requests can
be approved or declined in the appliance web UI, the CECSupport KVM card, or
AllMyStuff's KVM Support panel. Approval names the exact technician and session,
so an expired card cannot approve a replacement request.

The physical button and each app's approval-window button have the same behavior:

- If a request is pending, approve the oldest one immediately.
- Otherwise, wait up to five minutes for one support-number request.
- Pressing again refreshes the wait to five minutes.
- The first request consumes that permission and receives three hours of access.
- A reconnect under an existing grant does not extend its deadline or consume a
  newly opened window. Unclaiming the appliance clears pending requests and the window.

The web UI and app cards display the remaining wait from the appliance's duration,
using a local monotonic clock so an unset KVM wall clock cannot break the countdown.
The existing long-hold factory-reset gesture is preserved.

The compatibility status endpoint remains `GET /api/mesh/help`. Decisions use
`POST /api/mesh/help/approve` or `/deny` with `{ technician, sessionId }`, and
`POST /api/mesh/help/arm` approves the current request or opens/refreshes the window.
Approval endpoints require the local web login or an owner/fleet mesh tunnel.
Temporary technicians cannot mint or extend consent through their own tunnel.
The former raise, lower, and toggle endpoints are removed.

## Consent: Approve Once / 3 hours / Forever

The customer's three choices map to `allmystuff_cec_protocol::ApprovalScope` and
are enforced by `allmystuff-cec-consent`:

| Choice                   | `ApprovalScope` | Stored     | Lifetime            | Unattended? |
|--------------------------|-----------------|------------|---------------------|-------------|
| Approve Once             | `Once`          | in memory  | this session        | no          |
| Auto-Approve for 3 hours | `ThreeHours`    | disk       | 3 hours, then re-ask| yes, 3h     |
| Auto-Approve Forever     | `Forever`       | disk       | until revoked       | yes         |

- **Per-frame enforcement.** The node checks `ConsentStore::is_allowed(tech, cap,
  now)` on every privileged frame (screen, input), so **revoke bites mid-session**
  — it never caches authorisation for the duration of a route.
- **Time is injected, never read** by the store, so the whole thing is
  deterministic and unit-tested (26 tests across the two crates).
- **Forget this technician** = `ConsentStore::revoke` + teardown. Every node on a
  technician's graph also gets a general **"Forget this node"** gear action.

Persistent grants (3-hours, Forever) are what make **unattended repair** work:
combined with grant-scoped autostart (start with Windows while a grant is live),
CEC can reconnect after a reboot without anyone at the keyboard — bounded by the
3-hour window unless the customer chose Forever, and revocable from the customer
side at any time.

## Reuse, don't clobber

Like the MyOwnMesh installer, the CEC Support client **reuses an existing
AllMyStuff / `myownmesh` install if one is present and new enough** (shares the
daemon), and **bundles its own** binaries otherwise, so a customer who has never
heard of AllMyStuff still gets a self-contained app. Its background service uses
its **own** identity (`cec-support-service`, service name `CECSupport`) so it
never disturbs an AllMyStuff service on the same machine.

## One stack per machine — layered clients, not silos

CEC Support, AllMyStuff, and MyOwnMesh are **layers of one engine**: MyOwnMesh
is the mesh daemon, `allmystuff-serve` is the node riding it, and each app is a
client of that same per-machine stack over the shared control sockets. CEC
Support does **not** fork `MYOWNMESH_HOME`, the signaling app-id, or the
machine identity — one daemon, one node, one device id, whichever app brought
it up. Either app runs solo (it spawns the stack itself) or side by side (it
reuses the running one); neither requires the other's GUI. Per-session privacy
comes from the shared area being Silent (nobody connects to anybody until a
technician deliberately dials, and the apps' own graph presence never crosses
the CEC rooms except between deliberate CEC peers) plus the customer's
per-frame consent — not from siloing the apps.

`CEC_SUPPORT_HOME` holds only CEC's **own app files** (service state, logs);
the mesh stack's home stays the shared `~/.myownmesh`.

## Settings and component reconciliation

Settings is split into General, Startup, and Updates rather than one scrolling
surface. Updates combines CEC Support's own release updater with a live
component matrix. It reports Current and Pinned versions independently for the
desktop app, `allmystuff-serve`, `myownmesh`, AMSTerm, Crucible, and the
installed CEC service payload. An on-disk service copy that differs from the
running process is therefore visible instead of being inferred from the GUI's
version.

Repair respects component ownership:

- CEC Support updates through `cec-support-updater`.
- The shared node receives `request_update { minimum }`, so the process that
  owns the live backend performs and verifies its own replacement.
- MyOwnMesh and AMSTerm repair from CEC Support's pinned bundled sidecars; an
  installed support service is reinstalled/restarted when its payload changes.
- Crucible is re-materialized from its verified bundled portable archive.

Startup remains a separate policy: `while_granted`, `always`, or `off`, plus
whether closing the customer window leaves the app in the tray. Component
repair never silently changes that consent/startup policy.

## Toolbox execution boundary

The Toolbox is a dedicated Tauri window (`?toolbox=1`), created and foregrounded
by Rust rather than by a general-purpose webview window permission. The UI can
submit only a fixed `ToolboxAction`; `toolbox_spec` maps that id to one known
repair or Windows program. No arbitrary command string crosses the webview
boundary.

SFC, DISM, online CHKDSK, DNS flush, and the administrator-only Windows tools
launch through the bundled `amst.exe --admin --run` path in a real visible
console. A PowerShell wrapper mirrors stdout/stderr to a per-run UTF-8
transcript, which the backend tails into `toolbox://progress` events. Each run
has an independently validated id and child process, so long repairs update
their own progress cards without blocking other Toolbox actions.

The default surface contains repeatable checks plus Event Viewer, Device
Manager, Services, System Information, Task Manager, and Windows Settings.
**Show Advanced** reveals configuration-changing tools: Control Panel, Registry
Editor, Disk/Computer Management, System Configuration, Windows Features,
Resource Monitor, and Crucible Tests.

Crucible ships as the upstream portable zip, not a loose executable. The build
pins `.crucible-rev` and its SHA-256; runtime extraction rejects unsafe archive
paths and verifies the executable, PresentMon, and LibreHardwareMonitor tree.
The complete payload is atomically materialized under
`CEC_SUPPORT_HOME/tools/crucible/<version>` and launched through the same
visible administrator-terminal path. The archive marker makes the operation
idempotent and gives the Updates tab a targeted repair surface.

## Persistent state

Mesh state lives in the shared `~/.myownmesh` home (`MYOWNMESH_HOME`):

- `.secrets/identity.json` (0600) — the machine's ed25519 key; the Support
  number is derived from it (one identity per machine, shared with AllMyStuff).
- the consent store (0600) — persistent grants (3-hours + Forever), written
  atomically; a corrupt file is quarantined, never fatal.
- Transient "currently reachable / in a session" state is re-asserted each run,
  never persisted — so a machine is never silently reachable across reboots
  unless the customer left grant-scoped autostart on (the default, active only
  while a technician grant is live) or installed the background service.

CEC's own app files (service state, logs, and verified tool payloads) live under
`CEC_SUPPORT_HOME`
(default e.g. `%LOCALAPPDATA%\CEC Support` on Windows).

## Crate / component map

| Component | Repo | Status |
|---|---|---|
| `allmystuff-cec-protocol` — wire contract, Support ID, shared support area | AllMyStuff | ✅ implemented + tested |
| `allmystuff-cec-consent` — Once/3h/Forever store | AllMyStuff | ✅ implemented + tested |
| `NetworkKind::Silent` + `connect_peer` | MyOwnMesh | ✅ shipped |
| `SignalingConfig::listen_only` — the watcher's lurk join | MyOwnMesh | ✅ shipped |
| node "CEC mode" + technician secret tab (dialed customers are ordinary graph peers) | AllMyStuff | ✅ shipped |
| app-wide "Forget this node" on every node's gear | AllMyStuff | ✅ shipped |
| `cec-support-service` — the client's own service installer | CECSupport | ✅ implemented + tested |
| client GUI (`gui/`) + `cec-support` binary + installers | CECSupport | ✅ shipped |
| website | support.cec.direct | ✅ shipped |

See [ROADMAP.md](docs/ROADMAP.md) for exactly what is compiled-and-tested versus
what still needs the full media toolchain / a Windows box / a running mesh to
verify at runtime, and the cross-repo build order.
