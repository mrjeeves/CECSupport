#!/usr/bin/env bash
# Resolve the Tauri dependency graph from the suite pins and prove the lock
# agrees.  AllMyStuff is both a Rust git dependency and a bundled sidecar, so a
# forward pin cannot be locked until that upstream tag exists.  That is fine
# while preparing a PR; `just release` runs this script and now fails loudly
# instead of silently releasing the previous locked AllMyStuff graph.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
AMS_PIN="$(tr -d '[:space:]' < "$ROOT/.allmystuff-rev")"
MESH_PIN="$(tr -d '[:space:]' < "$ROOT/.myownmesh-rev")"
MANIFEST="$ROOT/gui/src-tauri/Cargo.toml"
LOCK="$ROOT/gui/src-tauri/Cargo.lock"
MODE="${1:-sync}"

fail() { echo "error: $*" >&2; exit 1; }

case "$MODE" in
  sync|--check) ;;
  *) fail "usage: $0 [--check]" ;;
esac

for pair in "AllMyStuff:$AMS_PIN" "MyOwnMesh:$MESH_PIN"; do
  name="${pair%%:*}"; pin="${pair#*:}"
  [[ "$pin" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-.][A-Za-z0-9.-]+)?$ ]] \
    || fail ".$(echo "$name" | tr '[:upper:]' '[:lower:]')-rev contains invalid release pin '$pin'"
done

for package in allmystuff-node allmystuff-cec-protocol allmystuff-cec-consent; do
  grep -Eq "^${package} = .*tag = \"${AMS_PIN}\"" "$MANIFEST" \
    || fail "$MANIFEST does not pin $package to .allmystuff-rev ($AMS_PIN)"
done

lock_matches_pin() {
  [[ -f "$LOCK" ]] || return 1
  grep -q 'source = "git+https://github.com/mrjeeves/AllMyStuff?tag=' "$LOCK" \
    && ! grep 'source = "git+https://github.com/mrjeeves/AllMyStuff?tag=' "$LOCK" \
      | grep -Fvq "?tag=${AMS_PIN}#"
}

if [[ "$MODE" == "sync" ]] && ! lock_matches_pin; then
  # A normal resolution updates the entire tagged AllMyStuff source as one
  # unit. It intentionally fails if the forward-pinned tag is not published:
  # CECSupport must be released only after its pinned AllMyStuff release.
  cargo update --manifest-path "$MANIFEST" \
    -p allmystuff-node \
    -p allmystuff-cec-protocol \
    -p allmystuff-cec-consent
fi

[[ -f "$LOCK" ]] || fail "$LOCK is missing"
source_count=0
while IFS= read -r source; do
  source_count=$((source_count + 1))
  [[ "$source" == *"?tag=${AMS_PIN}#"* ]] \
    || fail "$LOCK disagrees with .allmystuff-rev ($AMS_PIN): $source"
done < <(grep 'source = "git+https://github.com/mrjeeves/AllMyStuff?tag=' "$LOCK" || true)
((source_count > 0)) || fail "$LOCK contains no pinned AllMyStuff packages"

echo "pinned Cargo locks agree with AllMyStuff $AMS_PIN; MyOwnMesh sidecar pin is $MESH_PIN"
