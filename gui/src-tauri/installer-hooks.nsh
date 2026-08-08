; CEC Support — NSIS installer hooks.
;
; Installs the Microsoft Visual C++ 2015–2022 redistributable alongside the app.
;
; Why: both the app and the bundled `allmystuff-serve` media sidecar are MSVC
; builds that link the dynamic VC runtime (vcruntime140.dll, msvcp140.dll). On a
; Windows install that has never had it — a fresh machine, a reimaged one, i.e.
; exactly the machines someone points a repair tool at — the app dies at launch
; with "vcruntime140.dll was not found". That is a dead end for a customer who
; was reaching for support in the first place, so the installer handles it
; instead of handing them a Microsoft download link to go and find.
;
; The redistributable is idempotent and cheap to re-run: a machine that already
; has the same or a newer runtime exits 1638 without touching anything, so
; there's no need to detect it first — detection would just be another thing
; that can be wrong.

!macro NSIS_HOOK_POSTINSTALL
  ; The build script stages this next to the app's other resources. A dev build
  ; that couldn't fetch it stamps a zero-byte placeholder, which is skipped.
  ${If} ${FileExists} "$INSTDIR\resources\vc_redist.x64.exe"
    ${GetSize} "$INSTDIR\resources" "/M=vc_redist.x64.exe /S=0B" $0 $1 $2
    ${If} $0 > 0
      DetailPrint "Installing the Microsoft Visual C++ runtime…"
      ; /quiet /norestart: no second wizard in front of the customer, and never
      ; reboot their machine out from under them mid-install.
      ExecWait '"$INSTDIR\resources\vc_redist.x64.exe" /install /quiet /norestart' $3
      ; 0 = installed, 1638 = same-or-newer already present, 3010 = installed and
      ; wants a reboot (which the app does not need — the DLLs are usable now).
      ${If} $3 == 0
      ${OrIf} $3 == 1638
      ${OrIf} $3 == 3010
        DetailPrint "Visual C++ runtime is ready."
      ${Else}
        ; Not fatal: the machine may already be fine, and failing the whole
        ; install here would leave a customer with nothing rather than with an
        ; app that very probably runs.
        DetailPrint "Visual C++ runtime installer returned $3 — continuing."
      ${EndIf}
    ${EndIf}
  ${EndIf}

  ; The app itself stays at ordinary integrity. This one install-time approval
  ; provisions its protected LocalSystem supervisor, which launches the node
  ; into the active desktop session. Repairs then need no per-command UAC.
  DetailPrint "Installing the CEC Support privileged desktop host..."
  StrCpy $4 "$INSTDIR\cec-support.exe"
  ${IfNot} ${FileExists} "$4"
    StrCpy $4 "$INSTDIR\CEC Support.exe"
  ${EndIf}
  ${If} ${FileExists} "$4"
    ExecWait '"$4" --service-bootstrap install' $5
    ${If} $5 != 0
      MessageBox MB_ICONEXCLAMATION|MB_OK "CEC Support was installed, but its privileged desktop host could not be enabled. Open CEC Support to try again."
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  StrCpy $4 "$INSTDIR\cec-support.exe"
  ${IfNot} ${FileExists} "$4"
    StrCpy $4 "$INSTDIR\CEC Support.exe"
  ${EndIf}
  ${If} ${FileExists} "$4"
    ExecWait '"$4" --service-bootstrap uninstall' $5
  ${EndIf}
!macroend
