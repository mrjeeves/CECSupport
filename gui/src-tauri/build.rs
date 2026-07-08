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

/// The pinned AllMyStuff version from `.allmystuff-rev` (e.g. `v0.2.22`),
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

    if env::var_os("CEC_SKIP_SIDECAR").is_some() {
        return Err("CEC_SKIP_SIDECAR set".into());
    }

    // 1. Explicit override.
    if let Ok(p) = env::var(sc.bin_env) {
        let p = PathBuf::from(p);
        if nonempty_file(&p) {
            let sig = format!("bin:{}:{}", p.display(), file_mtime(&p));
            if !staged_matches(&slot, &sentinel, &sig) {
                stage(&p, &slot)?;
                let _ = fs::write(&sentinel, &sig);
                println!("cargo:warning=[{}] bundled from {}", sc.base, sc.bin_env);
            }
            return Ok(());
        }
    }

    // 2. Sibling checkout (release before debug) — the all-repos dev loop.
    if let Some(p) = sibling_binary(sc) {
        let sig = format!("sib:{}:{}", p.display(), file_mtime(&p));
        if !staged_matches(&slot, &sentinel, &sig) {
            stage(&p, &slot)?;
            let _ = fs::write(&sentinel, &sig);
            println!(
                "cargo:warning=[{}] bundled from sibling {} checkout",
                sc.base, sc.sibling_repo
            );
        }
        return Ok(());
    }

    // 3. Prebuilt release asset for the pinned tag.
    let rev = fs::read_to_string(rev_file(sc))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("no {} pin and no override/sibling binary", sc.rev_file))?;
    let sig = format!("rev:{rev}");
    if staged_matches(&slot, &sentinel, &sig) {
        return Ok(());
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let staging = out_dir.join(format!("{}-staging", sc.base));
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let staged_bin = download_release_asset(sc, &rev, &staging)?;
    stage(&staged_bin, &slot)?;
    let _ = fs::write(&sentinel, &sig);
    println!(
        "cargo:warning=[{}] sidecar ready ({} bytes) from {} {rev}",
        sc.base,
        fs::metadata(&slot).map(|m| m.len()).unwrap_or(0),
        sc.repo
    );
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

/// A built binary in a sibling `../<repo>[/sub]/target/{release,debug}/<base>`.
fn sibling_binary(sc: &Sidecar) -> Option<PathBuf> {
    let mut base_dir = repo_root().parent()?.join(sc.sibling_repo);
    if !sc.sibling_target_sub.is_empty() {
        base_dir = base_dir.join(sc.sibling_target_sub);
    }
    let target = base_dir.join("target");
    let name = format!("{}{}", sc.base, exe_suffix());
    let triple = target_triple();
    // With or without a `--target <triple>` segment; release before debug.
    let candidates = [
        target.join(&triple).join("release").join(&name),
        target.join("release").join(&name),
        target.join(&triple).join("debug").join(&name),
        target.join("debug").join(&name),
    ];
    candidates.into_iter().find(|p| nonempty_file(p))
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
fn download_release_asset(sc: &Sidecar, tag: &str, staging: &Path) -> Result<PathBuf, String> {
    let triple = target_triple();
    let platform = release_platform_name(&triple)?;
    let windows = triple.contains("windows");
    let archive_ext = if windows { "zip" } else { "tar.gz" };
    let asset = format!("{}-{platform}.{archive_ext}", sc.base);
    let url = format!("{}/releases/download/{tag}/{asset}", sc.repo);
    let archive = staging.join(&asset);
    let _ = fs::remove_file(&archive);

    let status = Command::new("curl")
        .args(["-fSL", "--retry", "3", "-o"])
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

    if windows {
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "Expand-Archive -Force -Path '{}' -DestinationPath '{}'",
                archive.display(),
                staging.display()
            ))
            .output()
            .map_err(|e| format!("Expand-Archive spawn failed: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "Expand-Archive failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    } else {
        let out = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(staging)
            .output()
            .map_err(|e| format!("tar spawn failed: {e}"))?;
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
