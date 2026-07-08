# CEC Support end-user installer (Windows — the primary target).
#
# Installs the `cec-support` client (a single exe that is both the app and its
# CLI/service verbs), then makes sure the mesh pieces it reuses from AllMyStuff
# are in place, WITHOUT ever clobbering an existing AllMyStuff install:
#
#   * The node binary (`allmystuff-serve`) and the `myownmesh` daemon are
#     REUSED when a machine already has them (an AllMyStuff install, or a
#     standalone daemon) and they're new enough; otherwise the versions bundled
#     with this release are installed next to the client ("reuse, don't
#     clobber"). The `myownmesh` "new enough" bar is the version pinned in
#     `.myownmesh-rev`, exactly like AllMyStuff's installer.
#
# Pass -Service to also install the background service (so CEC Support can
# reconnect after reboots during a repair). Mesh trouble never fails the
# install — the app still opens and shows the customer's number.
#
# Usage (PowerShell):
#   irm https://raw.githubusercontent.com/mrjeeves/CECSupport/main/scripts/install.ps1 | iex
#   iex "& { $(irm https://raw.githubusercontent.com/mrjeeves/CECSupport/main/scripts/install.ps1) } -Service"
#   .\scripts\install.ps1 -DryRun

[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$Service,
    [switch]$ForceBundle,
    [string]$Prefix = "$env:LOCALAPPDATA\Programs\CEC Support",
    [string]$Repo = $(if ($env:CEC_SUPPORT_REPO) { $env:CEC_SUPPORT_REPO } else { "mrjeeves/CECSupport" }),
    [string]$MeshRepo = $(if ($env:MYOWNMESH_REPO) { $env:MYOWNMESH_REPO } else { "mrjeeves/MyOwnMesh" })
)

$ErrorActionPreference = "Stop"

function Log($msg)  { Write-Host "==> $msg" -ForegroundColor Cyan }
function Warn($msg) { Write-Host "!!! $msg" -ForegroundColor Yellow }
function Err($msg)  { Write-Host "xxx $msg" -ForegroundColor Red }

$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    "AMD64" { "x86_64" }
    "ARM64" { "aarch64" }
    default { $env:PROCESSOR_ARCHITECTURE.ToLower() }
}
$asset = "cec-support-windows-$arch.zip"
$nodeAsset = "allmystuff-serve-windows-$arch.zip"
$meshAsset = "myownmesh-windows-$arch.zip"

# Where an AllMyStuff install drops its binaries — the first place to look when
# reusing its node / daemon.
$AllMyStuffPrefix = "$env:LOCALAPPDATA\Programs\AllMyStuff"

# Extract a release zip over $Prefix, retrying briefly (Windows can keep a file
# lock for a moment after a process exits).
function Expand-Over([string]$zipPath) {
    for ($i = 0; $i -lt 5; $i++) {
        try {
            Expand-Archive -Path $zipPath -DestinationPath $Prefix -Force -ErrorAction Stop
            return
        } catch {
            if ($i -eq 4) { throw }
            Start-Sleep -Milliseconds 500
        }
    }
}

# Download $url to $dest and SHA-256-verify against $url.sha256. A missing
# sidecar downgrades to a warning; a present-but-wrong checksum is fatal for the
# client and skips a bundled extra. Returns $true on a verified (or unchecked)
# download, $false when the checksum mismatched.
function Get-Verified([string]$url, [string]$dest, [bool]$fatal) {
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
    $shaFile = "$dest.sha256"
    try {
        Invoke-WebRequest -Uri "$url.sha256" -OutFile $shaFile -UseBasicParsing
    } catch {
        Warn "No SHA256 sidecar for $(Split-Path $url -Leaf); skipping integrity check."
        return $true
    }
    $expected = (Get-Content $shaFile -Raw).Split()[0].Trim().ToLower()
    $actual = (Get-FileHash -Algorithm SHA256 $dest).Hash.ToLower()
    if ($expected -ne $actual) {
        if ($fatal) {
            Err "SHA256 mismatch for $(Split-Path $url -Leaf) — not installing it."
            exit 1
        }
        Warn "SHA256 mismatch for $(Split-Path $url -Leaf) — skipping it."
        return $false
    }
    Log "SHA256 OK"
    return $true
}

# Find the download URL for $assetName in a repo's latest release, or $null.
function Find-Asset([string]$repo, [string]$assetName) {
    $api = "https://api.github.com/repos/$repo/releases/latest"
    try {
        $release = Invoke-RestMethod -Uri $api -Headers @{ "User-Agent" = "cec-support-installer" }
    } catch {
        Warn "GitHub releases unreachable for ${repo}: $($_.Exception.Message)"
        return $null
    }
    ($release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1).browser_download_url
}

# Stop a running CEC Support before overwriting its (locked) exe. Stops the
# service first (so it can't respawn), then the app/agent. Returns $true if the
# service was running, so we can restart it on the new binary.
function Stop-CecSupport {
    $serviceWasRunning = $false
    $svc = Get-Service -Name "CECSupport" -ErrorAction SilentlyContinue
    if ($svc -and $svc.Status -ne 'Stopped') {
        Log "Stopping the CEC Support service so its binaries can be replaced"
        $serviceWasRunning = $true
        try { Stop-Service -Name "CECSupport" -Force -ErrorAction Stop }
        catch { & sc.exe stop CECSupport *> $null }
    }
    # The app / headless agent (leave the short-lived installer/CLI alone). The
    # reused node + daemon are AllMyStuff's, not ours, so we don't stop them.
    $procs = @(Get-Process -Name "cec-support" -ErrorAction SilentlyContinue)
    if ($procs.Count -gt 0) {
        Log "Stopping the running CEC Support app so its .exe unlocks"
        $procs | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 800
    }
    return $serviceWasRunning
}

# Install the client (cec-support.exe) from the latest CECSupport release.
function Install-Client {
    $url = Find-Asset $Repo $asset
    if (-not $url) { Err "No release asset matched $asset in $Repo."; exit 1 }
    Log "Downloading $url"
    if ($DryRun) { Log "(dry-run) would download and install $asset"; return }

    $tmp = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP "cec-install-$([guid]::NewGuid())")
    try {
        $zip = Join-Path $tmp $asset
        [void](Get-Verified $url $zip $true)
        if (-not (Test-Path $Prefix)) { New-Item -ItemType Directory -Force -Path $Prefix | Out-Null }
        Expand-Over $zip
        $exe = Join-Path $Prefix "cec-support.exe"
        if (-not (Test-Path $exe)) { throw "cec-support.exe not found in $asset after extraction" }
        Log "Installed: $exe"

        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if (-not ($userPath -split ";" | Where-Object { $_ -ieq $Prefix })) {
            Log "Adding $Prefix to user PATH"
            [Environment]::SetEnvironmentVariable("Path", "$userPath;$Prefix", "User")
            $env:Path = "$env:Path;$Prefix"
        }
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Reuse-or-bundle: the node binary (allmystuff-serve).
#
# CEC Support reuses AllMyStuff's node. If a usable one is already on this
# machine (beside an AllMyStuff install, or on PATH), leave it — the client
# finds it. Only when none exists do we install the copy bundled with this
# release, next to the client, where the client checks first.

function Find-ExistingNode {
    foreach ($cand in @(
        (Join-Path $Prefix "allmystuff-serve.exe"),
        (Join-Path $AllMyStuffPrefix "allmystuff-serve.exe")
    )) {
        if (Test-Path $cand) { return $cand }
    }
    $cmd = Get-Command allmystuff-serve -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    return $null
}

function Ensure-Node {
    $existing = Find-ExistingNode
    if ($existing -and -not $ForceBundle) {
        Log "Node: reusing the existing allmystuff-serve at $existing (not clobbering it)."
        return
    }
    if ($DryRun) { Log "(dry-run) would install the bundled node ($nodeAsset) next to the client"; return }

    $url = Find-Asset $Repo $nodeAsset
    if (-not $url) {
        Warn "Node: no bundled $nodeAsset in $Repo. The app still opens; live help needs the"
        Warn "AllMyStuff node — install AllMyStuff, or re-run this installer later."
        return
    }
    Log "Node: no existing allmystuff-serve found — installing the bundled one…"
    Log "Downloading $url"
    $tmp = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP "cec-node-$([guid]::NewGuid())")
    try {
        $zip = Join-Path $tmp $nodeAsset
        if (Get-Verified $url $zip $false) {
            Expand-Over $zip
            Log "Installed: $(Join-Path $Prefix 'allmystuff-serve.exe')"
        }
    } catch {
        Warn "Node download/install failed: $($_.Exception.Message)"
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Reuse-or-bundle: the myownmesh daemon (same rules as AllMyStuff's installer).

function Get-MeshMinVersion {
    $rev = $null
    if ($PSScriptRoot) {
        $local = Join-Path (Split-Path $PSScriptRoot -Parent) ".myownmesh-rev"
        if (Test-Path $local) { $rev = (Get-Content $local -Raw).Trim() }
    }
    if (-not $rev) {
        try {
            $rev = (Invoke-RestMethod -Uri "https://raw.githubusercontent.com/$Repo/main/.myownmesh-rev" -Headers @{ "User-Agent" = "cec-support-installer" }).Trim()
        } catch { return $null }
    }
    if ($rev -match '^v(\d+\.\d+(\.\d+)?)') { return [version]$Matches[1] }
    return $null
}

function Get-MeshVersion([string]$exe) {
    try {
        $raw = (& $exe --version 2>&1 | Out-String)
        $m = [regex]::Match($raw, '\d+\.\d+(?:\.\d+)?')
        if ($m.Success) { return [version]$m.Value }
        return $null
    } catch { return $null }
}

function Find-ExistingMesh {
    foreach ($cand in @(
        (Join-Path $Prefix "myownmesh.exe"),
        (Join-Path $AllMyStuffPrefix "myownmesh.exe")
    )) {
        if (Test-Path $cand) { return $cand }
    }
    $cmd = Get-Command myownmesh -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    return $null
}

function Ensure-Mesh {
    $existing = Find-ExistingMesh
    $min = Get-MeshMinVersion

    if ($existing -and -not $ForceBundle) {
        $ver = Get-MeshVersion $existing
        if ($ver -and (-not $min -or $ver -ge $min)) {
            if ($min) { Log "Mesh: reusing the installed myownmesh v$ver at $existing (needs v$min+)." }
            else      { Log "Mesh: reusing the installed myownmesh v$ver at $existing." }
            return
        }
        if ($ver) { Log "Mesh: installed myownmesh is v$ver but this release wants v$min+." }
        else      { Log "Mesh: $existing didn't answer --version." }
        if ($DryRun) { Log "(dry-run) would ask it to update itself: myownmesh update"; return }
        Log "Asking it to update itself (myownmesh update)…"
        try { & $existing update } catch { Warn "myownmesh update failed: $($_.Exception.Message)" }
        $ver = Get-MeshVersion $existing
        if ($ver -and (-not $min -or $ver -ge $min)) { Log "Mesh: myownmesh is now v$ver." }
        elseif (-not $ver) { Log "Mesh: myownmesh responded to 'update' but reports no readable version; assuming it's fine." }
        else { Warn "Mesh: couldn't bring myownmesh up to v$min. The app still runs; retry later with: myownmesh update" }
        return
    }

    if ($DryRun) { Log "(dry-run) would install the bundled myownmesh daemon ($meshAsset) next to the client"; return }
    Log "Mesh: no myownmesh daemon found — installing it next to the client…"
    $url = Find-Asset $MeshRepo $meshAsset
    if (-not $url) {
        Warn "Mesh: no $meshAsset in $MeshRepo. The app still opens; for live help, install"
        Warn "MyOwnMesh: iex `"& { `$(irm https://raw.githubusercontent.com/$MeshRepo/main/scripts/install.ps1) } -NoGui`""
        return
    }
    Log "Downloading $url"
    $tmp = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP "cec-mesh-$([guid]::NewGuid())")
    try {
        $zip = Join-Path $tmp $meshAsset
        if (Get-Verified $url $zip $false) {
            Expand-Over $zip
            $ver = Get-MeshVersion (Join-Path $Prefix "myownmesh.exe")
            if ($ver) { Log "Mesh: installed myownmesh v$ver — the client starts it automatically." }
            else      { Log "Mesh: installed myownmesh — the client starts it automatically." }
        }
    } catch {
        Warn "Daemon download/install failed: $($_.Exception.Message)"
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------

$serviceWasRunning = $false
if (-not $DryRun) { $serviceWasRunning = Stop-CecSupport }

Install-Client
Ensure-Node
Ensure-Mesh

# Optionally install the background service so CEC Support reconnects across
# reboots. The client's own service verb elevates on Windows as needed.
if ($Service) {
    if ($DryRun) {
        Log "(dry-run) would install the background service: cec-support service install"
    } else {
        Log "Installing the background service (may prompt for Administrator)…"
        try { & (Join-Path $Prefix "cec-support.exe") service install }
        catch { Warn "Service install failed: $($_.Exception.Message)" }
    }
} elseif ($serviceWasRunning -and -not $DryRun) {
    # We stopped an existing service to swap binaries — bring it back on the new one.
    Log "Restarting the CEC Support service on the updated binary"
    try { Start-Service -Name "CECSupport" -ErrorAction Stop }
    catch { & sc.exe start CECSupport *> $null }
}

Log "Done."
Log "Open CEC Support and read the big number to your CEC technician."
Log "  cec-support        # open the app"
Log "  cec-support id     # print your support number"
if (-not $Service) {
    Log "  cec-support service install   # keep it connected across reboots (needs Administrator)"
}
Log "Open a new terminal so the updated PATH takes effect."
