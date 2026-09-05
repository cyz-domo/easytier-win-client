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
  DetailPrint "Stopping existing EasyTier Service..."
  nsExec::ExecToLog 'sc.exe stop EasyTierService'
  Pop $0
  Sleep 2000
  ; QueryEx output is parsed by a temporary PowerShell script. This handles
  ; START_PENDING and avoids passing the whole sc.exe output as a PID.
  FileOpen $1 "$PLUGINSDIR\stop-service.ps1" w
  FileWrite $1 "$$s = Get-CimInstance Win32_Service | Where-Object Name -eq 'EasyTierService' | Select-Object -First 1$\r$\n"
  FileWrite $1 "if ($$s -and $$s.ProcessId -gt 0) { Stop-Process -Id $$s.ProcessId -Force -ErrorAction SilentlyContinue }$\r$\n"
  FileClose $1
  nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\stop-service.ps1"'
  Pop $0
  ; Retry delete and confirm the SCM entry is gone before continuing.
  StrCpy $1 0
  service_delete_loop:
    nsExec::ExecToLog 'sc.exe delete EasyTierService'
    Pop $0
    Sleep 1000
    nsExec::ExecToStack 'sc.exe query EasyTierService'
    Pop $2
    Pop $3
    ${If} $2 != 0
      Goto service_deleted
    ${EndIf}
    IntOp $1 $1 + 1
    ${If} $1 < 15
      Goto service_delete_loop
    ${EndIf}
    MessageBox MB_ICONSTOP|MB_OK "无法停止或删除 EasyTierService。请重启 Windows 后再次运行卸载程序。"
    Abort
  service_deleted:
!macroend

; Resolve the original interactive user's SID via the explorer process owner.
; Writes the SID into $R8; empty string on failure (service then fails closed
; on IPC startup, which is safer than a permissive ACL).
!macro _EasyTierResolveUserSid
  StrCpy $R8 ""
  ; Dump the resolver to a temp .ps1: avoids NSIS quote-escaping pitfalls.
  FileOpen $1 "$PLUGINSDIR\resolve-sid.ps1" w
  FileWrite $1 "$$o = Get-CimInstance Win32_Process | Where-Object Name -eq 'explorer.exe' | Select-Object -First 1$\r$\n"
  FileWrite $1 "$$u = Invoke-CimMethod -InputObject $$o -MethodName GetOwner$\r$\n"
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

!macro NSIS_HOOK_PREINSTALL
  ; Stop/remove the old service before NSIS starts copying files. The old
  ; service may lock core DLL/SYS files, especially when upgrading in place.
  !insertmacro _EasyTierStopService
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro _EasyTierResolveUserSid
  ${If} $R8 == ""
    DetailPrint "Warning: cannot resolve interactive user SID; skipping service auto-install. Use app settings to install it."
  ${Else}
    ; Tauri resource layouts can place this file in either location depending
    ; on bundle mode; register the path that actually exists.
    StrCpy $R9 "$INSTDIR\easytier-service.exe"
    ${IfNot} ${FileExists} "$R9"
      StrCpy $R9 "$INSTDIR\resources\easytier-service.exe"
    ${EndIf}
    ${IfNot} ${FileExists} "$R9"
      DetailPrint "Warning: easytier-service.exe is missing; service was not registered."
    ${Else}
      DetailPrint "Installing EasyTier Service..."
      ; Persist the trusted SID beside the installed service. This avoids
      ; relying on optional NSIS $COMMONAPPDATA expansion and is readable by
      ; LocalSystem. The installer owns this file; users cannot modify it.
      FileOpen $1 "$INSTDIR\interactive-user.sid" w
      FileWrite $1 "$R8$\r$\n"
      FileClose $1
      nsExec::ExecToLog 'sc.exe create EasyTierService binPath= "\"$R9\" --interactive-user-sid=$R8" start= auto DisplayName= "EasyTier Service"'
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
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro _EasyTierStopService
  DetailPrint "EasyTier Service removed."
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Keep %ProgramData%\EasyTier (user config) unless a future option clears it.
  DetailPrint "EasyTier uninstall finished."
!macroend
