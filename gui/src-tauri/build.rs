//! Build-time sidecar handling for the CEC Support client.
//!
//! `tauri.conf.json` lists two `externalBin` sidecars the client ships beside
//! itself so a customer never has to install anything separately:
//!
//!   * `myownmesh`        — the MyOwnMesh peer-to-peer daemon the node runs on.
//!   * `allmystuff-serve` — the AllMyStuff node binary, run in CEC client mode.
//!
//! `tauri_build` requires those files to exist at build time. In a real release
//! build the pipeline stages signed binaries into `binaries/` first (and the
//! *installer* additionally does reuse-or-bundle at install time — an existing,
//! new-enough AllMyStuff/`myownmesh` install is reused, otherwise the bundled
//! ones are used). Here we only need the slots to exist, so we stamp a
//! zero-byte stub for any sidecar a release step didn't already provide. The
//! runtime treats a zero-byte stub as "not bundled" and falls back to a
//! reused/installed binary, exactly like AllMyStuff's node discovery.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=binaries");
    // The runtime uses the target triple to find a dev-staged sidecar name.
    let triple = env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=DAEMON_SIDECAR_TRIPLE={triple}");

    for base in ["myownmesh", "allmystuff-serve"] {
        if let Err(e) = ensure_sidecar_slot(&triple, base) {
            println!("cargo:warning=couldn't stamp {base} sidecar stub: {e}");
        }
    }

    tauri_build::build();
}

fn exe_suffix(triple: &str) -> &'static str {
    if triple.contains("windows") {
        ".exe"
    } else {
        ""
    }
}

/// Ensure `binaries/<base>-<triple>{.exe}` exists; write a zero-byte stub if a
/// release step hasn't already staged a real binary there.
fn ensure_sidecar_slot(triple: &str, base: &str) -> std::io::Result<()> {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("binaries");
    fs::create_dir_all(&dir)?;
    let slot = dir.join(format!("{base}-{triple}{}", exe_suffix(triple)));
    if !slot.exists() {
        fs::write(&slot, b"")?;
        make_executable(&slot);
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
