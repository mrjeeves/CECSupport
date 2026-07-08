#!/bin/sh
# CEC Support end-user installer (macOS / Linux — Windows is the primary
# target; see install.ps1).
#
# Installs the `cec-support` client (one exe that is both the app and its
# CLI/service verbs), then makes sure the mesh pieces it reuses from AllMyStuff
# are in place WITHOUT clobbering an existing AllMyStuff install:
#
#   * The node binary (`allmystuff-serve`) and the `myownmesh` daemon are
#     REUSED when a machine already has them and they're new enough; otherwise
#     the versions bundled with this release are installed next to the client
#     ("reuse, don't clobber"). The myownmesh "new enough" bar is the version
#     pinned in `.myownmesh-rev`, exactly like AllMyStuff's installer.
#
# Pass --service to also install the background service (so CEC Support can
# reconnect after reboots during a repair). Mesh trouble never fails the
# install — the app still opens and shows the customer's number.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/mrjeeves/CECSupport/main/scripts/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/mrjeeves/CECSupport/main/scripts/install.sh | sh -s -- --service
#   ./scripts/install.sh --dry-run
#
# POSIX sh (dash / ash / bash). No bash-only constructs.

set -eu
if (set -o pipefail) 2>/dev/null; then set -o pipefail; fi

REPO="${CEC_SUPPORT_REPO:-mrjeeves/CECSupport}"
MESH_REPO="${MYOWNMESH_REPO:-mrjeeves/MyOwnMesh}"
DRY_RUN=false
INSTALL_SERVICE=false
FORCE_BUNDLE=false
PREFIX_DIR="${CEC_SUPPORT_PREFIX:-}"

for arg in "$@"; do
  case "$arg" in
    --dry-run)      DRY_RUN=true ;;
    --service)      INSTALL_SERVICE=true ;;
    --force-bundle) FORCE_BUNDLE=true ;;
    --prefix=*)     PREFIX_DIR="${arg#*=}" ;;
    *) ;;
  esac
done

log()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!!\033[0m %s\n' "$*" >&2; }
err()  { printf '\033[1;31mxxx\033[0m %s\n' "$*" >&2; }

OS_RAW="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$OS_RAW" in
  darwin) OS="macos" ;;
  linux)  OS="linux" ;;
  *)      OS="$OS_RAW" ;;
esac
ARCH_RAW="$(uname -m)"
case "$ARCH_RAW" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *)             ARCH="$ARCH_RAW" ;;
esac
ASSET="cec-support-${OS}-${ARCH}.tar.gz"
NODE_ASSET="allmystuff-serve-${OS}-${ARCH}.tar.gz"
MESH_ASSET="myownmesh-${OS}-${ARCH}.tar.gz"

if [ -z "$PREFIX_DIR" ]; then
  if [ -w /usr/local/bin ] || sudo -n true 2>/dev/null; then
    PREFIX_DIR="/usr/local/bin"
  else
    PREFIX_DIR="$HOME/.local/bin"
  fi
fi

install_binary() {
  src="$1"; name="$2"
  mkdir -p "$PREFIX_DIR" 2>/dev/null || sudo mkdir -p "$PREFIX_DIR"
  if [ -w "$PREFIX_DIR" ]; then install -m 0755 "$src" "$PREFIX_DIR/$name"
  else sudo install -m 0755 "$src" "$PREFIX_DIR/$name"; fi
  log "Installed: $PREFIX_DIR/$name"
}

# Download $1 to $2 and SHA-256-verify against $1.sha256. $3="fatal" aborts on a
# mismatch (the client); otherwise a mismatch just returns non-zero (a bundled
# extra). A missing sidecar downgrades to a warning.
get_verified() {
  url="$1"; dest="$2"; fatal="$3"
  curl -fsSL "$url" -o "$dest" || return 1
  if curl -fsSL "$url.sha256" -o "$dest.sha256" 2>/dev/null; then
    if ! (cd "$(dirname "$dest")" && (sha256sum -c "$(basename "$dest").sha256" 2>/dev/null || shasum -a 256 -c "$(basename "$dest").sha256")); then
      if [ "$fatal" = "fatal" ]; then err "SHA256 verification failed for $(basename "$url") — not installing it."; exit 1; fi
      warn "SHA256 verification failed for $(basename "$url") — skipping it."
      return 1
    fi
  else
    warn "No SHA256 sidecar for $(basename "$url"); skipping integrity check."
  fi
  return 0
}

# Print the download URL for $2 in repo $1's latest release, or nothing.
find_asset() {
  api="https://api.github.com/repos/$1/releases/latest"
  json="$(curl -fsSL "$api" 2>/dev/null)" || { warn "GitHub releases unreachable for $1."; return 0; }
  printf '%s' "$json" | grep -Eo "https://[^\"]+/$2" | head -n1 || true
}

# Install the client (cec-support) from the latest CECSupport release.
install_client() {
  if ! command -v curl >/dev/null 2>&1; then err "curl is required."; exit 1; fi
  url="$(find_asset "$REPO" "$ASSET")"
  if [ -z "$url" ]; then err "No release asset matched $ASSET in $REPO."; exit 1; fi
  log "Downloading $url"
  if [ "$DRY_RUN" = "true" ]; then log "(dry-run) would download and install $ASSET"; return 0; fi
  tmp="$(mktemp -d)"
  get_verified "$url" "$tmp/$ASSET" fatal
  tar -xzf "$tmp/$ASSET" -C "$tmp"
  install_binary "$tmp/cec-support" "cec-support"
  rm -rf "$tmp"
}

# Reuse an existing node if present; else install the bundled one.
ensure_node() {
  if [ "$FORCE_BUNDLE" != "true" ]; then
    for cand in "$PREFIX_DIR/allmystuff-serve" "$(command -v allmystuff-serve 2>/dev/null || true)"; do
      if [ -n "$cand" ] && [ -x "$cand" ]; then
        log "Node: reusing the existing allmystuff-serve at $cand (not clobbering it)."
        return 0
      fi
    done
  fi
  if [ "$DRY_RUN" = "true" ]; then log "(dry-run) would install the bundled node ($NODE_ASSET)"; return 0; fi
  url="$(find_asset "$REPO" "$NODE_ASSET")"
  if [ -z "$url" ]; then
    warn "Node: no bundled $NODE_ASSET in $REPO. The app still opens; live help needs the"
    warn "AllMyStuff node — install AllMyStuff, or re-run this installer later."
    return 0
  fi
  log "Node: no existing allmystuff-serve found — installing the bundled one…"
  log "Downloading $url"
  tmp="$(mktemp -d)"
  if get_verified "$url" "$tmp/$NODE_ASSET" nonfatal; then
    tar -xzf "$tmp/$NODE_ASSET" -C "$tmp" && install_binary "$tmp/allmystuff-serve" "allmystuff-serve"
  fi
  rm -rf "$tmp"
}

# --- myownmesh daemon: same reuse-or-bundle rules as AllMyStuff's installer ---

version_ge() {
  IFS=. read -r a1 a2 a3 <<EOF
$1
EOF
  IFS=. read -r b1 b2 b3 <<EOF
$2
EOF
  a1="${a1%%[!0-9]*}"; a2="${a2%%[!0-9]*}"; a3="${a3%%[!0-9]*}"
  b1="${b1%%[!0-9]*}"; b2="${b2%%[!0-9]*}"; b3="${b3%%[!0-9]*}"
  a1="${a1:-0}"; a2="${a2:-0}"; a3="${a3:-0}"; b1="${b1:-0}"; b2="${b2:-0}"; b3="${b3:-0}"
  if [ "$a1" -ne "$b1" ]; then [ "$a1" -gt "$b1" ]; return; fi
  if [ "$a2" -ne "$b2" ]; then [ "$a2" -gt "$b2" ]; return; fi
  [ "$a3" -ge "$b3" ]
}

mesh_min_version() {
  rev=""
  if [ -f "$0" ] && [ -f "$(dirname "$0")/../.myownmesh-rev" ]; then
    rev="$(cat "$(dirname "$0")/../.myownmesh-rev" 2>/dev/null || true)"
  fi
  if [ -z "$rev" ] && command -v curl >/dev/null 2>&1; then
    rev="$(curl -fsSL "https://raw.githubusercontent.com/$REPO/main/.myownmesh-rev" 2>/dev/null || true)"
  fi
  rev="$(printf '%s' "$rev" | tr -d '[:space:]')"
  case "$rev" in v[0-9]*) printf '%s' "${rev#v}" ;; *) ;; esac
}

installed_mesh_version() {
  v="$("$1" --version 2>/dev/null | grep -Eo '[0-9]+\.[0-9]+(\.[0-9]+)?' | head -n1)"
  [ -z "$v" ] && v="$("$1" --version 2>&1 | grep -Eo '[0-9]+\.[0-9]+(\.[0-9]+)?' | head -n1)"
  printf '%s' "$v"
}

ensure_mesh() {
  existing=""
  if [ "$FORCE_BUNDLE" != "true" ]; then
    if [ -x "$PREFIX_DIR/myownmesh" ]; then existing="$PREFIX_DIR/myownmesh"
    elif command -v myownmesh >/dev/null 2>&1; then existing="$(command -v myownmesh)"; fi
  fi
  min="$(mesh_min_version || true)"

  if [ -n "$existing" ]; then
    ver="$(installed_mesh_version "$existing" || true)"
    if [ -n "$ver" ] && { [ -z "$min" ] || version_ge "$ver" "$min"; }; then
      log "Mesh: reusing the installed myownmesh v$ver at $existing${min:+ (needs v$min+)}."
      return 0
    fi
    [ -n "$ver" ] && log "Mesh: installed myownmesh is v$ver but this release wants v$min+." \
                  || log "Mesh: $existing didn't answer --version."
    if [ "$DRY_RUN" = "true" ]; then log "(dry-run) would ask it to update itself: myownmesh update"; return 0; fi
    log "Asking it to update itself (myownmesh update)…"
    "$existing" update || true
    ver="$(installed_mesh_version "$existing" || true)"
    if [ -n "$ver" ] && { [ -z "$min" ] || version_ge "$ver" "$min"; }; then log "Mesh: myownmesh is now v$ver."
    elif [ -z "$ver" ]; then log "Mesh: myownmesh responded to 'update' but reports no readable version; assuming it's fine."
    else warn "Mesh: couldn't bring myownmesh up to v${min:-a readable version}. The app still runs; retry later with: myownmesh update"; fi
    return 0
  fi

  if [ "$DRY_RUN" = "true" ]; then log "(dry-run) would install the bundled myownmesh daemon ($MESH_ASSET)"; return 0; fi
  log "Mesh: no myownmesh daemon found — installing it next to the client…"
  url="$(find_asset "$MESH_REPO" "$MESH_ASSET")"
  if [ -z "$url" ]; then
    warn "Mesh: no $MESH_ASSET in $MESH_REPO. The app still opens; for live help install MyOwnMesh:"
    warn "  curl -fsSL https://raw.githubusercontent.com/$MESH_REPO/main/scripts/install.sh | sh -s -- --no-gui"
    return 0
  fi
  log "Downloading $url"
  tmp="$(mktemp -d)"
  if get_verified "$url" "$tmp/$MESH_ASSET" nonfatal; then
    tar -xzf "$tmp/$MESH_ASSET" -C "$tmp" && install_binary "$tmp/myownmesh" "myownmesh"
  fi
  rm -rf "$tmp"
}

ensure_on_path() {
  case ":$PATH:" in *":$PREFIX_DIR:"*) return 0 ;; esac
  warn "$PREFIX_DIR is not on your PATH. Add it to your shell profile:"
  warn "  export PATH=\"$PREFIX_DIR:\$PATH\""
}

install_client
ensure_node
ensure_mesh
[ "$DRY_RUN" != "true" ] && ensure_on_path || true

if [ "$INSTALL_SERVICE" = "true" ]; then
  if [ "$DRY_RUN" = "true" ]; then
    log "(dry-run) would install the background service: cec-support service install"
  else
    log "Installing the background service (per-user)…"
    "$PREFIX_DIR/cec-support" service install || warn "Service install failed."
  fi
fi

log "Done."
log "Open CEC Support and read the big number to your CEC technician."
log "  cec-support        # open the app"
log "  cec-support id     # print your support number"
[ "$INSTALL_SERVICE" != "true" ] && log "  cec-support service install   # keep it connected across reboots"
