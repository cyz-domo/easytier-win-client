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
  FileWrite $1 "if ($$s -and $$s.ProcessId -gt 0) { for ($$i=0; $$i -lt 20; $$i++) { Start-Sleep -Milliseconds 500; $$state = (Get-CimInstance Win32_Service | Where-Object Name -eq 'EasyTierService').State; if ($$state -eq 'Stopped') { exit } }; Stop-Process -Id $$s.ProcessId -Force -ErrorAction SilentlyContinue }$\r$\n"
  FileClose $1
  nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\stop-service.ps1"'
  Pop $0
  ; Terminate processes, stop WinDivert kernel driver service, and rename any locked
  ; driver/DLL files so installer file replacement succeeds without reboot.
  FileOpen $1 "$PLUGINSDIR\stop-cores.ps1" w
  FileWrite $1 "@('easytier-core', 'easytier-cli', 'easytier-win-client', 'easytier-service') | ForEach-Object { Get-Process -Name $$_ -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue }$\r$\n"
  FileWrite $1 "if ('$INSTDIR') { Get-Process -ErrorAction SilentlyContinue | Where-Object { try { $$_.Path -and $$_.Path.StartsWith('$INSTDIR', [System.StringComparison]::OrdinalIgnoreCase) } catch { $$false } } | Stop-Process -Force -ErrorAction SilentlyContinue }$\r$\n"
  FileWrite $1 "@('WinDivert', 'WinDivert14', 'WinDivert22') | ForEach-Object { & sc.exe stop $$_ 2>$$null; & net.exe stop $$_ /y 2>$$null; Start-Sleep -Milliseconds 200; & sc.exe delete $$_ 2>$$null }$\r$\n"
  FileWrite $1 "$$dirs = @((Join-Path '$INSTDIR' 'core'), '$INSTDIR')$\r$\n"
  FileWrite $1 "foreach ($$dir in $$dirs) {$\r$\n"
  FileWrite $1 "  if (Test-Path $$dir) {$\r$\n"
  FileWrite $1 "    Get-ChildItem -Path $$dir -File -ErrorAction SilentlyContinue | Where-Object { $$_.Extension -in '.sys', '.dll', '.exe' } | ForEach-Object {$\r$\n"
  FileWrite $1 "      $$p = $$_.FullName; $$locked = $$false$\r$\n"
  FileWrite $1 "      try { $$f = [System.IO.File]::Open($$p, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None); $$f.Close() } catch { $$locked = $$true }$\r$\n"
  FileWrite $1 "      if ($$locked) {$\r$\n"
  FileWrite $1 "        $$old = $$p + '.' + [System.Guid]::NewGuid().ToString('N').Substring(0,8) + '.old'$\r$\n"
  FileWrite $1 "        try { [System.IO.File]::Move($$p, $$old) } catch { try { Move-Item -LiteralPath $$p -Destination $$old -Force -ErrorAction SilentlyContinue } catch {} }$\r$\n"
  FileWrite $1 "      }$\r$\n"
  FileWrite $1 "    }$\r$\n"
  FileWrite $1 "    Get-ChildItem -Path $$dir -Filter '*.old' -ErrorAction SilentlyContinue | ForEach-Object { try { Remove-Item -LiteralPath $$_.FullName -Force -ErrorAction SilentlyContinue } catch {} }$\r$\n"
  FileWrite $1 "  }$\r$\n"
  FileWrite $1 "}$\r$\n"
  FileClose $1
  nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\stop-cores.ps1"'
  Pop $0
  Delete /REBOOTOK "$INSTDIR\core\*.old"
  Delete /REBOOTOK "$INSTDIR\*.old"
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
