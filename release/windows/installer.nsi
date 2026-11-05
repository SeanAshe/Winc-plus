; Native current-user installation. Updater invocations use /S and /D=<installation directory>.
Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma
!include "MUI2.nsh"

Name "winc"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\winc"
InstallDirRegKey HKCU "Software\DEEIX\winc" "InstallDir"
Icon "${SOURCE}\winc.ico"
UninstallIcon "${SOURCE}\winc.ico"
VIProductVersion "${VIVERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "winc"
VIAddVersionKey /LANG=1033 "FileDescription" "winc installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "DEEIX"
!ifdef SIGN_SCRIPT
  !uninstfinalize 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "${SIGN_SCRIPT}" -FilePath "%1"' = 0
!endif
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SOURCE}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\winc.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

Section "winc" SEC_MAIN
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${SOURCE}\winc.exe"
  File "${SOURCE}\winc-pty.exe"
  File "${SOURCE}\winc.ico"
  File "${SOURCE}\LICENSE"
  File "${SOURCE}\NOTICE"
  File "${SOURCE}\nsis\winc-installation"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\DEEIX\winc" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "DisplayName" "winc"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "Publisher" "DEEIX"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "DisplayIcon" "$INSTDIR\winc.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "NoRepair" 1
  CreateShortcut "$SMPROGRAMS\winc.lnk" "$INSTDIR\winc.exe" "" "$INSTDIR\winc.ico"
  ; ProgIDs, file icons and Open With entries. Failure here must not roll back the copy.
  ClearErrors
  ExecWait '"$INSTDIR\winc.exe" --register-file-types'
  ClearErrors
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  IfFileExists "$INSTDIR\winc.exe" 0 uninstall_files
    ClearErrors
    ExecWait '"$INSTDIR\winc.exe" --unregister-file-types'
    ClearErrors
uninstall_files:
  Delete "$SMPROGRAMS\winc.lnk"
  Delete "$INSTDIR\winc.exe"
  Delete "$INSTDIR\winc-pty.exe"
  Delete "$INSTDIR\winc.ico"
  Delete "$INSTDIR\winc-installation"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\NOTICE"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\DEEIX\winc"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc"
SectionEnd
