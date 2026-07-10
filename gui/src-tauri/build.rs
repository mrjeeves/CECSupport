//! Build-time bundling of the two sidecars the CEC Support client ships beside
//! itself, so a customer installs one file and never has to fetch anything:
//!
//!   * `myownmesh`        — the MyOwnMesh peer-to-peer daemon the node runs on,
//!     pinned in `.myownmesh-rev`, fetched from MyOwnMesh's GitHub Releases.
//!   * `allmystuff-serve` — the AllMyStuff node binary, run in CEC client mode,
//!     pinned in `.allmystuff-rev`, fetched from AllMyStuff's GitHub Releases.
//!
//! Both are dropped at `binaries/<base>-<target-triple>{.exe}`;
//! `tauri.conf.json`'s `externalBin` then ships them *inside* the app bundle
//! (the NSIS `setup.exe` / `.msi`). Resolution order for each, mirroring
//! AllMyStuff's own sidecar build script:
//!
//!   1. **`<BASE>_BIN`** override (`MYOWNMESH_BIN` / `ALLMYSTUFF_SERVE_BIN`) — a
//!      release pipeline can hand us a pre-signed binary and skip the fetch.
//!   2. **Sibling checkout** — a side-by-side `../MyOwnMesh` / `../AllMyStuff`
//!      with a built binary (the all-repos dev setup).
//!   3. **Prebuilt release asset** — download `<base>-<platform>.{tar.gz,zip}`
//!      from the pinned tag's GitHub release (no native build needed).
//!
//! Everything is best-effort: on any failure we stamp a zero-byte stub at the
//! sidecar slot (so `tauri_build`'s existence check passes) and the runtime
//! falls back to a reused/installed binary, exactly like AllMyStuff's node
//! discovery. Set `CEC_SKIP_SIDECAR=1` to skip the fetch entirely (offline / CI
//! builds that only verify compilation).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One sidecar to bundle: which binary, and where the pinned copy comes from.
struct Sidecar {
    /// Binary base name, e.g. `"myownmesh"` — also the `externalBin` slot base.
    base: &'static str,
    /// GitHub repo the release asset lives in.
    repo: &'static str,
    /// Repo-root pin file naming the release tag, e.g. `".myownmesh-rev"`.
    rev_file: &'static str,
    /// Env var that overrides everything with an explicit binary path.
    bin_env: &'static str,
    /// Sibling checkout dir name (`"MyOwnMesh"`) for the dev setup.
    sibling_repo: &'static str,
    /// Sub-path from the sibling repo root to its cargo `target/` dir
    /// (`""` for MyOwnMesh, `"node"` for AllMyStuff's node workspace).
    sibling_target_sub: &'static str,
}

const SIDECARS: &[Sidecar] = &[
    Sidecar {
        base: "myownmesh",
        repo: "https://github.com/mrjeeves/MyOwnMesh",
        rev_file: ".myownmesh-rev",
        bin_env: "MYOWNMESH_BIN",
        sibling_repo: "MyOwnMesh",
        sibling_target_sub: "",
    },
    Sidecar {
        base: "allmystuff-serve",
        repo: "https://github.com/mrjeeves/AllMyStuff",
        rev_file: ".allmystuff-rev",
        bin_env: "ALLMYSTUFF_SERVE_BIN",
        sibling_repo: "AllMyStuff",
        sibling_target_sub: "node",
    },
];

fn main() {
    // The runtime uses the target triple to find a dev-staged sidecar name.
    println!("cargo:rustc-env=DAEMON_SIDECAR_TRIPLE={}", target_triple());
    println!("cargo:rerun-if-env-changed=CEC_SKIP_SIDECAR");
    println!("cargo:rerun-if-env-changed=CEC_REQUIRE_SIDECARS");

    // Stamp the pinned AllMyStuff version (`.allmystuff-rev`, the same tag the
    // `allmystuff-serve` sidecar is fetched at) into the binary so the running
    // app can read it via `option_env!("ALLMYSTUFF_PIN")` and ask a reused,
    // separately-installed `allmystuff-serve` it doesn't own to update up to it.
    if let Some(pin) = read_allmystuff_pin() {
        println!("cargo:rustc-env=ALLMYSTUFF_PIN={pin}");
    }

    // A release build sets CEC_REQUIRE_SIDECARS=1 (see release.yml): then a
    // sidecar that can't be bundled fails the build *loudly* instead of stamping
    // a zero-byte stub — so a broken installer (green build, but no mesh inside)
    // can never ship. Dev builds leave it unset and fall back to a stub (offline,
    // or the pinned release not published yet).
    let require = env::var_os("CEC_REQUIRE_SIDECARS").is_some();

    for sc in SIDECARS {
        println!("cargo:rerun-if-changed={}", rev_file(sc).display());
        println!("cargo:rerun-if-env-changed={}", sc.bin_env);
        if let Err(e) = bundle_sidecar(sc) {
            if require {
                panic!(
                    "{} sidecar is required for a release build but could not be bundled: {e}. \
                     Ensure the pinned release ({}) is published with its \
                     {}-<platform> asset, then rebuild.",
                    sc.base, sc.repo, sc.base
                );
            }
            println!(
                "cargo:warning={} sidecar bundle skipped: {e} — the app still builds; \
                 at runtime it falls back to a reused/installed {} or one on PATH",
                sc.base, sc.base
            );
            if let Err(stub_err) = write_stub(sc) {
                println!(
                    "cargo:warning=could not write {} sidecar stub: {stub_err}",
                    sc.base
                );
            }
        }
    }

    tauri_build::build();
}

fn target_triple() -> String {
    env::var("TARGET").unwrap_or_else(|_| "unknown".into())
}

fn exe_suffix() -> &'static str {
    if target_triple().contains("windows") {
        ".exe"
    } else {
        ""
    }
}

fn binaries_dir() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("binaries")
}

/// The `externalBin` slot for this sidecar: `binaries/<base>-<triple>{.exe}`.
fn slot_path(sc: &Sidecar) -> PathBuf {
    binaries_dir().join(format!("{}-{}{}", sc.base, target_triple(), exe_suffix()))
}

/// The repo root — two parents up from `gui/src-tauri`.
fn repo_root() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// A sidecar's pin file at the repo root (`.myownmesh-rev` / `.allmystuff-rev`).
fn rev_file(sc: &Sidecar) -> PathBuf {
    repo_root().join(sc.rev_file)
}

/// The pinned AllMyStuff version from `.allmystuff-rev` (e.g. `v0.2.25`),
/// trimmed. `None` for a missing or empty file (an unpinned dev build).
fn read_allmystuff_pin() -> Option<String> {
    let path = repo_root().join(".allmystuff-rev");
    println!("cargo:rerun-if-changed={}", path.display());
    let raw = fs::read_to_string(path).ok()?;
    let pin = raw.trim();
    (!pin.is_empty()).then(|| pin.to_string())
}

fn bundle_sidecar(sc: &Sidecar) -> Result<(), String> {
    let bin_dir = binaries_dir();
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let slot = slot_path(sc);
    let sentinel = bin_dir.join(format!(".bundled-{}", sc.base));
    // Release mode (`CEC_REQUIRE_SIDECARS`, set by release.yml *and* `just
    // gui-build`): hermetic sidecars. Only the pinned, checksum-verified
    // release asset may fill the slot — never a sibling checkout or whatever
    // binary happens to be lying on the build machine. Dev mode keeps the
    // sibling convenience, but every staged binary still has to prove itself
    // (see [`verify_slot`]) before the build stamps it.
    let release_mode = env::var_os("CEC_REQUIRE_SIDECARS").is_some();
    let pin = read_pin(sc).as_deref().and_then(parse_semver);

    if env::var_os("CEC_SKIP_SIDECAR").is_some() {
        return Err("CEC_SKIP_SIDECAR set".into());
    }

    // 1. Explicit override — a deliberate human act, honored in both modes,
    //    but verified like every other source.
    if let Ok(p) = env::var(sc.bin_env) {
        let p = PathBuf::from(p);
        if nonempty_file(&p) {
            let sig = format!("bin:{}:{}", p.display(), file_mtime(&p));
            let fresh = !staged_matches(&slot, &sentinel, &sig);
            if fresh {
                stage(&p, &slot)?;
            }
            verify_slot(sc, &slot, pin, false)?;
            if fresh {
                let _ = fs::write(&sentinel, &sig);
                println!("cargo:warning=[{}] bundled from {}", sc.base, sc.bin_env);
            }
            return Ok(());
        }
    }

    // 2. Sibling checkout (matching the current build profile) — the all-repos
    //    dev loop. DEV ONLY: a release build must be reproducible from the pin,
    //    not from the state of a neighbouring checkout. Watch the picked binary
    //    so rebuilding the sidecar in its own repo re-runs this script and
    //    re-stages.
    if !release_mode {
        if let Some(p) = sibling_binary(sc) {
            println!("cargo:rerun-if-changed={}", p.display());
            if sibling_is_current(sc, &p) {
                let sig = format!("sib:{}:{}", p.display(), file_mtime(&p));
                let fresh = !staged_matches(&slot, &sentinel, &sig);
                if fresh {
                    stage(&p, &slot)?;
                }
                verify_slot(sc, &slot, pin, false)?;
                if fresh {
                    let _ = fs::write(&sentinel, &sig);
                    println!(
                        "cargo:warning=[{}] bundled from sibling {} checkout",
                        sc.base, sc.sibling_repo
                    );
                }
                return Ok(());
            }
            // A sibling older than the pin (or one that wouldn't state its
            // version) must not shadow the pinned release — that's what
            // bundled a stale/wedged node into `just dev`. Fall through to
            // the pinned download; rebuild the sibling to use it again.
            println!(
                "cargo:warning=[{}] sibling {} checkout is not usable at the {} pin — \
                 fetching the pinned release instead (update+rebuild the sibling to use it)",
                sc.base, sc.sibling_repo, sc.rev_file
            );
        }
    }

    // 3. Prebuilt release asset for the pinned tag — the only source a release
    //    build accepts. Checksum-verified in download_release_asset; version-
    //    verified (exact match to the pin) before the sentinel is stamped.
    let rev = fs::read_to_string(rev_file(sc))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("no {} pin and no override/sibling binary", sc.rev_file))?;
    let sig = format!("rev:{rev}");
    if staged_matches(&slot, &sentinel, &sig) {
        verify_slot(sc, &slot, pin, true)?;
        return Ok(());
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let staging = out_dir.join(format!("{}-staging", sc.base));
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let staged_bin = download_release_asset(sc, &rev, &staging)?;
    stage(&staged_bin, &slot)?;
    verify_slot(sc, &slot, pin, true)?;
    let _ = fs::write(&sentinel, &sig);
    println!(
        "cargo:warning=[{}] sidecar ready ({} bytes) from {} {rev}",
        sc.base,
        fs::metadata(&slot).map(|m| m.len()).unwrap_or(0),
        sc.repo
    );
    Ok(())
}

/// The staged slot must prove itself before the build stamps it: answer
/// `--version` (bounded — see [`binary_version`]) and satisfy the pin, `>=`
/// for dev sources (a dev sibling legitimately runs ahead) or `==` for a
/// pinned download. On failure the slot is deleted so a bad binary can't
/// linger for a later build (or the runtime's dev-slot lookup) to find. This
/// is the invariant that makes a green build mean "runnable sidecars at the
/// pinned versions": a wedged, corrupt, or ancient binary can never ship.
fn verify_slot(
    sc: &Sidecar,
    slot: &Path,
    pin: Option<(u64, u64, u64)>,
    exact: bool,
) -> Result<(), String> {
    let Some(have) = binary_version(slot) else {
        let _ = fs::remove_file(slot);
        return Err(format!(
            "staged {} wouldn't report a version — refusing to bundle it",
            sc.base
        ));
    };
    if let Some(want) = pin {
        let ok = if exact { have == want } else { have >= want };
        if !ok {
            let _ = fs::remove_file(slot);
            let rel = if exact { "exactly" } else { "at least" };
            return Err(format!(
                "staged {} is v{}.{}.{} but the pin wants {rel} v{}.{}.{}",
                sc.base, have.0, have.1, have.2, want.0, want.1, want.2
            ));
        }
    }
    Ok(())
}

/// True when the slot is non-empty and the sentinel records this exact staging
/// signature — the skip condition that keeps incremental builds cheap.
fn staged_matches(slot: &Path, sentinel: &Path, sig: &str) -> bool {
    let present = slot.metadata().map(|m| m.len() > 0).unwrap_or(false);
    present
        && fs::read_to_string(sentinel)
            .map(|s| s.trim() == sig)
            .unwrap_or(false)
}

fn file_mtime(p: &Path) -> u64 {
    p.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn nonempty_file(p: &Path) -> bool {
    p.metadata()
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

/// A built binary in a sibling `../<repo>[/sub]/target/{profile}/<base>`.
///
/// Prefers the profile **CEC itself is currently building** (`PROFILE`): a
/// `just dev` (debug) build takes the sibling's `debug/` binary — the one an
/// all-repos dev loop rebuilds — so a stale `release/allmystuff-serve.exe` from
/// some past `cargo build --release` can no longer shadow a freshly rebuilt
/// `debug/` one. (That shadowing is what silently bundled an old node into
/// `just dev`.) The opposite profile is a fallback for when only it was built,
/// and a `--target <triple>/` build outranks a plain one within each profile.
fn sibling_binary(sc: &Sidecar) -> Option<PathBuf> {
    let mut base_dir = repo_root().parent()?.join(sc.sibling_repo);
    if !sc.sibling_target_sub.is_empty() {
        base_dir = base_dir.join(sc.sibling_target_sub);
    }
    let target = base_dir.join("target");
    let name = format!("{}{}", sc.base, exe_suffix());
    let triple = target_triple();
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let other = if profile == "release" { "debug" } else { "release" };
    let candidates = [
        target.join(&triple).join(&profile).join(&name),
        target.join(&profile).join(&name),
        target.join(&triple).join(other).join(&name),
        target.join(other).join(&name),
    ];
    candidates.into_iter().find(|p| nonempty_file(p))
}

/// Whether a sibling binary is new enough to bundle: its `--version` is at or
/// above the pinned release named in `rev_file`. A missing / non-semver pin (a
/// sha or branch) or an unreadable `--version` returns `true` — we trust the
/// sibling rather than break a dev loop; only a *confident* "older than the pin"
/// (both sides parse, sibling is behind) demotes it to the pinned download. This
/// is what stops a forgotten, stale sibling checkout from shadowing the pin and
/// bundling an old node into `just dev`.
fn sibling_is_current(sc: &Sidecar, bin: &Path) -> bool {
    let Some(want) = read_pin(sc).as_deref().and_then(parse_semver) else {
        return true; // no comparable pin — keep the sibling
    };
    match binary_version(bin) {
        Some(have) => have >= want,
        None => {
            // A binary that can't state its version can't be trusted to run a
            // mesh either (wedged, corrupt, ancient). With a pinned release
            // available, never bundle an unverifiable sibling — leniency here
            // is exactly how a broken sibling exe became "app stuck at
            // Starting up".
            println!(
                "cargo:warning=[{}] sibling {} wouldn't report a version — rejecting it",
                sc.base,
                bin.display()
            );
            false
        }
    }
}

/// A sidecar's pinned tag from its `rev_file`, trimmed; `None` if missing/empty.
fn read_pin(sc: &Sidecar) -> Option<String> {
    let raw = fs::read_to_string(rev_file(sc)).ok()?;
    let pin = raw.trim().to_string();
    (!pin.is_empty()).then_some(pin)
}

/// Parse a `vX.Y.Z` / `X.Y.Z` tag into a comparable tuple. `None` for a
/// non-release pin (a commit sha or branch name), which can't be ordered.
fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut it = s.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    // The patch field may carry a pre-release/build suffix (e.g. "3-rc1"); take
    // its leading digits only.
    let patch = it
        .next()?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

/// `<bin> --version` parsed to `(major, minor, patch)` — the first `X.Y.Z` token
/// in its output (e.g. `allmystuff-serve 0.2.26`). `None` if the binary won't
/// run, prints nothing parseable, or doesn't answer in time.
///
/// **Bounded**: this step executes a foreign binary during the build, and a
/// sibling that wedges at startup (AV interference, a lock held by a running
/// daemon) would otherwise hang the whole build at the final crate with no
/// output. Poll up to 5s, then kill it and treat the sibling as current —
/// `--version` is a one-shot CLI verb, so a healthy binary answers instantly.
fn binary_version(bin: &Path) -> Option<(u64, u64, u64)> {
    let mut cmd = Command::new(bin);
    cmd.arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                use std::io::Read;
                let mut s = String::new();
                child.stdout.take()?.read_to_string(&mut s).ok()?;
                return s.split_whitespace().find_map(parse_semver);
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    println!(
                        "cargo:warning={} --version didn't answer within 5s — killed it",
                        bin.display()
                    );
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// GitHub release platform name for a Rust target triple (matches the asset
/// names AllMyStuff / MyOwnMesh publish).
fn release_platform_name(triple: &str) -> Result<&'static str, String> {
    Ok(match triple {
        t if t.contains("x86_64") && t.contains("linux") => "linux-x86_64",
        t if t.contains("aarch64") && t.contains("linux") => "linux-aarch64",
        t if t.contains("x86_64") && t.contains("apple") => "macos-x86_64",
        t if t.contains("aarch64") && t.contains("apple") => "macos-aarch64",
        t if t.contains("x86_64") && t.contains("windows") => "windows-x86_64",
        other => return Err(format!("no release platform mapping for target {other}")),
    })
}

/// Download + extract `<base>-<platform>.{tar.gz,zip}` for `tag`, returning the
/// path to the extracted binary. Shells out to `curl` + `tar` / `Expand-Archive`
/// so the build needs no extra crates.
/// Run a staging subprocess with a hard deadline, killing it on overrun.
/// Everything build.rs shells out to must be bounded — an unbounded child
/// hangs the whole build at the final crate with no output.
fn run_bounded(
    cmd: &mut Command,
    what: &str,
    secs: u64,
) -> Result<std::process::Output, String> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{what} spawn failed: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|e| format!("{what} output: {e}"));
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{what} didn't finish within {secs}s — killed"));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{what} wait failed: {e}"));
            }
        }
    }
}

/// Verify a downloaded archive against the `<asset>.sha256` its release
/// publishes. Fails closed: no readable checksum, no bundle — a release
/// asset without its checksum is as suspect as a mismatch.
fn verify_archive_sha256(archive: &Path, url: &str) -> Result<(), String> {
    let sha_url = format!("{url}.sha256");
    let out = run_bounded(
        Command::new("curl").args([
            "-fSL",
            "--connect-timeout",
            "15",
            "--max-time",
            "30",
            &sha_url,
        ]),
        "sha256 fetch",
        45,
    )?;
    if !out.status.success() {
        return Err(format!("couldn't fetch {sha_url} to verify the download"));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let want = text.split_whitespace().next().unwrap_or("").to_lowercase();
    if want.len() != 64 || !want.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{sha_url} didn't contain a sha256"));
    }
    let bytes = fs::read(archive).map_err(|e| e.to_string())?;
    let got = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&bytes);
        format!("{:x}", h.finalize())
    };
    if got != want {
        return Err(format!(
            "sha256 mismatch for {} (expected {want}, got {got}) — refusing the download",
            archive.display()
        ));
    }
    Ok(())
}

fn download_release_asset(sc: &Sidecar, tag: &str, staging: &Path) -> Result<PathBuf, String> {
    let triple = target_triple();
    let platform = release_platform_name(&triple)?;
    let windows = triple.contains("windows");
    let archive_ext = if windows { "zip" } else { "tar.gz" };
    let asset = format!("{}-{platform}.{archive_ext}", sc.base);
    let url = format!("{}/releases/download/{tag}/{asset}", sc.repo);
    let archive = staging.join(&asset);
    let _ = fs::remove_file(&archive);

    // Say so before going to the network: this step is why a build can sit on
    // the final crate for a while, and without the line it reads as a hang.
    println!("cargo:warning=[{}] downloading {asset} from {} {tag}…", sc.base, sc.repo);
    // Bounded fetch: a stalled connection must fail the step, not wedge the
    // whole build indefinitely (--retry would otherwise multiply the wait).
    // On timeout/failure a dev build falls back to the stub path and keeps
    // building; a release build still hard-fails via CEC_REQUIRE_SIDECARS.
    let status = Command::new("curl")
        .args([
            "-fSL",
            "--connect-timeout",
            "15",
            "--max-time",
            "120",
            "--retry",
            "3",
            "--retry-max-time",
            "180",
            "-o",
        ])
        .arg(&archive)
        .arg(&url)
        .status()
        .map_err(|e| format!("curl spawn failed: {e} (install curl, or use a sibling checkout)"))?;
    if !status.success() {
        return Err(format!("curl failed fetching {url}"));
    }
    if fs::metadata(&archive).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err("downloaded archive is empty".into());
    }
    // The release publishes `<asset>.sha256` — verify before trusting the
    // bytes. A proxy error page or truncated download must never become the
    // bundled mesh engine.
    verify_archive_sha256(&archive, &url)?;

    if windows {
        let out = run_bounded(
            Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(format!(
                    "Expand-Archive -Force -Path '{}' -DestinationPath '{}'",
                    archive.display(),
                    staging.display()
                )),
            "Expand-Archive",
            60,
        )?;
        if !out.status.success() {
            return Err(format!(
                "Expand-Archive failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    } else {
        let out = run_bounded(
            Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(staging),
            "tar",
            60,
        )?;
        if !out.status.success() {
            return Err(format!(
                "tar failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }

    let bin = staging.join(format!("{}{}", sc.base, exe_suffix()));
    if !bin.exists() {
        return Err(format!(
            "extracted {asset} but {} binary not found",
            sc.base
        ));
    }
    validate_binary(&bin)?;
    Ok(bin)
}

/// Sanity-check the extracted binary's magic bytes — guard against an HTML
/// error page or a truncated download landing in the sidecar slot.
fn validate_binary(p: &Path) -> Result<(), String> {
    let bytes = fs::read(p).map_err(|e| e.to_string())?;
    let ok = bytes.starts_with(b"\x7fELF")               // Linux ELF
        || bytes.starts_with(b"MZ")                      // Windows PE
        || bytes.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])  // macOS Mach-O (64-bit LE)
        || bytes.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE]); // macOS universal
    if ok {
        Ok(())
    } else {
        Err(format!("{} is not a recognised executable", p.display()))
    }
}

/// Copy `src` into the sidecar slot and mark it executable.
fn stage(src: &Path, dst: &Path) -> Result<(), String> {
    fs::copy(src, dst).map_err(|e| format!("copy {} → {}: {e}", src.display(), dst.display()))?;
    make_executable(dst);
    Ok(())
}

/// Zero-byte placeholder so `tauri_build`'s `externalBin` existence check passes
/// when no real binary could be staged. The runtime ignores zero-byte stubs.
fn write_stub(sc: &Sidecar) -> Result<(), String> {
    let bin_dir = binaries_dir();
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let p = slot_path(sc);
    if !p.exists() {
        fs::write(&p, b"").map_err(|e| e.to_string())?;
        make_executable(&p);
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
fn make_executable(_p: &Path) {}
