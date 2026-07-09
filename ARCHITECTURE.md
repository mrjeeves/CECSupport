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
│  install-as-service. Embeds the AllMyStuff node in "CEC client mode".  │
└───────────────▲──────────────────────────────────────────────────────┘
                │ reuses (git dependency)
┌───────────────┴──────────────────────────────────────────────────────┐
│  AllMyStuff  (the shared engine + the technician app)                  │
│  • node engine: screen capture / input inject / sessions / graph       │
│  • crates/allmystuff-cec-protocol   the CEC wire contract + Support ID  │
│  • crates/allmystuff-cec-consent    Once / 3-hours / Forever grants     │
│  • GUI: a secret "CEC Support" tab (Agent Name + Customer number →      │
│    dial); dialed customers show as ordinary graph peers (no fleet       │
│    group — the CEC mesh is Silent, with no roster)                      │
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

## The "Silent" mesh, named after the number

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

On top of that, the **`network_id` is derived from the customer's number**
(`allmystuff_cec_protocol::network_id_for_number`, e.g. number `123456789` →
room `cec-123456789`). The consequences are the whole trust model:

- Each customer effectively has **their own secret room**. A technician who
  hasn't been told the number can't even compute the `network_id`, so they can't
  signal, discover, or reach that customer at all.
- There is **no shared lobby** and therefore **no N² connection fan-out**: a
  customer's room only ever contains that customer plus whoever they handed the
  number to.
- The number is derived deterministically from the device's public key
  (`support_id_from_device`), so the customer and technician independently
  compute the same room from the same number — **no directory, no server**.

### Two layers of access control

1. **Discovery gate — the number.** Knowing the number lets you *find and
   signal* the customer (join their Silent room). It is communicated out of band
   (the customer reads it to the technician on the phone). ~40 bits of entropy:
   enough that you can't stumble into a customer's room, not a cryptographic
   secret on its own.
2. **Access gate — the approval.** Discovery is not access. Even inside the
   room, nothing happens until the customer **approves** the technician (below).
   Screen and control frames are authorised per-frame against the customer's
   consent store, so a revoke stops the stream immediately.

This is exactly the AnyDesk shape ("here's my ID" → "allow this session"), but
the ID doubles as the private rendezvous and there is no server in the middle.

## The connect flow

```
Customer (CEC Support app)                 Technician (AllMyStuff + CEC tab)
──────────────────────────                 ────────────────────────────────
launch → identity → number N
join Silent mesh room = f(N)
advertise presence, wait
                                           told N on the phone; sets Agent Name
                                           join Silent mesh room = f(N)
                                           see the one customer peer (Sighted)
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
combined with *install-as-service*, CEC can reconnect after a reboot without
anyone at the keyboard — bounded by the 3-hour window unless the customer chose
Forever, and revocable from the customer side at any time.

## Reuse, don't clobber

Like the MyOwnMesh installer, the CEC Support client **reuses an existing
AllMyStuff / `myownmesh` install if one is present and new enough** (shares the
daemon), and **bundles its own** binaries otherwise, so a customer who has never
heard of AllMyStuff still gets a self-contained app. Its background service uses
its **own** identity (`cec-support-service`, service name `CECSupport`) so it
never disturbs an AllMyStuff service on the same machine.

## Isolation from other MyOwnMesh ecosystems

CEC Support forks MyOwnMesh's signing tags and home dir (see
`allmystuff-cec-protocol`) so its signatures never cross-verify against an
AllMyStuff / MyOwnMesh / MyOwnLLM mesh and its identity + state never collide
with an existing install. It deliberately does **not** fork the signaling
app-id: each support session is already isolated by its per-number `network_id`
(`cec-<number>`), which seeds a distinct room handle, so technician and customer
meet on the default app-id with no env override.

- signing domain tags `cec-support-mesh-auth-v1:` / `cec-support-network-state-v1:`.
- home dir via `CEC_SUPPORT_HOME` (a `MYOWNMESH_HOME` override), so identity +
  state never collide with an existing AllMyStuff install.

## Persistent state

Under `CEC_SUPPORT_HOME` (default a CEC-specific dir, e.g.
`%LOCALAPPDATA%\CEC Support` on Windows):

- `.secrets/identity.json` (0600) — the device ed25519 key; the number is
  derived from it.
- the consent store (0600) — persistent grants (3-hours + Forever), written
  atomically; a corrupt file is quarantined, never fatal.
- Transient "currently reachable / in a session" state is re-asserted each run,
  never persisted — so a machine is never silently reachable across reboots
  unless the customer installed the service.

## Crate / component map

| Component | Repo | Status |
|---|---|---|
| `allmystuff-cec-protocol` — wire contract, Support ID, per-number room | AllMyStuff | ✅ implemented + tested |
| `allmystuff-cec-consent` — Once/3h/Forever store | AllMyStuff | ✅ implemented + tested |
| `NetworkKind::Silent` + `connect_peer` | MyOwnMesh | 🚧 the one substrate addition |
| node "CEC mode" + technician secret tab (dialed customers are ordinary graph peers) | AllMyStuff | 🚧 |
| app-wide "Forget this node" on every node's gear | AllMyStuff | 🚧 |
| `cec-support-service` — the client's own service installer | CECSupport | ✅ implemented + tested |
| client GUI (`gui/`) + `cec-support` binary + installers | CECSupport | 🚧 |
| website | support.cec.direct | 🚧 |

See [ROADMAP.md](docs/ROADMAP.md) for exactly what is compiled-and-tested versus
what still needs the full media toolchain / a Windows box / a running mesh to
finalise, and the cross-repo build order.
