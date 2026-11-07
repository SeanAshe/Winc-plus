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

; Adds winc to "Open with" for these types without taking over their default program; Windows
; only lets the user change that. Keep this list the same as FileTypes in installer.wxs.
; Registry keys ignore case; upper case gives the "JSON File" name the shell shows for a type.
!macro ForEachFileType Action
  !insertmacro ${Action} ASTRO
  !insertmacro ${Action} BASH
  !insertmacro ${Action} BAT
  !insertmacro ${Action} C
  !insertmacro ${Action} CC
  !insertmacro ${Action} CFG
  !insertmacro ${Action} CJS
  !insertmacro ${Action} CLJ
  !insertmacro ${Action} CMD
  !insertmacro ${Action} CONF
  !insertmacro ${Action} CPP
  !insertmacro ${Action} CS
  !insertmacro ${Action} CSS
  !insertmacro ${Action} CSV
  !insertmacro ${Action} CXX
  !insertmacro ${Action} DART
  !insertmacro ${Action} ENV
  !insertmacro ${Action} ERL
  !insertmacro ${Action} EX
  !insertmacro ${Action} EXS
  !insertmacro ${Action} FISH
  !insertmacro ${Action} FS
  !insertmacro ${Action} GO
  !insertmacro ${Action} GQL
  !insertmacro ${Action} GRADLE
  !insertmacro ${Action} GRAPHQL
  !insertmacro ${Action} H
  !insertmacro ${Action} HH
  !insertmacro ${Action} HPP
  !insertmacro ${Action} HS
  !insertmacro ${Action} HTM
  !insertmacro ${Action} HTML
  !insertmacro ${Action} INI
  !insertmacro ${Action} JAVA
  !insertmacro ${Action} JS
  !insertmacro ${Action} JSON
  !insertmacro ${Action} JSON5
  !insertmacro ${Action} JSONC
  !insertmacro ${Action} JSX
  !insertmacro ${Action} KT
  !insertmacro ${Action} KTS
  !insertmacro ${Action} LESS
  !insertmacro ${Action} LOG
  !insertmacro ${Action} LUA
  !insertmacro ${Action} M
  !insertmacro ${Action} MD
  !insertmacro ${Action} MJS
  !insertmacro ${Action} MM
  !insertmacro ${Action} PHP
  !insertmacro ${Action} PL
  !insertmacro ${Action} PROTO
  !insertmacro ${Action} PS1
  !insertmacro ${Action} PSM1
  !insertmacro ${Action} PY
  !insertmacro ${Action} PYI
  !insertmacro ${Action} R
  !insertmacro ${Action} RB
  !insertmacro ${Action} RS
  !insertmacro ${Action} SASS
  !insertmacro ${Action} SCALA
  !insertmacro ${Action} SCSS
  !insertmacro ${Action} SH
  !insertmacro ${Action} SQL
  !insertmacro ${Action} SVELTE
  !insertmacro ${Action} SWIFT
  !insertmacro ${Action} TF
  !insertmacro ${Action} TOML
  !insertmacro ${Action} TS
  !insertmacro ${Action} TSV
  !insertmacro ${Action} TSX
  !insertmacro ${Action} TXT
  !insertmacro ${Action} VUE
  !insertmacro ${Action} XML
  !insertmacro ${Action} YAML
  !insertmacro ${Action} YML
  !insertmacro ${Action} ZIG
  !insertmacro ${Action} ZSH
!macroend

!macro RegisterFileType Type
  WriteRegStr HKCU "Software\Classes\.${Type}\OpenWithProgids" "winc.${Type}" ""
  WriteRegStr HKCU "Software\Classes\winc.${Type}" "" "${Type} File"
  WriteRegStr HKCU "Software\Classes\winc.${Type}\DefaultIcon" "" "$INSTDIR\winc.exe,0"
  WriteRegStr HKCU "Software\Classes\winc.${Type}\shell\open\command" "" '"$INSTDIR\winc.exe" "%1"'
!macroend

!macro UnregisterFileType Type
  DeleteRegValue HKCU "Software\Classes\.${Type}\OpenWithProgids" "winc.${Type}"
  DeleteRegKey HKCU "Software\Classes\winc.${Type}"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${Type}\OpenWithProgids"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${Type}"
!macroend

; SHCNE_ASSOCCHANGED: the shell refreshes its icons and "Open with" lists.
!macro NotifyAssociationsChanged
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

; Adds the command-line launchers to the user's Path or removes them. A failure leaves winc usable.
!macro UpdateUserPath ACTION
  InitPluginsDir
  File "/oname=$PLUGINSDIR\path.ps1" "${SOURCE}\path.ps1"
  nsExec::Exec 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\path.ps1" -Action ${ACTION} -Directory "$INSTDIR\bin"'
  Pop $0
!macroend

; These files belong to processes that exit shortly after winc does, so they can still be open:
; winc-watch inside a WSL distribution, OpenConsole.exe and conpty.dll with a closing terminal.
; Waits up to 5 s; a file that stays open keeps the old copy until the next update.
!macro InstallRetrying NAME
  !define /redef RETRY ${__COUNTER__}
  SetOverwrite try
  StrCpy $1 0
  retry_install_${RETRY}:
    ClearErrors
    File "${SOURCE}\${NAME}"
    IfErrors 0 retry_installed_${RETRY}
    IntOp $1 $1 + 1
    IntCmp $1 20 retry_installed_${RETRY}
    Sleep 250
    Goto retry_install_${RETRY}
  retry_installed_${RETRY}:
  SetOverwrite on
!macroend

!macro DeleteRetrying NAME
  !define /redef RETRY ${__COUNTER__}
  StrCpy $1 0
  retry_delete_${RETRY}:
    Delete "$INSTDIR\${NAME}"
    IfFileExists "$INSTDIR\${NAME}" 0 retry_deleted_${RETRY}
    IntOp $1 $1 + 1
    IntCmp $1 20 retry_deleted_${RETRY}
    Sleep 250
    Goto retry_delete_${RETRY}
  retry_deleted_${RETRY}:
!macroend

Section "winc" SEC_MAIN
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${SOURCE}\winc.exe"
  File "${SOURCE}\winc-pty.exe"
  !insertmacro InstallRetrying "OpenConsole.exe"
  !insertmacro InstallRetrying "conpty.dll"
  !insertmacro InstallRetrying "winc-watch"
  File "${SOURCE}\winc.ico"
  File "${SOURCE}\LICENSE"
  File "${SOURCE}\NOTICE"
  File "${SOURCE}\nsis\winc-installation"
  SetOutPath "$INSTDIR\bin"
  File "${SOURCE}\bin\winc"
  File "${SOURCE}\bin\winc.cmd"
  SetOutPath "$INSTDIR"
  !insertmacro UpdateUserPath "Add"
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
  !insertmacro DeleteRetrying "OpenConsole.exe"
  !insertmacro DeleteRetrying "conpty.dll"
  !insertmacro DeleteRetrying "winc-watch"
  Delete "$INSTDIR\winc.ico"
  Delete "$INSTDIR\winc-installation"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\NOTICE"
  Delete "$INSTDIR\uninstall.exe"
  !insertmacro UpdateUserPath "Remove"
  Delete "$INSTDIR\bin\winc"
  Delete "$INSTDIR\bin\winc.cmd"
  RMDir "$INSTDIR\bin"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\DEEIX\winc"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc"
  !insertmacro ForEachFileType UnregisterFileType
  DeleteRegKey HKCU "Software\Classes\Applications\winc.exe"
  !insertmacro NotifyAssociationsChanged
SectionEnd
