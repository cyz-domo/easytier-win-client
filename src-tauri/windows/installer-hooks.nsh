; NSIS installer hooks for the EasyTierService lifecycle.
; The installer runs elevated (perMachine). POSTINSTALL resolves the ORIGINAL
; interactive user's SID (the elevated admin token would give the wrong SID),
; registers EasyTierService with it for named-pipe ACL validation, and starts
; it. SID resolution uses a temp PowerShell script file - inline -Command
; quoting inside NSIS is fragile and was the cause of silent registration
; failures. PREUNINSTALL stops owned cores, waits for the service to exit,
; then deletes it. Failures surface as warnings so uninstall can be retried
; from app settings without leaving a half-deleted install.

!macro _EasyTierStopService
  DetailPrint "Stopping EasyTier Service..."
  nsExec::ExecToLog 'sc.exe stop EasyTierService'
  ; Wait up to ~15s for the service (and the cores it owns) to exit so files
  ; are not locked during uninstall.
  StrCpy $0 0
  wait_loop:
    nsExec::ExecToLog 'sc.exe query EasyTierService /nopaging'
    Pop $1
    ${If} $1 != 0
      Goto service_stopped
    ${EndIf}
    Sleep 1000
    IntOp $0 $0 + 1
    ${If} $0 < 15
      Goto wait_loop
    ${EndIf}
  service_stopped:
!macroend

; Resolve the original interactive user's SID via the explorer process owner.
; Writes the SID into $R8; empty string on failure (service then fails closed
; on IPC startup, which is safer than a permissive ACL).
!macro _EasyTierResolveUserSid
  StrCpy $R8 ""
  ; Dump the resolver to a temp .ps1: avoids NSIS quote-escaping pitfalls.
  FileOpen $1 "$PLUGINSDIR\resolve-sid.ps1" w
  FileWrite $1 "$$o = Get-CimInstance Win32_Process | Where-Object Name -eq 'explorer.exe' | Select-Object -First 1$\r$\n"
  FileWrite $1 "$$u = $$o | ForEach-Object { $$_.GetOwner() }$\r$\n"
  FileWrite $1 "(New-Object System.Security.Principal.NTAccount($$u.Domain, $$u.User)).Translate([System.Security.Principal.SecurityIdentifier]).Value$\r$\n"
  FileClose $1
  nsExec::ExecToStack '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\resolve-sid.ps1"'
  Pop $2 ; exit code
  Pop $3 ; stdout (SID)
  ${If} $2 = 0
    ; Trim whitespace/CR that PowerShell may append.
    Push $3
    Call trim_sid
    Pop $R8
    ${If} $R8 == "S-1-5-18"
      StrCpy $R8 ""
    ${EndIf}
  ${EndIf}
!macroend

; Strip CR/LF/spaces from a stack string in place.
!macro _EasyTierTrim input output
  Push `${input}`
  Call trim_sid
  Pop `${output}`
!macroend

Function trim_sid
  Exch $R0
  Push $R1
  trim_loop:
    StrCpy $R1 "$R0" 1
    ${If} $R1 == " "
    ${OrIf} $R1 == "$\r"
    ${OrIf} $R1 == "$\n"
      StrCpy $R0 "$R0" "" 1
      Goto trim_loop
    ${EndIf}
  trim_tail:
    StrCpy $R1 "$R0" 1 -1
    ${If} $R1 == " "
    ${OrIf} $R1 == "$\r"
    ${OrIf} $R1 == "$\n"
      StrCpy $R0 "$R0" -1
      Goto trim_tail
    ${EndIf}
  Pop $R1
  Exch $R0
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro _EasyTierResolveUserSid
  ${If} $R8 == ""
    DetailPrint "Warning: cannot resolve interactive user SID; skipping service auto-install. Use app settings to install it."
  ${Else}
    DetailPrint "Installing EasyTier Service..."
    nsExec::ExecToLog 'sc.exe create EasyTierService binPath= "\"$INSTDIR\resources\easytier-service.exe\" --interactive-user-sid=$R8" start= auto DisplayName= "EasyTier Service"'
    Pop $0
    ${If} $0 = 0
      nsExec::ExecToLog 'sc.exe description EasyTierService "EasyTier background service"'
      nsExec::ExecToLog 'sc.exe start EasyTierService'
      Pop $0
      ${If} $0 = 0
        DetailPrint "EasyTier Service started."
      ${Else}
        DetailPrint "Warning: service registered but failed to start (error $0). It can be started from the app settings."
      ${EndIf}
    ${Else}
      DetailPrint "Warning: service registration failed (error $0). You can install it later from app settings."
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro _EasyTierStopService
  DetailPrint "Removing EasyTier Service..."
  nsExec::ExecToLog 'sc.exe delete EasyTierService'
  Pop $0
  ${If} $0 != 0
    DetailPrint "Warning: service deletion failed (error $0). Files may remain locked; retry uninstall."
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Keep %ProgramData%\EasyTier (user config) unless a future option clears it.
  DetailPrint "EasyTier uninstall finished."
!macroend
