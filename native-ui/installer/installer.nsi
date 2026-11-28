; Standalone NSIS installer for the native EasyTier client.
; Build with: makensis /DAPP_VERSION=0.1.5 /DARCH=x64 /DSTAGE=... /DOUTFILE=... /DICON=... installer.nsi
; Registers EasyTierService for the original interactive user (named-pipe ACL),
; stops/removes any previous service before upgrading, and cleans up on uninstall.

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel admin

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!ifndef APP_VERSION
!define APP_VERSION "0.1.5"
!endif
!ifndef ARCH
!define ARCH "x64"
!endif
!ifndef STAGE
!error "STAGE define is required (/DSTAGE=<staged files dir>)"
!endif
!ifndef OUTFILE
!error "OUTFILE define is required (/DOUTFILE=<installer.exe path>)"
!endif
!ifndef ICON
!define ICON "icon.ico"
!endif

Name "EasyTier 原生客户端 ${APP_VERSION} (${ARCH})"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\EasyTier"
InstallDirRegKey HKLM "Software\EasyTierNative" "InstallDir"
BrandingText "EasyTier Native Client"
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductName" "EasyTier"
VIAddVersionKey /LANG=2052 "FileDescription" "EasyTier 原生客户端安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "EasyTier"

!define MUI_ABORTWARNING
!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

; Stop the service (if present), kill stack processes, then delete the SCM entry.
!macro STOP_SERVICE_BODY
  ; Uninstaller pages do not run MUI's plugin-dir init in /S mode; without this
  ; FileOpen/nsExec target an empty $PLUGINSDIR and the cleanup silently no-ops.
  InitPluginsDir
  DetailPrint "停止 EasyTier 服务与进程..."
  FileOpen $1 "$PLUGINSDIR\stop-all.ps1" w
  FileWrite $1 "if (Get-Service EasyTierService -ErrorAction SilentlyContinue) {$\r$\n"
  FileWrite $1 "  sc.exe stop EasyTierService | Out-Null$\r$\n"
  FileWrite $1 "  for ($$i = 0; $$i -lt 30; $$i++) {$\r$\n"
  FileWrite $1 "    if ((Get-Service EasyTierService -ErrorAction SilentlyContinue).Status -eq 'Stopped') { break }$\r$\n"
  FileWrite $1 "    Start-Sleep -Milliseconds 500$\r$\n"
  FileWrite $1 "  }$\r$\n"
  FileWrite $1 "}$\r$\n"
  FileWrite $1 "@('easytier-native-ui', 'easytier-service', 'easytier-core', 'easytier-cli') | ForEach-Object {$\r$\n"
  FileWrite $1 "  $$p = Get-Process -Name $$_ -ErrorAction SilentlyContinue$\r$\n"
  FileWrite $1 "  if ($$p) {$\r$\n"
  FileWrite $1 "    $$p | Stop-Process -Force -ErrorAction SilentlyContinue$\r$\n"
  FileWrite $1 "    for ($$j = 0; $$j -lt 40; $$j++) {$\r$\n"
  FileWrite $1 "      if (-not (Get-Process -Name $$_ -ErrorAction SilentlyContinue)) { break }$\r$\n"
  FileWrite $1 "      Start-Sleep -Milliseconds 250$\r$\n"
  FileWrite $1 "    }$\r$\n"
  FileWrite $1 "  }$\r$\n"
  FileWrite $1 "}$\r$\n"
  FileWrite $1 "@('WinDivert', 'WinDivert14', 'WinDivert22') | ForEach-Object {$\r$\n"
  FileWrite $1 "  & sc.exe stop $$_ 2>$$null; & sc.exe delete $$_ 2>$$null$\r$\n"
  FileWrite $1 "}$\r$\n"
  FileWrite $1 "Start-Sleep -Milliseconds 500$\r$\n"
  FileWrite $1 "if (Get-Service EasyTierService -ErrorAction SilentlyContinue) {$\r$\n"
  FileWrite $1 "  for ($$i = 0; $$i -lt 15; $$i++) {$\r$\n"
  FileWrite $1 "    sc.exe delete EasyTierService | Out-Null$\r$\n"
  FileWrite $1 "    Start-Sleep -Milliseconds 800$\r$\n"
  FileWrite $1 "    if (-not (Get-Service EasyTierService -ErrorAction SilentlyContinue)) { break }$\r$\n"
  FileWrite $1 "  }$\r$\n"
  FileWrite $1 "}$\r$\n"
  FileClose $1
  ${IfNot} ${FileExists} "$PLUGINSDIR\stop-all.ps1"
    DetailPrint "警告：无法写出停止脚本，跳过服务与进程清理。"
  ${Else}
    nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\stop-all.ps1"'
    Pop $0
  ${EndIf}
!macroend

Function StopServiceAndCores
  !insertmacro STOP_SERVICE_BODY
FunctionEnd

Function un.StopServiceAndCores
  !insertmacro STOP_SERVICE_BODY
FunctionEnd

; Resolve the original interactive user's SID via the explorer process owner.
; The elevated admin token would yield the wrong SID for named-pipe ACLs.
Function ResolveUserSid
  InitPluginsDir
  StrCpy $R8 ""
  FileOpen $1 "$PLUGINSDIR\resolve-sid.ps1" w
  FileWrite $1 "$$o = Get-CimInstance Win32_Process | Where-Object Name -eq 'explorer.exe' | Select-Object -First 1$\r$\n"
  FileWrite $1 "if (-not $$o) { exit 1 }$\r$\n"
  FileWrite $1 "$$u = Invoke-CimMethod -InputObject $$o -MethodName GetOwner$\r$\n"
  FileWrite $1 "(New-Object System.Security.Principal.NTAccount($$u.Domain, $$u.User)).Translate([System.Security.Principal.SecurityIdentifier]).Value$\r$\n"
  FileClose $1
  nsExec::ExecToStack '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\resolve-sid.ps1"'
  Pop $2
  Pop $3
  ${If} $2 = 0
    Push $3
    Call TrimSid
    Pop $R8
    ${If} $R8 == "S-1-5-18"
      StrCpy $R8 ""
    ${EndIf}
  ${EndIf}
FunctionEnd

Function TrimSid
  Exch $R0
  Push $R1
  trim_head:
    StrCpy $R1 "$R0" 1
    ${If} $R1 == " "
    ${OrIf} $R1 == "$\r"
    ${OrIf} $R1 == "$\n"
      StrCpy $R0 "$R0" "" 1
      Goto trim_head
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

Function InstallService
  Call ResolveUserSid
  ${If} $R8 == ""
    DetailPrint "警告：无法解析交互用户 SID，跳过服务注册（可稍后在应用设置中安装服务）。"
    Return
  ${EndIf}
  ${IfNot} ${FileExists} "$INSTDIR\easytier-service.exe"
    DetailPrint "警告：缺少 easytier-service.exe，服务未注册。"
    Return
  ${EndIf}
  FileOpen $1 "$INSTDIR\interactive-user.sid" w
  FileWrite $1 "$R8$\r$\n"
  FileClose $1
  DetailPrint "注册 EasyTier Service..."
  nsExec::ExecToLog 'sc.exe create EasyTierService binPath= "\"$INSTDIR\easytier-service.exe\" --interactive-user-sid=$R8" start= auto DisplayName= "EasyTier Service"'
  Pop $0
  ${If} $0 = 0
    nsExec::ExecToLog 'sc.exe description EasyTierService "EasyTier background service"'
    nsExec::ExecToLog 'sc.exe start EasyTierService'
    Pop $0
    ${If} $0 = 0
      DetailPrint "EasyTier Service 已启动。"
    ${Else}
      DetailPrint "警告：服务已注册但启动失败（错误 $0），可在应用设置中手动启动。"
    ${EndIf}
  ${Else}
    DetailPrint "警告：服务注册失败（错误 $0），可在应用设置中稍后安装。"
  ${EndIf}
FunctionEnd

Function .onInit
  ; NSIS >= 3.11 rejects SetRegView at global scope; the stub is 32-bit, so
  ; both InstallDirRegKey and HKLM writes need the 64-bit view explicitly.
  SetRegView 64
  IfSilent init_done
  nsExec::Exec 'sc.exe query EasyTierService'
  Pop $0
  ${If} $0 = 0
    MessageBox MB_OKCANCEL|MB_ICONINFORMATION "检测到已安装的 EasyTierService。安装程序会先停止旧服务与内核进程，然后升级文件并重新注册服务。" IDOK init_done
    Abort
  ${EndIf}
  init_done:
FunctionEnd

Section "EasyTier Native Client" SecMain
  SetRegView 64
  Call StopServiceAndCores

  SetOutPath "$INSTDIR"
  File /r "${STAGE}\*"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\EasyTierNative" "InstallDir" "$INSTDIR"

  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "DisplayName" "EasyTier 原生客户端 (${ARCH})"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "Publisher" "EasyTier"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "DisplayIcon" "$INSTDIR\easytier-native-ui.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative" "NoRepair" 1

  SetShellVarContext all
  CreateDirectory "$SMPROGRAMS\EasyTier"
  CreateShortcut "$SMPROGRAMS\EasyTier\EasyTier.lnk" "$INSTDIR\easytier-native-ui.exe"
  CreateShortcut "$SMPROGRAMS\EasyTier\卸载 EasyTier.lnk" "$INSTDIR\uninstall.exe"

  Call InstallService
SectionEnd

Section "Uninstall"
  SetRegView 64
  Call un.StopServiceAndCores

  ; TerminateProcess returns before the kernel releases the image section, and
  ; AV scanners (Huorong) may hold a fresh handle; retry before giving up.
  StrCpy $0 0
  un_delete_service:
    Delete "$INSTDIR\easytier-service.exe"
    ${IfNot} ${FileExists} "$INSTDIR\easytier-service.exe"
      Goto un_delete_done
    ${EndIf}
    IntOp $0 $0 + 1
    ${If} $0 < 20
      Sleep 300
      Goto un_delete_service
    ${EndIf}
    DetailPrint "警告：easytier-service.exe 仍被占用，已安排在重启后删除。"
    Delete /REBOOTOK "$INSTDIR\easytier-service.exe"
  un_delete_done:

  Delete /REBOOTOK "$INSTDIR\easytier-native-ui.exe"
  Delete "$INSTDIR\interactive-user.sid"
  RMDir /r /REBOOTOK "$INSTDIR\core"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EasyTierNative"
  DeleteRegKey HKLM "Software\EasyTierNative"

  SetShellVarContext all
  Delete "$SMPROGRAMS\EasyTier\EasyTier.lnk"
  Delete "$SMPROGRAMS\EasyTier\卸载 EasyTier.lnk"
  RMDir "$SMPROGRAMS\EasyTier"
SectionEnd
