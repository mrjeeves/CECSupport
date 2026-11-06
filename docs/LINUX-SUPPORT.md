# Linux build and support readiness

CECSupport shares its engine, capture/input backends, consent contract, and
sidecar releases with AllMyStuff. Linux support should follow that engine's
release and desktop behavior. This pipeline work prepares Linux packages;
successful compilation alone does not establish runtime support.

## Build targets

| Platform | Native runner | Rust target | Desktop bundles |
|---|---|---|---|
| Linux x86_64 | `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `.deb`, `.AppImage` |
| Linux aarch64 | `ubuntu-22.04-arm` | `aarch64-unknown-linux-gnu` | `.deb`, `.AppImage` |
| Windows x86_64 | `windows-latest` | `x86_64-pc-windows-msvc` | NSIS `setup.exe` |

Ubuntu 22.04 matches AllMyStuff's glibc 2.35 build baseline. It avoids
accidentally raising the glibc requirement by building on `ubuntu-latest`.
The baseline is not a promise that every distribution with glibc 2.35 works:
WebKitGTK, GTK, PipeWire, desktop portals, and their versions still matter.
Test packages on clean target machines, including machines without the build
PPA, before naming supported distributions.

`scripts/install-linux-deps.sh` supplies the shared native build packages for
CI and release jobs. On Ubuntu 22.04 it enables the PipeWire upstream PPA:
stock Jammy PipeWire headers are too old for the shared capture stack's
libspa bindings. The script is a developer/build-runner setup step, not a
customer installation script.

## Local development

Install Rust, Node 22, and the pnpm version in `gui/package.json`, then run on
Ubuntu:

```sh
just linux-deps
just setup
just pin-locks
just gui-backend
just gui-build
```

`just gui-backend` checks the native backend; `just gui-build` requires real,
checksum-verified release sidecars. `just dev` can use the development
sidecar fallback, so a working development session does not prove that a
release bundle is complete.

## Upstream pins and release order

The AllMyStuff release selected by `.allmystuff-rev` must match all three
AllMyStuff git dependencies in `gui/src-tauri/Cargo.toml`. Its Linux assets
must include `allmystuff-serve-linux-<arch>.tar.gz` and
`amst-linux-<arch>.tar.gz`, each with its published `.sha256`. The MyOwnMesh
release in `.myownmesh-rev` must likewise publish each Linux daemon archive
and checksum.

AllMyStuff v0.2.122 supplies the Linux/Wayland fixes and is now the pinned
Linux baseline. That pin meets the release workflow's Linux publication
threshold. All required upstream assets and checksums must still be available
for both architectures, and desktop runtime acceptance remains required before
advertising support. CI builds complete Linux candidates with the committed
pins and retains them as workflow artifacts. Windows publication continues
independently.

For the Linux rollout:

1. Verify that AllMyStuff v0.2.122 or a newer release containing the
   Linux/Wayland fixes has published its assets for both architectures.
2. Update `.allmystuff-rev` and all three Rust dependency tags together, run
   `just pin-locks`, and commit the matching GUI Cargo lockfile.
3. Run CI with the published pins. Linux jobs must compile/test the backend
   and build complete bundles with required sidecars for both architectures.
   Review the retained `.deb` and AppImage candidates before cutting a release.
4. Complete runtime acceptance below, then cut CECSupport's release with
   `just release <version>`. The release pin/lock guard must pass; do not
   bypass it to build against an older graph.
5. Add Linux download choices to the customer website only after validating
   the uploaded packages.

Release staging requires `myownmesh`, `allmystuff-serve`, and `amst` for the
target architecture. Linux skips Windows-only Crucible and VC runtime
resources. CEC Support's Windows Toolbox actions and Crucible are not Linux
maintenance tools.

## Install and update ownership

The `.deb` is a package-managed install: replace it through the package
installer when updating. For AppImage, replace the complete AppImage; the
current updater swaps a raw executable and does not rebuild an AppImage's
read-only mounted filesystem. Neither format should be presented as having
verified in-app self-update until that path has been tested.

Linux native drive mapping uses the shared node's rootless rclone/FUSE path.
The `.deb` declares `rclone` and `fuse3` as runtime dependencies; install it
through a package manager that resolves those dependencies. AppImage and
portable users must install `rclone` and `fuse3` separately for native drive
mapping: those formats do not bundle the external helpers.

Linux `cec-support-linux-<arch>.tar.gz` assets are separate updater/portable
payloads with `.sha256` sidecars. Each contains `cec-support`, `myownmesh`,
`allmystuff-serve`, and `amst` so it can launch without a separate AllMyStuff
installation. It does not install desktop dependencies. The current Linux
self-updater replaces only `cec-support`; it does not replace the three
companions. Replace the complete portable payload to refresh those files,
and test upgrade/repair separately from package or AppImage replacement.

## Runtime acceptance

Record desktop, distribution, architecture, installed PipeWire/portal
versions, and package format for each run. Test both Linux architectures
before advertising both downloads.

- On a clean machine, install and launch the `.deb` and AppImage. Verify
  sidecar discovery, support number, tray/background behavior, and autostart.
  Check native library dependencies without relying on developer packages.
- Test an X11 session and GNOME Wayland. Test KDE Wayland separately before
  claiming it. Wayland capture and input use the active user's session bus,
  PipeWire, and the desktop's portal backend; test the actual portal prompts,
  paired screen/input permissions, and cancellation behavior.
- From the matching AllMyStuff technician release, connect by support number,
  approve with the verification code, view the screen, move/click/type, then
  revoke access. Confirm capture and control stop immediately after revocation.
- Test display selection, scaling, multiple monitors, clipboard behavior,
  reconnect, and a new session after logging out and back in. Record any
  portal prompt that returns despite a standing CECSupport grant.
- Test native drive mapping with `rclone` and `fuse3` installed: mount a
  consented remote folder as the desktop user, read/write as permitted, and
  unmount cleanly after disconnect or revocation. Confirm the `.deb` resolves
  both dependencies and AppImage/portable installs find the external helpers.
- Test with AllMyStuff already installed and running, and with CECSupport
  alone. Verify both reuse the shared node without overwriting each other's
  service identity or state.
- Exercise user autostart and systemd lifecycle separately. A running
  headless service or standing consent grant does not bypass Wayland portal
  approval, make a logged-out desktop available, or prove unattended control.
- Verify package replacement, AppImage replacement, and raw-install update
  behavior independently; confirm the reported component versions afterward.

Keep unsupported session states and any distribution-specific dependency
requirements explicit in release notes and customer support guidance.
