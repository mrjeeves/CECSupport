# CEC Support — Architecture

CEC Support is Critical Error Computing's **one-tap remote help desk**: a
customer runs a tiny app, reads out a short number, and a CEC technician can
then view and control their screen to fix things — like AnyDesk, but over a
private peer-to-peer mesh with **no central server holding a directory or the
session data**, and with access that the customer grants per-session or
generally and can revoke at any time.

It is not a new networking stack. It is a thin, focused **re-scope of
AllMyStuff's remote-control console**, layered on the same substrate:

```
┌──────────────────────────────────────────────────────────────────────┐
│  CEC Support client app  (this repo)                                   │
│  Windows-first customer GUI: show my number · approve/deny · revoke ·  │
│  reconnect on boot. Embeds the AllMyStuff node in "CEC client mode".   │
└───────────────▲──────────────────────────────────────────────────────┘
                │ reuses (git dependency)
┌───────────────┴──────────────────────────────────────────────────────┐
│  AllMyStuff  (the shared engine + the technician app)                  │
│  • node engine: screen capture / input inject / sessions / graph       │
│  • crates/allmystuff-cec-protocol   the CEC wire contract + Support ID  │
│  • crates/allmystuff-cec-consent    Once / 3-hours / Forever grants     │
│  • GUI: a secret "CEC Support" tab (Agent Name + raised-hand queue →   │
│    answer, or type the number as a fallback); dialed customers show    │
│    as ordinary graph peers (no fleet group — CEC is Silent, no roster) │
└───────────────▲──────────────────────────────────────────────────────┘
                │ embeds
┌───────────────┴──────────────────────────────────────────────────────┐
│  MyOwnMesh  (the peer-to-peer substrate)                               │
│  identity · mutual ed25519 auth · WebRTC transport · signaling ·       │
│  + a new network type: **Silent** (no auto-connect, no gossip)         │
└──────────────────────────────────────────────────────────────────────┘
```

Why this split, and not one monolith: AllMyStuff is already the remote-control
engine and the technician's app; putting the CEC logic there means the
technician side and the customer side share **one** implementation of the wire
protocol, the consent rules, and the media planes. The customer's app is then a
small, calm, single-purpose GUI on top. MyOwnMesh stays a general substrate —
its only CEC-related addition is the reusable `Silent` network type.

## The "Silent" mesh — one shared support area

An ordinary MyOwnMesh network is always-on: every co-present peer auto-dials
every other and the roster gossips. That is wrong for a help desk — you don't
want to be connected to strangers just because you launched an app. So CEC
Support uses a new MyOwnMesh network type, **`Silent`**:

- **No auto-connect.** A Silent node dials *nobody* on its own. Peers are merely
  *discoverable* (signaling presence); a WebRTC connection forms only when
  someone calls `connect_peer` for a specific peer.
- **No gossip.** A Silent network never broadcasts a roster; membership is never
  shared. Each connection is deliberate.
- **Open, never closed.** A mesh that auto-accepts members can't be governed by a
  signed roster, so `Silent` is inherently open; access control happens
  out-of-band (below), not via network governance.

On top of that, **every CEC node — customers and technicians alike — lives on
one well-known shared area**: the constant
`allmystuff_cec_protocol::HELP_NETWORK_ID` (`cecsupport-clients`), shaped into a
hub topology by `CEC_HELP_HUBS`. There is no per-customer room; the hub shape is
what makes one shared area safe:

- Customers connect **only to CEC-operated infra hubs**, never to each other. On
  the area a customer dials nobody and sees nobody — the Silent rules above mean
  no auto-connect, no gossip, no roster — so there is no N² connection fan-out.
- A technician on the area reaches a customer by **deliberately dialing that one
  device**: they answer a raised hand (a `SupportPresence` beacon) by
  `connect_peer`-ing that customer's device, or — as a fallback when the
  raised-hand queue is too crowded to pick someone out — type the number and let
  the node resolve those digits to a device on the area.
- The number (`support_id_from_device`, 9 digits like `123 456 789`) is a
  **display / verification label and a dial fallback — not a network id, room
  key, or rendezvous secret**. It is derived deterministically from the device's
  public key, so the customer can read it out to confirm identity, but knowing it
  puts no one in a private room — there is no per-number room to enter.

### Two layers of access control

1. **Discovery gate — being a technician who dials you.** Reaching a customer at
   all means being a technician on the shared support area and **deliberately
   dialing that one device**. Customers are non-connectable to one another; the
   area is hub-mediated and Silent — no auto-connect, no gossip, no roster — so
   nothing reaches a customer until a technician picks their device (from a
   raised hand, or the number as a fallback) and dials it.
2. **Access gate — the approval.** Discovery is not access. Even once a
   technician has dialed in, nothing happens until the customer **approves** them
   (below). Screen and control frames are authorised per-frame against the
   customer's consent store, so a revoke stops the stream immediately.

This is the AnyDesk shape ("here's my ID" → "allow this session"), with no
central directory or session server in the middle — but the ID is only a label
the customer reads out to be found and verified, not a private rendezvous:
reach comes from a technician on the shared area dialing the device directly.

## The connect flow

```
Customer (CEC Support app)                 Technician (AllMyStuff + CEC tab)
──────────────────────────                 ────────────────────────────────
launch → identity → number N
join shared support area (cecsupport-clients)
Ask for help → raise hand (SupportPresence), wait
                                           already on the shared support area
                                           sees the raised hand in the queue
                                           answers it (or types N as a fallback)
                                           connect_peer(customer)         ─┐
   ◀───────────── inbound offer + connect-request (agent_name) ───────────┘
prompt: "‹Agent Name› is trying to
 connect" + 6-digit verification code
 [Approve Once] [3 hours] [Forever] [Deny]
   │
   ├─ approve(scope) → consent grant + mesh approve → session goes Active
   │                                                  customer appears on the
   │                                                  graph as a normal peer
   ├─ screen frames ─────────────────────────────────▶ view
   │  ◀──────────────────────────────── input events (if control granted)
   │  (every frame re-checked against the grant)
   └─ Revoke / "Forget this technician" → teardown, immediately
```

The 6-digit verification code and the Agent Name let the customer confirm *who*
they're letting in before approving — the human check on top of the ed25519
mutual authentication MyOwnMesh already performs.

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
comes from the shared area being Silent and hub-mediated (customers reach only
CEC infra hubs, never each other) plus the customer's per-frame consent — not
from siloing the apps.

`CEC_SUPPORT_HOME` holds only CEC's **own app files** (service state, logs);
the mesh stack's home stays the shared `~/.myownmesh`.

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

CEC's own app files (service state, logs) live under `CEC_SUPPORT_HOME`
(default e.g. `%LOCALAPPDATA%\CEC Support` on Windows).

## Crate / component map

| Component | Repo | Status |
|---|---|---|
| `allmystuff-cec-protocol` — wire contract, Support ID, shared support area | AllMyStuff | ✅ implemented + tested |
| `allmystuff-cec-consent` — Once/3h/Forever store | AllMyStuff | ✅ implemented + tested |
| `NetworkKind::Silent` + `connect_peer` | MyOwnMesh | ✅ shipped (the one substrate addition) |
| node "CEC mode" + technician secret tab (dialed customers are ordinary graph peers) | AllMyStuff | ✅ shipped |
| app-wide "Forget this node" on every node's gear | AllMyStuff | ✅ shipped |
| `cec-support-service` — the client's own service installer | CECSupport | ✅ implemented + tested |
| client GUI (`gui/`) + `cec-support` binary + installers | CECSupport | ✅ shipped |
| website | support.cec.direct | ✅ shipped |

See [ROADMAP.md](docs/ROADMAP.md) for exactly what is compiled-and-tested versus
what still needs the full media toolchain / a Windows box / a running mesh to
verify at runtime, and the cross-repo build order.
