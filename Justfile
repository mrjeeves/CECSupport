# CEC Support — one-command operations.
# Install `just` (https://just.systems) then run `just setup` to get going.
#
# Mirrors the AllMyStuff Justfile (the shared engine + technician app) so both
# repos drive the same way. The differences come from CEC Support having no
# local `node/` workspace: it reuses the AllMyStuff node engine as a git
# dependency (pinned in `.allmystuff-rev`) and bundles the `myownmesh` +
# `allmystuff-serve` sidecars into its installer — so there's no node
# build/serve/check here, and `dev` needs no pre-build.
#
# `set shell` is used on Linux/macOS. On Windows the global `windows-shell`
# override routes recipes through PowerShell so they find `pnpm.cmd` / `node.exe`
# via the Windows PATH. Recipes with bash-specific syntax need a `[windows]`
# variant; recipes that just call cross-platform tools (cargo, pnpm, git) work
# in both shells unmodified.
set shell := ["bash", "-cu"]
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"]

default: help

help:
    @just --list

# Install the front-end deps, and remind about the toolchain prerequisites. CEC
# Support is a Tauri + Svelte app, so it needs Rust (rustup), Node 22+, pnpm,
# and (on Linux) the WebKitGTK dev libs Tauri links against; on Windows it needs
# WebView2 (preinstalled on Windows 11). The per-OS system packages are listed at
# https://v2.tauri.app/start/prerequisites/.
[unix]
[doc("Install front-end deps (+ print the Tauri system-package prerequisites).")]
setup:
    @echo "Prerequisites: Rust (rustup), Node 22+, pnpm, and the Tauri system deps:"
    @echo "  https://v2.tauri.app/start/prerequisites/"
    @cd gui && pnpm install

[windows]
[doc("Install front-end deps (+ print the Tauri system-package prerequisites).")]
setup:
    @Write-Output "Prerequisites: Rust (rustup), Node 22+, pnpm, and WebView2 (ships with Windows 11)."
    @cd gui; pnpm install

# Run the desktop app (Tauri + Svelte) with hot reload. Unlike AllMyStuff there's
# no local node binary to pre-build: the AllMyStuff node engine is a git dep the
# Tauri backend embeds, and the `myownmesh` + `allmystuff-serve` sidecars are
# staged into the bundle at release time (build.rs stamps stubs otherwise, and
# the client falls back to a reused/installed copy). So this just runs the app.
[unix]
[doc("Run the app with hot reload.")]
dev *ARGS:
    @cd gui && pnpm install --silent && pnpm tauri dev {{ARGS}}

[windows]
[doc("Run the app with hot reload.")]
dev *ARGS:
    @cd gui; pnpm install --silent; pnpm tauri dev {{ARGS}}

# Build the light root workspace (the service-installer crate — no webview).
build:
    @cargo build --workspace

build-release:
    @cargo build --workspace --release

# Build the desktop installer: the NSIS `setup.exe` + `.msi` on Windows (what a
# customer downloads), or the `.deb`/`.AppImage`/`.dmg` on Linux/macOS.
# CEC_REQUIRE_SIDECARS=1 makes a local installer build behave exactly like CI:
# hermetic sidecars — only the pinned, checksum-verified release assets are
# bundled (never a sibling checkout or stray binary), and a sidecar that can't
# be fetched+verified fails the build loudly instead of stamping a stub.
[unix]
[doc("Build the desktop installer bundle (hermetic sidecars, like CI).")]
gui-build:
    @cd gui && pnpm install --silent && CEC_REQUIRE_SIDECARS=1 pnpm tauri build

[windows]
[doc("Build the desktop installer bundle (hermetic sidecars, like CI).")]
gui-build:
    @cd gui; pnpm install --silent; $env:CEC_REQUIRE_SIDECARS='1'; pnpm tauri build

# Print this computer's CEC Support number — the code a customer reads out to
# their technician. Runs the `cec-support` binary's CLI (needs the full
# toolchain: the AllMyStuff git deps + webview/media stack).
[doc("Print this computer's CEC Support number (needs the full toolchain).")]
id:
    @cargo run --manifest-path gui/src-tauri/Cargo.toml --bin cec-support -- id

# Run the `cec-support` binary directly — its CLI escape hatch. e.g.
# `just run -- run --service` (headless agent), `just run -- service install`
# (keep it connected across reboots). Needs the full toolchain, so it's for a
# real dev machine. Mirrors AllMyStuff's `serve`.
[doc("Run the cec-support binary's CLI, e.g. `just run -- service install`.")]
run *ARGS:
    @cargo run --manifest-path gui/src-tauri/Cargo.toml --bin cec-support -- {{ARGS}}

# Stop this machine's whole CEC Support stack: the client (`cec-support`), the
# reused/bundled AllMyStuff node (`allmystuff-serve`), and the `myownmesh` daemon
# it runs on. Use it for a clean slate between `just dev` runs, or to sweep an
# orphaned daemon (a hard Ctrl-C can leave one running, and the next run silently
# reuses it). Restart with `just dev`.
[unix]
[doc("Kill this machine's CEC stack (client + node + myownmesh daemon).")]
kill:
    @pkill -TERM -f '[c]ec-support' 2>/dev/null; pkill -TERM -f '[a]llmystuff-serve' 2>/dev/null; pkill -f '[m]yownmesh.* serve' 2>/dev/null; echo 'stopped the CEC client + node + mesh daemon (whatever was running)'

[windows]
[doc("Kill this machine's CEC stack (client + node + myownmesh daemon).")]
kill:
    @Get-Process cec-support,allmystuff-serve,myownmesh,myownmesh-* -ErrorAction SilentlyContinue | Stop-Process -Force; Write-Output "CEC stack stopped (cec-support + allmystuff-serve + myownmesh)"; exit 0

# Clean restart: kill the stack, then start the app fresh.
[doc("Kill the stack, then `just dev`.")]
restart *ARGS: kill
    @just dev {{ARGS}}

# Discard local changes, pull the latest, and fetch every remote branch — a
# pristine tree so `just dev` starts clean each time. git commands are identical
# on bash and PowerShell, so no [windows] variant is needed.
[doc("Discard local changes + git pull + fetch all branches — a clean slate.")]
pull:
    @git reset --hard HEAD
    @git pull
    @git fetch --all --prune

# `git checkout` with a clean slate first: `pull` runs ahead of it, so the tree
# is pristine and whatever branch you name is already fetched. Args pass straight
# through — `just checkout main`, `just checkout -b feature`, etc.
[doc("just pull (clean + fetch all), then git checkout (e.g. `just checkout main`).")]
checkout *args: pull
    @git checkout {{args}}

# The one-liner clean start: stop the stack, pull a pristine tree, then run the
# app. `just go -- <args>` forwards to `dev`.
[doc("just kill + just pull + just dev.")]
go *ARGS: kill pull
    @just dev {{ARGS}}

fmt:
    @cargo fmt --all

fmt-check:
    @cargo fmt --all --check

lint:
    @cargo clippy --workspace --all-targets -- -D warnings

test:
    @cargo test --workspace --no-fail-fast

# Typecheck + build the Svelte front-end (no webview needed) — the client's hard
# CI validation.
[unix]
[doc("Typecheck + build the front-end.")]
gui-check:
    @cd gui && pnpm install --frozen-lockfile && pnpm check && pnpm build

[windows]
[doc("Typecheck + build the front-end.")]
gui-check:
    @cd gui; pnpm install --frozen-lockfile; pnpm check; pnpm build

# The Tauri backend (`gui/src-tauri`) — fmt + clippy + test. NOT part of `check`
# or CI yet: it depends on the AllMyStuff node engine + CEC crates via git
# (tag = `.allmystuff-rev`), which only resolve once that tag is published, and
# it needs the media/webview toolchain. Run it on a dev machine once the deps
# land — the analog of AllMyStuff's `node-check`.
[doc("fmt+clippy+test the Tauri backend (needs published AllMyStuff deps; not in CI).")]
gui-backend:
    @cd gui/src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test

# Everything CI runs: Rust fmt + clippy + test (the light service workspace),
# then the GUI typecheck/build. (The Tauri backend isn't in CI — see
# `gui-backend` for why.)
[doc("Everything CI runs: Rust fmt + clippy + test, then the GUI typecheck/build.")]
check: fmt-check lint test gui-check

# Cut a release: bump the workspace + GUI versions, commit, push, then push the
# `v{{VERSION}}` tag. Mirrors AllMyStuff / MyOwnMesh. Pushing the tag triggers
# `.github/workflows/release.yml`, which builds the Windows Tauri bundle (with
# the pinned myownmesh + allmystuff-serve sidecars fetched in) and publishes the
# `setup.exe` / `.msi` to the GitHub release. (The installer ships unsigned for
# now — see docs/WINDOWS-CODE-SIGNING.md to add Authenticode signing.) Cut this
# only after the pinned AllMyStuff release (.allmystuff-rev) is published, so the
# serve sidecar resolves.
[unix]
[doc("Cut a release: bump versions, commit, push, tag (triggers the build+publish workflow).")]
release VERSION:
    @./scripts/bump-version.sh {{VERSION}}
    @if ! git diff --quiet Cargo.toml Cargo.lock gui/src-tauri/Cargo.toml gui/package.json; then \
        git add Cargo.toml Cargo.lock crates/*/Cargo.toml gui/src-tauri/Cargo.toml gui/package.json; \
        git commit -m "chore(release): {{VERSION}}"; \
    fi
    @git push
    @git tag v{{VERSION}}
    @git push origin v{{VERSION}}
    @echo ""
    @echo "✓ pushed tag v{{VERSION}}."
