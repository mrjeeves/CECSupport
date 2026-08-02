#!/usr/bin/env sh
# Provision one CEC support-infrastructure box ("hub node"): a MyOwnMesh
# daemon hosting the signaling relay + TURN, with mesh membership off.
# Idempotent; run as root on a fresh Debian/Ubuntu-ish VM. See ../README.md
# for what these boxes are (and are not).
#
#   PUBLIC_IP=203.0.113.7 TURN_PASSWORD=... sh setup-hub.sh
#
# PUBLIC_IP     required — the routable address TURN hands out.
# TURN_USER     optional — TURN username (default "cec").
# TURN_PASSWORD required — TURN credential; mirror it into the client config.
# MYOWNMESH_BIN optional — path to a prebuilt `myownmesh` binary to install;
#               unset, the script expects `myownmesh` already on PATH.
set -eu

PUBLIC_IP="${PUBLIC_IP:?set PUBLIC_IP to this box's routable address}"
TURN_USER="${TURN_USER:-cec}"
TURN_PASSWORD="${TURN_PASSWORD:?set TURN_PASSWORD (and mirror it client-side)}"

HOME_DIR=/var/lib/cec-signaling
CONF_DIR="$HOME_DIR/.myownmesh"

# 1. The binary.
if [ -n "${MYOWNMESH_BIN:-}" ]; then
  install -m 0755 "$MYOWNMESH_BIN" /usr/local/bin/myownmesh
fi
command -v myownmesh >/dev/null 2>&1 || command -v /usr/local/bin/myownmesh >/dev/null 2>&1 || {
  echo "myownmesh binary not found — set MYOWNMESH_BIN or install it on PATH" >&2
  exit 1
}

# 2. A dedicated system user + state dir.
id cec-signaling >/dev/null 2>&1 || useradd --system --home "$HOME_DIR" --shell /usr/sbin/nologin cec-signaling
mkdir -p "$CONF_DIR"

# 3. Pure-infrastructure config: services on, mesh membership off. Kept
#    verbatim if one already exists (edits survive re-runs).
if [ ! -f "$CONF_DIR/config.json" ]; then
  cat >"$CONF_DIR/config.json" <<EOF
{
  "version": 1,
  "services": {
    "node":      { "enabled": false },
    "signaling": { "enabled": true, "bind": "0.0.0.0", "port": 4848 },
    "turn": {
      "enabled": true,
      "bind": "0.0.0.0",
      "port": 3478,
      "public_ip": "$PUBLIC_IP",
      "realm": "cecsupport",
      "credentials": [ { "username": "$TURN_USER", "password": "$TURN_PASSWORD" } ],
      "relay_port_min": 49152,
      "relay_port_max": 65535,
      "max_bps_per_connection": 0
    }
  },
  "networks": []
}
EOF
fi
chown -R cec-signaling:cec-signaling "$HOME_DIR"
chmod 700 "$CONF_DIR"

# 4. Firewall (ufw when present; otherwise print the checklist). The cloud
#    security group must mirror this — the host firewall alone is not enough.
if command -v ufw >/dev/null 2>&1; then
  ufw allow 4848/tcp
  ufw allow 3478/udp
  ufw allow 49152:65535/udp
else
  echo "open at the firewall + provider security group: tcp 4848, udp 3478, udp 49152-65535"
fi

# 5. The service unit.
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install -m 0644 "$SCRIPT_DIR/../systemd/cec-signaling.service" /etc/systemd/system/cec-signaling.service
systemctl daemon-reload
systemctl enable --now cec-signaling

sleep 2
systemctl --no-pager status cec-signaling || true
echo
echo "Next: front ws://127.0.0.1:4848 with TLS (wss://…), then add this box to"
echo "the client config lists — see infra/README.md 'Pointing the CEC apps at"
echo "the hubs'. TURN credential to mirror client-side: $TURN_USER / (the one you set)"
