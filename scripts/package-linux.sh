#!/usr/bin/env bash
# The native portable/self-update archive. Debian and AppImage remain the
# customer installation formats; this archive also carries the standalone
# sidecars needed to run an extracted portable copy.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:?usage: package-linux.sh <Rust target triple>}"
case "$TARGET" in
  x86_64-unknown-linux-gnu) PLATFORM=linux-x86_64 ;;
  aarch64-unknown-linux-gnu) PLATFORM=linux-aarch64 ;;
  *) echo "unsupported Linux target: $TARGET" >&2; exit 1 ;;
esac

OUT="$ROOT/dist-bin"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$OUT"

for binary in cec-support myownmesh allmystuff-serve amst; do
  if [[ "$binary" == cec-support ]]; then
    source="$ROOT/gui/src-tauri/target/$TARGET/release/$binary"
  else
    source="$ROOT/gui/src-tauri/binaries/$binary-$TARGET"
  fi
  [[ -s "$source" && -x "$source" ]] || { echo "missing executable: $source" >&2; exit 1; }
  install -m 755 "$source" "$STAGE/$binary"
done

NAME="cec-support-$PLATFORM.tar.gz"
tar -czf "$OUT/$NAME" -C "$STAGE" cec-support myownmesh allmystuff-serve amst
(cd "$OUT" && sha256sum "$NAME" > "$NAME.sha256")
