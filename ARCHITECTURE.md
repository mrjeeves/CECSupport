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

## The diagnostic purchase — money without a middleman server

A technician can require the **$50 diagnostic session** at any point in a help
call: **before answering** (quoting the work from the help queue),
**mid-session**, or **after disconnecting** (the Diag control on a stored
machine). It is optional and **technician-triggered only**; the customer's app
never initiates a purchase. The design keeps every architectural promise:

```
Technician (AllMyStuff CEC tab)            Customer (CEC Support app)
───────────────────────────────            ──────────────────────────
"Request $50 diagnostic"
 (help queue · live session · stored
  machine — connected or not: the node
  joins their number room like a dial
  and re-sends until answered)
  PurchaseControl::Request ───────────────▶ prompt: "‹Agent› is asking you to
                                             complete a purchase — $50"
  ◀─────────────── Status: seen             [Open secure checkout]
                                              └─▶ default browser:
                                                  support.cec.direct/buy/diagnostic/
                                                    ?sn=‹number›&ref=…&agent=…
                                                  → store's hosted checkout
  ◀─────────────── Status: opened           (pays in their own browser)
  ◀─────────────── Status: claimed          "I've completed my purchase"
verify the order in the store admin
(it carries the Support Number)
  PurchaseControl::Confirm ────────────────▶ "You're all set" ✓
```

- **No payment data on the mesh, ever.** The wire
  (`allmystuff-cec-protocol::PurchaseControl`) carries ids and display strings
  only. Payment happens on the store's hosted checkout in the customer's own
  browser — real address bar, their password manager, an emailed receipt.
- **The checkout URL is never taken from the wire.** The app opens its own
  built-in `DIAGNOSTIC_BUY_URL` and appends only attribution (support number,
  reference, agent name) — so nobody who can reach the customer's room can
  steer their browser anywhere else. The Tauri `open_url` command refuses
  anything but that page.
- **The ask carries the connect prompt's trust, deliberately.** A `Request`
  needs no prior grant — a technician quotes the diagnostic *before* the
  customer has ever let them in — so its gate is the same as "‹Agent› is
  trying to connect": reaching the room took the number (told out of band, on
  the phone), the prompt names the asker, the customer verifies that name
  against the person they're talking to and can always decline. When a grant
  *does* exist, the prompt prefers the grant's name — the one the customer
  actually approved — over the wire's claim. And whatever anyone does, the
  buttons only ever lead to CEC's own checkout.
- **Human confirmation instead of webhooks.** The order lands in the store
  tagged with the customer's support number; the technician (on the phone,
  already in the store admin) verifies it and sends `Confirm`. No server, no
  webhook receiver, no polling — the same person-checks-person shape as the
  agent-name verification. `Claimed` is a claim; the store order is the truth.
- **Ephemeral by design.** Purchase asks live in memory (one live ask per
  peer — a re-ask supersedes; settled by Confirm/Cancel; a session's ask is
  auto-cancelled when that session ends). The durable record of payment is the
  store's order — the mesh keeps no financial state.
- **Skew-safe.** An older customer app decodes the whole purchase envelope to
  `Unknown` and ignores it; no `seen` coming back tells the technician to
  handle payment by phone instead.

The store configuration (domain + product variant) lives behind
`support.cec.direct/buy/diagnostic/`, so prices, products, or even the store
itself can change without shipping a new installer.

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
comes from the number-derived `network_id` (`cec-<number>`), which seeds each
support session's own room handle — not from siloing the apps.

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
  unless the customer installed the service.

CEC's own app files (service state, logs) live under `CEC_SUPPORT_HOME`
(default e.g. `%LOCALAPPDATA%\CEC Support` on Windows).

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
