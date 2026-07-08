#!/usr/bin/env bash
# Bump the CEC Support version everywhere it's pinned, in one atomic edit.
# Argument is the new version, e.g. `./scripts/bump-version.sh 0.2.0`.
#
# Edits:
#   - Cargo.toml                    [workspace.package].version (the canonical
#                                   source; `cec-support-service` inherits it via
#                                   `version.workspace = true`)
#   - Cargo.lock                    refreshed by `cargo update` to track it
#   - gui/src-tauri/Cargo.toml      [package].version (the `cec-support` binary —
#                                   a separate, excluded workspace)
#   - gui/src-tauri/Cargo.lock      cec-support [[package]] version (if present —
#                                   it isn't committed until the AllMyStuff git
#                                   deps resolve, so this is best-effort)
#   - gui/package.json              "version"
#
# The GUI lives in its own Cargo workspace (so `cargo build --workspace` at the
# root stays fast — no Tauri compile), so its version doesn't auto-inherit and we
# keep it in lockstep here. Tauri reports the app version to the frontend from
# `gui/src-tauri/Cargo.toml`. Unlike AllMyStuff there's no `node/` workspace to
# bump — the node engine is a git dependency pinned in `.allmystuff-rev`.
#
# After this script: stage + commit + tag — the Justfile's `release` recipe does
# that part. Mirrors AllMyStuff's / MyOwnMesh's bump-version.sh.

set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <version>" >&2
    exit 2
fi

VERSION="$1"

# Validate looks-like-semver.
if ! echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$'; then
    echo "error: '$VERSION' does not look like a semver string" >&2
    exit 2
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE_TOML="$ROOT/Cargo.toml"
GUI_TAURI_TOML="$ROOT/gui/src-tauri/Cargo.toml"
GUI_TAURI_LOCK="$ROOT/gui/src-tauri/Cargo.lock"
GUI_PACKAGE_JSON="$ROOT/gui/package.json"

if [ ! -f "$WORKSPACE_TOML" ]; then
    echo "error: $WORKSPACE_TOML not found" >&2
    exit 2
fi

# [workspace.package].version — the canonical source. The lone member crate
# (`cec-support-service`) declares `version.workspace = true` and inherits it, so
# there are no internal dep pins to keep in sync (all [workspace.dependencies]
# here are external).
python3 - "$WORKSPACE_TOML" "$VERSION" <<'PY'
import re
import sys

path, version = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    content = f.read()

pkg_pattern = re.compile(
    r'(\[workspace\.package\][^\[]*?\n\s*version\s*=\s*")[^"]*(")',
    re.DOTALL,
)
content, n = pkg_pattern.subn(rf'\g<1>{version}\g<2>', content, count=1)
if n != 1:
    print(f"error: could not find [workspace.package].version in {path}", file=sys.stderr)
    sys.exit(1)

with open(path, "w", encoding="utf-8") as f:
    f.write(content)
print(f"bumped {path} -> {version}")
PY

# Refresh Cargo.lock so it tracks the new version.
cd "$ROOT"
cargo update --workspace --quiet || true

# --- GUI sub-workspace (the `cec-support` Tauri binary) -----------------

if [ -f "$GUI_TAURI_TOML" ]; then
    # gui/src-tauri/Cargo.toml — bump [package].version (first match under the
    # [package] header).
    python3 - "$GUI_TAURI_TOML" "$VERSION" <<'PY'
import re
import sys

path, version = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    content = f.read()

pattern = re.compile(
    r'(\[package\][^\[]*?\n\s*version\s*=\s*")[^"]*(")',
    re.DOTALL,
)
new_content, n = pattern.subn(rf'\g<1>{version}\g<2>', content, count=1)
if n != 1:
    print(f"error: could not find [package].version in {path}", file=sys.stderr)
    sys.exit(1)

with open(path, "w", encoding="utf-8") as f:
    f.write(new_content)
print(f"bumped {path} -> {version}")
PY
fi

if [ -f "$GUI_TAURI_LOCK" ]; then
    # gui/src-tauri/Cargo.lock — bump the [[package]] entry named "cec-support".
    python3 - "$GUI_TAURI_LOCK" "$VERSION" <<'PY'
import re
import sys

path, version = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    content = f.read()

pattern = re.compile(
    r'(name\s*=\s*"cec-support"\s*\nversion\s*=\s*")[^"]*(")',
)
new_content, n = pattern.subn(rf'\g<1>{version}\g<2>', content, count=1)
if n != 1:
    print(f"warning: could not find cec-support in {path} (skipping)", file=sys.stderr)
else:
    with open(path, "w", encoding="utf-8") as f:
        f.write(new_content)
    print(f"bumped {path} -> {version}")
PY
fi

if [ -f "$GUI_PACKAGE_JSON" ]; then
    # gui/package.json — node is the most portable JSON editor we can rely on
    # across maintainer machines.
    node -e '
        const fs = require("fs");
        const f = process.argv[1];
        const j = JSON.parse(fs.readFileSync(f, "utf8"));
        j.version = process.argv[2];
        fs.writeFileSync(f, JSON.stringify(j, null, 2) + "\n");
        console.log(`bumped ${f} -> ${process.argv[2]}`);
    ' "$GUI_PACKAGE_JSON" "$VERSION"
fi

echo "ok"
