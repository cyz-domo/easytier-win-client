; NSIS installer hooks for the EasyTierService lifecycle.
; The installer runs elevated (perMachine); POSTINSTALL registers and starts
; the background service, resolving the ORIGINAL interactive user's SID from
; the explorer process so named-pipe ACLs match the user who launched setup
; (not the elevated admin token). PREUNINSTALL stops owned cores, waits for
; the service to exit, then deletes it before files are removed. Failures
; surface as warnings instead of blocking uninstall, so a stuck service can
; be retried from app settings without leaving a half-deleted install.

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

; Resolve the original interactive user's SID via the explorer process token.
; Writes the SID into $R8; empty string on failure (service then fails closed
; on IPC startup, which is safer than a permissive ACL).
!macro _EasyTierResolveUserSid
  StrCpy $R8 ""
  nsExec::ExecToStack 'powershell.exe -NoProfile -NonInteractive -Command "(Get-CimInstance Win32_Process -Filter \"Name=''explorer.exe''\" | Select-Object -First 1 | ForEach-Object { $_.GetOwner() } | ForEach-Object { (New-Object System.Security.Principal.NTAccount($_.Domain, $_.User)).Translate([System.Security.Principal.SecurityIdentifier]).Value })"'
  Pop $1 ; exit code
  Pop $2 ; stdout
  ${If} $1 = 0
    ${If} $2 == "S-1-5-18"
      StrCpy $R8 ""
    ${Else}
      StrCpy $R8 $2
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro _EasyTierResolveUserSid
  ${If} $R8 == ""
    DetailPrint "Warning: cannot resolve interactive user SID; skipping service auto-install. Use app settings to install it."
  ${Else}
    DetailPrint "Installing EasyTier Service..."
    nsExec::ExecToLog 'sc.exe create EasyTierService binPath= "\"$INSTDIR\easytier-service.exe\" --interactive-user-sid=$R8" start= auto DisplayName= "EasyTier Service"'
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
