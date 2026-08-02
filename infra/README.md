# CEC support infrastructure — the "hub nodes"

The hub nodes are **signaling and NAT-traversal infrastructure, nothing
else**. MyOwnMesh is a mesh *signaling* system for direct WebRTC
peer-to-peer connections: customers and technicians meet in a signaling
room, and every actual session is a direct WebRTC link between exactly two
machines. Nothing routes data or media through CEC — the only relay that
can ever touch session traffic is the TURN server, and only as WebRTC's
own last-resort ICE fallback when NAT rules out a direct pair (the frames
are DTLS-encrypted; TURN sees ciphertext).

So "standing up the hubs" means running two services, both shipped by
[MyOwnMesh](https://github.com/mrjeeves/MyOwnMesh) in the `myownmesh`
binary (see its `docs/SERVICES.md` for the full reference):

| Service | What it carries | Port |
|---|---|---|
| **Signaling relay** — an intelligent NIP-01 (Nostr/WebSocket) room server | Presence announces (who is on the support area / in the asking queue), SDP offer/answer/ICE for dials. Metadata only, never session data. | `tcp 4848` (put wss/TLS in front) |
| **TURN** (answers STUN too) | Relayed WebRTC allocations for symmetric-NAT customers — encrypted session bytes, as ICE fallback only. | `udp 3478` + a relay port range |

Without CEC-operated instances, the mesh rides the public-relay defaults
and the reference STUN/TURN. That works, but CEC-operated boxes give the
help desk dedicated capacity, no third-party dependency, and instant
departure notices (the smart relay synthesizes `leave` the moment a
customer's socket closes, so the queue drops a withdrawn hand immediately
instead of after the announce timeout).

## What the CEC rooms are (context for operators)

- `cecsupport-clients` — the standing support area. Every CEC install is
  a **Silent** resident: present in the signaling room, connected to
  nobody. A technician's pinned redial finds a rebooted customer here,
  and a phoned-in number resolves against the room's member list.
- `cecsupport-asking` — the queue. A customer joins it only while their
  hand is up; membership *is* the ask. Technicians watch it with a
  **listen-only** join (they read the room without announcing into it).

Both rooms are just namespaces on the signaling relay — the relay needs
no CEC-specific configuration, and one relay instance serves both (plus
anything else that points at it).

## Provisioning a hub box

Any small always-on VM per region works; signaling is tiny (text frames,
token-bucket flood limits), TURN is the only bandwidth consumer. Two
boxes in different failure domains are plenty to start: peers accept a
*list* of relays and use them redundantly.

1. **Install the `myownmesh` binary** (from the MyOwnMesh releases, or
   `cargo install --path crates/myownmesh` from a checkout).

2. **Configure a pure-infrastructure daemon** — services on, mesh
   membership off — in `~/.myownmesh/config.json`:

   ```json
   {
     "version": 1,
     "services": {
       "node":      { "enabled": false },
       "signaling": { "enabled": true, "bind": "0.0.0.0", "port": 4848 },
       "turn": {
         "enabled": true,
         "bind": "0.0.0.0",
         "port": 3478,
         "public_ip": "<this box's public IP>",
         "realm": "cecsupport",
         "credentials": [ { "username": "cec", "password": "<generate>" } ],
         "relay_port_min": 49152,
         "relay_port_max": 65535,
         "max_bps_per_connection": 0
       }
     },
     "networks": []
   }
   ```

   Notes:
   - `node: false` = the box joins no meshes; it is pure plumbing.
   - TURN **requires** `public_ip` and at least one credential, and the
     same credential pair goes into the client config below.
   - TURN also answers STUN on 3478 — no separate STUN service needed.
   - The signaling flood limits default sane; tune under
     `services.signaling.limits` if the queue ever gets loud.

3. **Open the firewall** (host **and** cloud security group):

   ```sh
   sudo ufw allow 4848/tcp          # signaling (or 443 if TLS-fronted)
   sudo ufw allow 3478/udp          # TURN control
   sudo ufw allow 49152:65535/udp   # TURN relay allocations (the pinned range)
   ```

   The classic failure is opening only the control port and then seeing
   `0 srflx · 0 relay` candidates on every client.

4. **Front the signaling relay with TLS** (customers sit behind
   corporate proxies that only pass 443): any TLS terminator → 
   `wss://signal-1.cecsupport.example` → `ws://127.0.0.1:4848`.

5. **Run it as a service** — `systemd/cec-signaling.service` in this
   directory; `scripts/setup-hub.sh` does steps 1–3 and installs it.

6. **Verify**:

   ```sh
   myownmesh ctl services status      # both listeners "running"
   ```

   and from a laptop, point a scratch network's signaling at the new
   relay and watch two peers discover each other (MyOwnMesh
   `docs/SERVICES.md` → *Pointing peers at your services*).

## Pointing the CEC apps at the hubs

Client-side, the rooms' configs are built in AllMyStuff
`node/src/cec.rs` (`help_network_config` / `ask_network_config`) and
mirrored in the NanoKVM bridges (`server/service/mesh/cec.go`). Today
they ride the built-in public-relay defaults and the reference
STUN/TURN. When CEC's own boxes are up, add to those builders:

```jsonc
"signaling": {
  "strategy": "nostr",
  "mdns": true,
  "servers": ["wss://signal-1.cecsupport.example", "wss://signal-2.cecsupport.example"]
},
"turn_servers": [{
  "urls": ["turn:turn-1.cecsupport.example:3478"],
  "username": "cec", "credential": "<the credential>"
}]
```

and ship a release. Existing installs converge without reinstalling:
every `cec_online` / dial / watch re-pushes the room config to the
daemon (`NetworkUpdate`), and a signaling change triggers the daemon's
leave-and-rejoin, so the fleet migrates onto CEC relays as the apps
update. Keep the defaults as fallback during the transition — the
signaling driver treats the list as a redundant set.

## What these boxes can and cannot see

Worth stating plainly, since it's the whole point of the design:

- The signaling relay sees room membership (device ids and derived
  support numbers), announce timing, and encrypted-ish SDP metadata. It
  never sees screens, input, files, or chat — those ride direct WebRTC.
- TURN sees packet sizes/timing of the sessions that fall back to it,
  all DTLS ciphertext.
- Neither holds a customer directory: the dialed directory, consent
  grants, and chat history live on the technician's and customer's own
  machines.
