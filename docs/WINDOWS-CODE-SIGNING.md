# Windows code signing (Authenticode)

This guide is about making CEC Support **install and run on Windows without a
scary warning** — no SmartScreen "Windows protected your PC" and no UAC
"Unknown publisher". That is what **Authenticode code signing** buys you.

> **Not the same thing as release signing.** The sibling MyOwnMesh / AllMyStuff
> repos have a `RELEASE-SIGNING.md` that covers **minisign** — a checksum +
> signature the *self-updater* verifies before staging an update. That protects
> the update channel. **This** doc is a different layer: an OS-level signature
> Windows itself checks at download/install time. A shipping build wants both;
> they don't overlap.

CEC Support ships as an `.exe` / `.msi` (plus a PowerShell one-liner,
`irm https://support.cec.direct/install.ps1 | iex`) and installs under
`%LOCALAPPDATA%\Programs\CEC Support`, optionally registering a Windows service.
Every one of those binaries should be signed.

## TL;DR

1. Buy an **OV** or **EV** code-signing certificate from a cloud signer — we
   recommend **Azure Trusted Signing** (~$10/month).
2. In CI, **sign every `.exe` and the `.msi`** with SHA-256 **and an RFC-3161
   timestamp**, before uploading release assets.
3. Optionally Authenticode-sign `install.ps1` (see the caveat below).
4. Verify with `signtool verify /pa /v`.
5. SmartScreen reputation is **instant with EV**, and **accrues over time/volume
   with OV** — so keep the same certificate across releases.

---

## 1. What the warnings actually are

- **SmartScreen** ("Windows protected your PC" / "Application not commonly
  downloaded…"): triggered on a file that carries the **Mark-of-the-Web** (MOTW)
  — the zone flag Windows adds to anything downloaded from a browser or fetched
  over HTTPS. SmartScreen looks up the file's reputation, keyed on the **signing
  identity** (or, for unsigned files, the file hash). Unknown ⇒ warning.
- **UAC "Unknown publisher"**: the elevation prompt shows a yellow banner and no
  verified publisher name when the elevated binary is unsigned. A valid
  Authenticode signature turns it blue with your company name.

A `curl | iex` / `irm | iex` install is judged by the signatures on the
**binaries it downloads and runs**, and (if the user saves it) on the `.ps1`
itself. So the payoff comes almost entirely from signing the shipped
executables and the MSI.

## 2. Certificate options — OV vs EV, and the hardware rule

| | OV (Organization Validation) | EV (Extended Validation) |
|---|---|---|
| Publisher identity verified | yes | yes, more stringent |
| UAC shows your name | yes | yes |
| SmartScreen reputation | **accrues** over downloads/time | **immediate** |
| Cost | lower | higher |

**The hardware rule (important):** since **June 1, 2023**, the CA/Browser Forum
requires the private key for *all* publicly-trusted code-signing certificates
(OV **and** EV) to live on **FIPS 140-2 Level 2+ hardware** — a USB token, an
HSM, or a **cloud signing service**. You can no longer export a `.pfx` and sign
on any laptop. This is why a cloud signer is the path of least resistance.

## 3. Recommended: a cloud signing service

For a shop CEC's size, **[Azure Trusted Signing](https://learn.microsoft.com/azure/trusted-signing/)**
(formerly Azure Code Signing) is the cheapest, lowest-friction option:

- ~**$9.99/month**, Microsoft-run, key held in Azure (no token to mail around).
- One-time **identity validation** of your organization (historically ~3+ years
  of verifiable business history, or an alternative validation path).
- Integrates directly with `signtool` (via a signing **dlib**) and with a
  first-party **GitHub Action**.

Alternatives, all cloud/HSM-backed OV or EV:

- **DigiCert KeyLocker** — OV/EV, cloud HSM, good CI story.
- **SSL.com eSigner** — OV/EV, cloud signing API.
- **Sectigo** — OV/EV on a token or via their cloud.

Decision guide: cheapest + easiest ⇒ **Azure Trusted Signing (OV)**. Want
SmartScreen clean from day one and willing to pay more ⇒ **EV** via DigiCert or
Sectigo.

## 4. Signing with `signtool`

Sign **with SHA-256** and **always add an RFC-3161 timestamp** — the timestamp
is what keeps the signature valid after the certificate expires (an unsigned
timestamp means every binary "expires" when the cert does).

Sign every shipped executable — the client (`cec-support.exe`), the GUI exe, any
bundled `myownmesh` / node sidecar — and the `.msi`. Ideally sign the EXEs
*before* they're packaged into the MSI, then sign the MSI too.

**Generic (token / local cert):**

```powershell
signtool sign `
  /fd SHA256 `
  /tr http://timestamp.digicert.com /td SHA256 `
  /a `                            # auto-select the best cert in the store
  "cec-support.exe"
```

**Azure Trusted Signing** (uses the `Azure.CodeSigning.Dlib`):

```powershell
signtool sign `
  /v /debug `
  /fd SHA256 `
  /tr http://timestamp.acs.microsoft.com /td SHA256 `
  /dlib "C:\path\to\Azure.CodeSigning.Dlib.dll" `
  /dmdf ".\trusted-signing-metadata.json" `
  "cec-support.exe" "cec-support-gui.exe" "CEC Support.msi"
```

where `trusted-signing-metadata.json` names your endpoint / account / profile:

```json
{
  "Endpoint": "https://eus.codesigning.azure.net/",
  "CodeSigningAccountName": "<YOUR_ACCOUNT>",
  "CertificateProfileName": "<YOUR_PROFILE>"
}
```

## 5. Signing the PowerShell installer (`install.ps1`)

```powershell
Set-AuthenticodeSignature `
  -FilePath .\scripts\install.ps1 `
  -Certificate (Get-ChildItem Cert:\CurrentUser\My\<YOUR_THUMBPRINT>) `
  -TimestampServer http://timestamp.digicert.com
```

**Caveat, stated plainly:** `irm … | iex` executes the script **text in memory**,
and PowerShell's Execution Policy applies to script **files on disk**, not to a
piped string — so the one-liner does **not** enforce the `.ps1` signature. The
real protection for one-liner installs is that the script downloads
**signed binaries**. Signing `install.ps1` mainly helps the users who **save and
run** the file, and is cheap to do, so do it — just don't treat it as the
safeguard.

## 6. CI integration (GitHub Actions)

Slot a signing step into the release workflow **after build, before uploading
assets**, on a `windows-latest` runner. With Azure Trusted Signing:

```yaml
# .github/workflows/release.yml  (excerpt — fill in your account/profile)
jobs:
  release-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build
        run: |  # build the .exe / .msi ...

      - name: Sign with Azure Trusted Signing
        uses: azure/trusted-signing-action@v0
        with:
          azure-tenant-id: ${{ secrets.AZURE_TENANT_ID }}
          azure-client-id: ${{ secrets.AZURE_CLIENT_ID }}
          azure-client-secret: ${{ secrets.AZURE_CLIENT_SECRET }}
          endpoint: https://eus.codesigning.azure.net/
          trusted-signing-account-name: <YOUR_ACCOUNT>
          certificate-profile-name: <YOUR_PROFILE>
          files-folder: ${{ github.workspace }}\dist
          files-folder-filter: exe,msi
          file-digest: SHA256
          timestamp-rfc3161: http://timestamp.acs.microsoft.com
          timestamp-digest: SHA256

      - name: Upload release assets
        run: |  # gh release upload ...
```

Store the Azure credentials as repository **secrets**; nothing secret goes in
the repo. (DigiCert KeyLocker / SSL.com eSigner have equivalent Actions — swap
the signing step, keep the "sign before upload" placement.)

## 7. Verify the signature

```powershell
signtool verify /pa /v "cec-support.exe"      # /pa = default (non-driver) policy
Get-AuthenticodeSignature "cec-support.exe" | Format-List
```

Or right-click the file → **Properties → Digital Signatures**. A good signature
shows your organization name and a countersignature (the timestamp).

## 8. SmartScreen reputation

- Reputation is tied to the **signing identity** (and, until enough clean
  download volume accrues, roughly per-file). **EV shortcuts this to zero-day.**
- **Keep the same certificate / subject across releases** — rotating it resets
  OV reputation to zero. Only rotate on expiry or compromise, and overlap two
  releases when you do.
- A fresh **OV** cert starts cold; expect a few early downloads to still warn
  until Microsoft has seen enough clean installs. Submitting the app to
  Microsoft can help seed it.
- Signing does **not** exempt you from SmartScreen entirely on an OV cert with no
  history — it just makes the reputation *accumulate* instead of never starting.

## See also

- `RELEASE-SIGNING.md` (the release-signing / **minisign** doc, if/when added to
  this repo) — a different layer: updater artifact trust, not OS install trust.
- Microsoft: [Trusted Signing](https://learn.microsoft.com/azure/trusted-signing/),
  [SmartScreen for developers](https://learn.microsoft.com/windows/security/operating-system-security/virus-and-threat-protection/microsoft-defender-smartscreen/).
