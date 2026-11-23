; Native current-user installation. Updater invocations use /S and /D=<installation directory>.
Unicode true
RequestExecutionLevel user
; DPI-aware, so Windows does not stretch the installer on scaled displays and blur its text. NSIS
; keeps the checkboxes on the components page at 16 pixels.
ManifestDPIAware true
SetCompressor /SOLID lzma
!include "MUI2.nsh"
!include "Sections.nsh"
!include "FileFunc.nsh"

Name "Winc"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\winc"
InstallDirRegKey HKCU "Software\DEEIX\winc" "InstallDir"
Icon "${SOURCE}\winc.ico"
UninstallIcon "${SOURCE}\winc.ico"
VIProductVersion "${VIVERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "Winc"
VIAddVersionKey /LANG=1033 "FileDescription" "Winc installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "DEEIX"
!ifdef SIGN_SCRIPT
  !uninstfinalize 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "${SIGN_SCRIPT}" -FilePath "%1"' = 0
!endif
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SOURCE}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\winc.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

; The optional section that lists winc under the supported file types.
LangString FileTypesSection ${LANG_ENGLISH} "Associate file types"
LangString FileTypesSection ${LANG_SIMPCHINESE} "关联文件类型"
LangString FileTypesText ${LANG_ENGLISH} "Lists Winc among the programs for source and text files. A type opened with Winc by default then shows its own document icon. Windows may ask once which program to use for these files."
LangString FileTypesText ${LANG_SIMPCHINESE} "把 Winc 列为源代码和文本文件的可选程序。设为默认程序的类型会显示各自的文档图标。Windows 可能会询问一次用哪个程序打开这些文件。"

; The optional section that adds winc to the Explorer context menu.
LangString ContextMenuSection ${LANG_ENGLISH} "Explorer context menu"
LangString ContextMenuSection ${LANG_SIMPCHINESE} "资源管理器右键菜单"
LangString ContextMenuText ${LANG_ENGLISH} "Adds Open with Winc to the context menu of files and Open in Winc to that of folders. It can be turned off later in Winc's settings."
LangString ContextMenuText ${LANG_SIMPCHINESE} "在文件的右键菜单中加入“通过 Winc 打开”，在文件夹的右键菜单中加入“在 Winc 中打开”。之后可以在 Winc 的设置中关闭。"

; Explorer context menu labels, in the installer's language.
LangString OpenFileVerb ${LANG_ENGLISH} "Open with Winc"
LangString OpenFileVerb ${LANG_SIMPCHINESE} "通过 Winc 打开"
LangString OpenFolderVerb ${LANG_ENGLISH} "Open in Winc"
LangString OpenFolderVerb ${LANG_SIMPCHINESE} "在 Winc 中打开"

; Types winc can open, listed under Applications\winc.exe\SupportedTypes: winc appears in
; "Open with" > "Choose another app" for them. Nothing is written under the types themselves unless
; the optional file types section is selected, so Windows keeps their default program and icon and
; does not ask which program to use; scripts still run on double-click. Keep this list the same as
; FileTypes in installer.wxs.
; Registry keys ignore case; upper case gives the "JSON File" name the shell shows for a type.
; The second argument names the type's document icon in the icons folder (release/windows/icons).
!macro ForEachFileType Action
  !insertmacro ${Action} ASH script
  !insertmacro ${Action} ASTRO markup
  !insertmacro ${Action} ATOM markup
  !insertmacro ${Action} BASH script
  !insertmacro ${Action} BB code
  !insertmacro ${Action} BZL python
  !insertmacro ${Action} C c
  !insertmacro ${Action} CC c
  !insertmacro ${Action} CFG config
  !insertmacro ${Action} CJS javascript
  !insertmacro ${Action} CLJ code
  !insertmacro ${Action} CLJC code
  !insertmacro ${Action} CLJS code
  !insertmacro ${Action} CLS markup
  !insertmacro ${Action} CMAKE code
  !insertmacro ${Action} CODE-WORKSPACE project
  !insertmacro ${Action} CONF config
  !insertmacro ${Action} CPP c
  !insertmacro ${Action} CPPM c
  !insertmacro ${Action} CS code
  !insertmacro ${Action} CSPROJ markup
  !insertmacro ${Action} CSS style
  !insertmacro ${Action} CSV table
  !insertmacro ${Action} CSX code
  !insertmacro ${Action} CTS typescript
  !insertmacro ${Action} CU c
  !insertmacro ${Action} CUH c
  !insertmacro ${Action} CXX c
  !insertmacro ${Action} DART code
  !insertmacro ${Action} DESKTOP config
  !insertmacro ${Action} DIFF text
  !insertmacro ${Action} DOCKERFILE config
  !insertmacro ${Action} DTX markup
  !insertmacro ${Action} EDN code
  !insertmacro ${Action} ENV config
  !insertmacro ${Action} EPUB text
  !insertmacro ${Action} ERL code
  !insertmacro ${Action} ESCRIPT code
  !insertmacro ${Action} ETS code
  !insertmacro ${Action} EX code
  !insertmacro ${Action} EXS code
  !insertmacro ${Action} FISH script
  !insertmacro ${Action} FS code
  !insertmacro ${Action} FSI code
  !insertmacro ${Action} FSPROJ markup
  !insertmacro ${Action} FSX code
  !insertmacro ${Action} GEMSPEC code
  !insertmacro ${Action} GEOJSON json
  !insertmacro ${Action} GO go
  !insertmacro ${Action} GQL code
  !insertmacro ${Action} GRADLE code
  !insertmacro ${Action} GRAPHQL code
  !insertmacro ${Action} GRAPHQLS code
  !insertmacro ${Action} GROOVY code
  !insertmacro ${Action} H c
  !insertmacro ${Action} HCL code
  !insertmacro ${Action} HH c
  !insertmacro ${Action} HPP c
  !insertmacro ${Action} HRL code
  !insertmacro ${Action} HS code
  !insertmacro ${Action} HSC code
  !insertmacro ${Action} HTM markup
  !insertmacro ${Action} HTML markup
  !insertmacro ${Action} HXX c
  !insertmacro ${Action} INF config
  !insertmacro ${Action} INI config
  !insertmacro ${Action} INL c
  !insertmacro ${Action} INO c
  !insertmacro ${Action} IPP c
  !insertmacro ${Action} IPR project
  !insertmacro ${Action} IXX c
  !insertmacro ${Action} JAVA code
  !insertmacro ${Action} JBUILDER code
  !insertmacro ${Action} JL code
  !insertmacro ${Action} JS javascript
  !insertmacro ${Action} JSON json
  !insertmacro ${Action} JSON5 json
  !insertmacro ${Action} JSONC json
  !insertmacro ${Action} JSONL json
  !insertmacro ${Action} JSX javascript
  !insertmacro ${Action} KML markup
  !insertmacro ${Action} KSH script
  !insertmacro ${Action} KT code
  !insertmacro ${Action} KTS code
  !insertmacro ${Action} LESS style
  !insertmacro ${Action} LOG text
  !insertmacro ${Action} LTX markup
  !insertmacro ${Action} LUA code
  !insertmacro ${Action} LUAU code
  !insertmacro ${Action} M code
  !insertmacro ${Action} MARKDOWN markdown
  !insertmacro ${Action} MD markdown
  !insertmacro ${Action} MDOWN markdown
  !insertmacro ${Action} MJS javascript
  !insertmacro ${Action} MK script
  !insertmacro ${Action} MKD markdown
  !insertmacro ${Action} MKDN markdown
  !insertmacro ${Action} ML code
  !insertmacro ${Action} MLI code
  !insertmacro ${Action} MLL code
  !insertmacro ${Action} MLY code
  !insertmacro ${Action} MM code
  !insertmacro ${Action} MTS typescript
  !insertmacro ${Action} NDJSON json
  !insertmacro ${Action} NIX code
  !insertmacro ${Action} NOMAD code
  !insertmacro ${Action} NUSPEC markup
  !insertmacro ${Action} PATCH text
  !insertmacro ${Action} PDF text
  !insertmacro ${Action} PHP code
  !insertmacro ${Action} PHP3 code
  !insertmacro ${Action} PHP4 code
  !insertmacro ${Action} PHP5 code
  !insertmacro ${Action} PHP8 code
  !insertmacro ${Action} PHPS code
  !insertmacro ${Action} PHTML code
  !insertmacro ${Action} PL code
  !insertmacro ${Action} PLIST markup
  !insertmacro ${Action} PM code
  !insertmacro ${Action} POD code
  !insertmacro ${Action} PODSPEC code
  !insertmacro ${Action} PROPERTIES config
  !insertmacro ${Action} PROPS markup
  !insertmacro ${Action} PROTO code
  !insertmacro ${Action} PS1 script
  !insertmacro ${Action} PSD1 script
  !insertmacro ${Action} PSGI code
  !insertmacro ${Action} PSM1 script
  !insertmacro ${Action} PY python
  !insertmacro ${Action} PYI python
  !insertmacro ${Action} R code
  !insertmacro ${Action} RAKE code
  !insertmacro ${Action} RB code
  !insertmacro ${Action} RBI code
  !insertmacro ${Action} RESX markup
  !insertmacro ${Action} ROCKSPEC code
  !insertmacro ${Action} RPROFILE code
  !insertmacro ${Action} RS rust
  !insertmacro ${Action} RSS markup
  !insertmacro ${Action} SASS style
  !insertmacro ${Action} SC code
  !insertmacro ${Action} SCALA code
  !insertmacro ${Action} SCSS style
  !insertmacro ${Action} SERVICE config
  !insertmacro ${Action} SH script
  !insertmacro ${Action} SQL code
  !insertmacro ${Action} STAR python
  !insertmacro ${Action} STORYBOARD markup
  !insertmacro ${Action} STY markup
  !insertmacro ${Action} SUBLIME-PROJECT project
  !insertmacro ${Action} SVELTE markup
  !insertmacro ${Action} SVG markup
  !insertmacro ${Action} SWIFT code
  !insertmacro ${Action} T code
  !insertmacro ${Action} TAB table
  !insertmacro ${Action} TARGETS markup
  !insertmacro ${Action} TEX markup
  !insertmacro ${Action} TF code
  !insertmacro ${Action} TFTEST code
  !insertmacro ${Action} TFVARS code
  !insertmacro ${Action} TOML config
  !insertmacro ${Action} TPP c
  !insertmacro ${Action} TS typescript
  !insertmacro ${Action} TSV table
  !insertmacro ${Action} TSX typescript
  !insertmacro ${Action} TXT text
  !insertmacro ${Action} VBPROJ markup
  !insertmacro ${Action} VCXPROJ markup
  !insertmacro ${Action} VUE markup
  !insertmacro ${Action} WEBMANIFEST json
  !insertmacro ${Action} WINC-PROJECT project
  !insertmacro ${Action} WSDL markup
  !insertmacro ${Action} WXML markup
  !insertmacro ${Action} WXS code
  !insertmacro ${Action} WXSS style
  !insertmacro ${Action} XAML markup
  !insertmacro ${Action} XCSCHEME markup
  !insertmacro ${Action} XHTML markup
  !insertmacro ${Action} XIB markup
  !insertmacro ${Action} XML markup
  !insertmacro ${Action} XSD markup
  !insertmacro ${Action} XSL markup
  !insertmacro ${Action} XSLT markup
  !insertmacro ${Action} YAML config
  !insertmacro ${Action} YML config
  !insertmacro ${Action} ZIG code
  !insertmacro ${Action} ZON code
  !insertmacro ${Action} ZSH script
!macroend

; Types earlier versions registered: unregistered on install and uninstall, so upgrading also repairs
; them. Installing over an older version leaves its registry entries for types no longer listed.
!macro ForEachRetiredFileType Action
  !insertmacro ${Action} BAT file
  !insertmacro ${Action} CMD file
!macroend

; The winc.<type> ProgID is not listed under any type. Earlier versions listed it in the type's
; OpenWithProgids, and a user who then chose winc as the default program has a choice that names
; this ProgID; keeping it keeps that choice working.
; The ProgID carries the type's own document icon. Explorer shows it for a type whose chosen program
; is this ProgID; a choice made through "Open with" names winc.exe instead and shows its icon.
!macro RegisterFileType Type Icon
  WriteRegStr HKCU "Software\Classes\Applications\winc.exe\SupportedTypes" ".${Type}" ""
  WriteRegStr HKCU "Software\Classes\winc.${Type}" "" "${Type} File"
  WriteRegStr HKCU "Software\Classes\winc.${Type}\DefaultIcon" "" "$INSTDIR\icons\${Icon}.ico"
  WriteRegStr HKCU "Software\Classes\winc.${Type}\shell\open\command" "" '"$INSTDIR\winc.exe" "%1"'
!macroend

; Earlier versions added "Open with Winc" to each type under SystemFileAssociations. The context
; menu is now one entry for all files (RegisterContextMenu), so the per-type verbs are removed.
!macro RemoveFileTypeVerb Type Icon
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.${Type}\shell\winc"
  DeleteRegKey /ifempty HKCU "Software\Classes\SystemFileAssociations\.${Type}\shell"
  DeleteRegKey /ifempty HKCU "Software\Classes\SystemFileAssociations\.${Type}"
!macroend

; Removes the entry earlier versions added to the type's OpenWithProgids. A new handler listed there
; makes Windows ask which program to use the next time such a file is opened.
!macro UnlistFileType Type Icon
  DeleteRegValue HKCU "Software\Classes\.${Type}\OpenWithProgids" "winc.${Type}"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${Type}\OpenWithProgids"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${Type}"
!macroend

; Lists the ProgID under the type, on the user's request (the optional "file types" section). The
; type's default program is not changed; choosing winc for it then shows the type's document icon.
!macro ListFileType Type Icon
  WriteRegStr HKCU "Software\Classes\.${Type}\OpenWithProgids" "winc.${Type}" ""
!macroend

!macro UnregisterFileType Type Icon
  !insertmacro RemoveFileTypeVerb ${Type} ${Icon}
  !insertmacro UnlistFileType ${Type} ${Icon}
  DeleteRegKey HKCU "Software\Classes\winc.${Type}"
!macroend

; .winc-project is Winc's own project file: the extension opens with winc by default. Nothing else
; claims it, so Windows does not ask which program to use.
!macro RegisterProjectType
  WriteRegStr HKCU "Software\Classes\.winc-project" "" "winc.WINC-PROJECT"
!macroend

; Removes the default program only while it is still winc.
!macro UnregisterProjectType
  ReadRegStr $0 HKCU "Software\Classes\.winc-project" ""
  StrCmp $0 "winc.WINC-PROJECT" 0 +2
    DeleteRegValue HKCU "Software\Classes\.winc-project" ""
  DeleteRegKey /ifempty HKCU "Software\Classes\.winc-project"
!macroend

; "Open in Winc" on a folder and on the empty space inside one (also the desktop); %V is that folder.
!macro RegisterFolderVerb Key
  WriteRegStr HKCU "Software\Classes\${Key}\shell\winc" "" "$(OpenFolderVerb)"
  WriteRegStr HKCU "Software\Classes\${Key}\shell\winc" "Icon" "$INSTDIR\winc.exe,0"
  WriteRegStr HKCU "Software\Classes\${Key}\shell\winc\command" "" '"$INSTDIR\winc.exe" "%V"'
!macroend

!macro UnregisterFolderVerb Key
  DeleteRegKey HKCU "Software\Classes\${Key}\shell\winc"
!macroend

; The Explorer context menu: one "Open with Winc" for every file, under the * class, which leaves
; each type's default program alone, and "Open in Winc" on a folder and on the empty space inside
; one (also the desktop), where %V is that folder. Windows 11 shows them under "Show more options".
; Winc's settings write the same keys, so the entries can be turned off and on there.
!macro RegisterContextMenu
  WriteRegStr HKCU "Software\Classes\*\shell\winc" "" "$(OpenFileVerb)"
  WriteRegStr HKCU "Software\Classes\*\shell\winc" "Icon" "$INSTDIR\winc.exe,0"
  WriteRegStr HKCU "Software\Classes\*\shell\winc\command" "" '"$INSTDIR\winc.exe" "%1"'
  !insertmacro RegisterFolderVerb "Directory"
  !insertmacro RegisterFolderVerb "Directory\Background"
!macroend

!macro UnregisterContextMenu
  DeleteRegKey HKCU "Software\Classes\*\shell\winc"
  !insertmacro UnregisterFolderVerb "Directory"
  !insertmacro UnregisterFolderVerb "Directory\Background"
!macroend

; The winc: URL scheme: winc://file/<absolute path>[:line[:column]] opens a file or folder in winc,
; which receives the whole link as its argument.
!macro RegisterUrlScheme
  WriteRegStr HKCU "Software\Classes\winc" "" "URL:Winc"
  WriteRegStr HKCU "Software\Classes\winc" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\winc\DefaultIcon" "" "$INSTDIR\winc.exe,0"
  WriteRegStr HKCU "Software\Classes\winc\shell\open\command" "" '"$INSTDIR\winc.exe" "%1"'
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

Section "Winc" SEC_MAIN
  SectionIn RO
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${SOURCE}\winc.exe"
  File "${SOURCE}\winc-pty.exe"
  !insertmacro InstallRetrying "OpenConsole.exe"
  !insertmacro InstallRetrying "conpty.dll"
  !insertmacro InstallRetrying "winc-watch"
  File "${SOURCE}\winc-server-Linux-x86_64"
  File "${SOURCE}\winc-server-Linux-aarch64"
  File "${SOURCE}\winc.ico"
  File "${SOURCE}\LICENSE"
  File "${SOURCE}\NOTICE"
  File "${SOURCE}\nsis\winc-installation"
  SetOutPath "$INSTDIR\bin"
  File "${SOURCE}\bin\winc"
  File "${SOURCE}\bin\winc.cmd"
  SetOutPath "$INSTDIR\icons"
  File "${SOURCE}\icons\*.ico"
  SetOutPath "$INSTDIR"
  !insertmacro UpdateUserPath "Add"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\DEEIX\winc" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "DisplayName" "Winc"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "Publisher" "DEEIX"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "DisplayIcon" "$INSTDIR\winc.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "NoRepair" 1
  ; The Start menu shows a shortcut's file name, and creating Winc.lnk over an existing winc.lnk
  ; keeps the existing file name, so any shortcut of that name is removed first.
  Delete "$SMPROGRAMS\winc.lnk"
  CreateShortcut "$SMPROGRAMS\Winc.lnk" "$INSTDIR\winc.exe" "" "$INSTDIR\winc.ico"
  WriteRegStr HKCU "Software\Classes\Applications\winc.exe" "FriendlyAppName" "Winc"
  WriteRegStr HKCU "Software\Classes\Applications\winc.exe\DefaultIcon" "" "$INSTDIR\winc.exe,0"
  WriteRegStr HKCU "Software\Classes\Applications\winc.exe\shell\open\command" "" '"$INSTDIR\winc.exe" "%1"'
  ; This fork also registers ProgIDs, capabilities, and (when nothing else owns the type)
  ; an Open With entry plus the folder menu. That runs first. The steps below then apply this
  ; installer's choices: listings follow the file-types section, and the context menu stays off
  ; unless that section is selected. Failure here must not roll back the copy.
  ClearErrors
  ExecWait '"$INSTDIR\winc.exe" --register-file-types'
  ClearErrors
  !insertmacro ForEachRetiredFileType UnregisterFileType
  !insertmacro ForEachFileType UnlistFileType
  !insertmacro ForEachFileType RemoveFileTypeVerb
  !insertmacro ForEachFileType RegisterFileType
  !insertmacro RegisterProjectType
  !insertmacro RegisterUrlScheme
  ; The optional sections below record the choices again when they are selected.
  !insertmacro UnregisterContextMenu
  WriteRegDWORD HKCU "Software\DEEIX\winc" "ContextMenu" 0
  WriteRegDWORD HKCU "Software\DEEIX\winc" "FileTypes" 0
  !insertmacro NotifyAssociationsChanged
SectionEnd

; Off by default in a new installation.
Section /o "$(ContextMenuSection)" SEC_CONTEXT_MENU
  !insertmacro RegisterContextMenu
  WriteRegDWORD HKCU "Software\DEEIX\winc" "ContextMenu" 1
  !insertmacro NotifyAssociationsChanged
SectionEnd

; On by default in a new installation. A handler listed under a type makes Windows ask once which
; program to use.
Section "$(FileTypesSection)" SEC_FILE_TYPES
  !insertmacro ForEachFileType ListFileType
  WriteRegDWORD HKCU "Software\DEEIX\winc" "FileTypes" 1
  !insertmacro NotifyAssociationsChanged
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_CONTEXT_MENU} "$(ContextMenuText)"
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_FILE_TYPES} "$(FileTypesText)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; The earlier choices are kept: updates run silently and show no page. /FILETYPES=1 or /FILETYPES=0
; and /CONTEXTMENU=1 or /CONTEXTMENU=0 on the command line, before /D, set them for a silent
; installation. An installation from before the choices were recorded keeps what it had: the context
; menu, which was always added, and no listing under the file types.
Function .onInit
  ReadRegStr $3 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc" "DisplayName"
  ${GetParameters} $1

  ReadRegDWORD $0 HKCU "Software\DEEIX\winc" "ContextMenu"
  StrCmp $0 "" 0 context_menu_recorded
  StrCmp $3 "" context_menu_recorded
  StrCpy $0 "1"
  context_menu_recorded:
  ClearErrors
  ${GetOptions} $1 "/CONTEXTMENU=" $2
  IfErrors context_menu_decided
  StrCpy $0 $2
  context_menu_decided:
  StrCmp $0 "1" 0 context_menu_done
  !insertmacro SelectSection ${SEC_CONTEXT_MENU}
  context_menu_done:

  ReadRegDWORD $0 HKCU "Software\DEEIX\winc" "FileTypes"
  StrCmp $0 "" 0 file_types_recorded
  StrCmp $3 "" file_types_recorded
  StrCpy $0 "0"
  file_types_recorded:
  ClearErrors
  ${GetOptions} $1 "/FILETYPES=" $2
  IfErrors file_types_decided
  StrCpy $0 $2
  file_types_decided:
  StrCmp $0 "0" 0 file_types_done
  !insertmacro UnselectSection ${SEC_FILE_TYPES}
  file_types_done:
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  IfFileExists "$INSTDIR\winc.exe" 0 uninstall_files
    ClearErrors
    ExecWait '"$INSTDIR\winc.exe" --unregister-file-types'
    ClearErrors
uninstall_files:
  Delete "$SMPROGRAMS\Winc.lnk"
  Delete "$INSTDIR\winc.exe"
  Delete "$INSTDIR\winc-pty.exe"
  !insertmacro DeleteRetrying "OpenConsole.exe"
  !insertmacro DeleteRetrying "conpty.dll"
  !insertmacro DeleteRetrying "winc-watch"
  Delete "$INSTDIR\winc-server-Linux-x86_64"
  Delete "$INSTDIR\winc-server-Linux-aarch64"
  Delete "$INSTDIR\winc.ico"
  Delete "$INSTDIR\winc-installation"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\NOTICE"
  Delete "$INSTDIR\uninstall.exe"
  !insertmacro UpdateUserPath "Remove"
  Delete "$INSTDIR\bin\winc"
  Delete "$INSTDIR\bin\winc.cmd"
  RMDir "$INSTDIR\bin"
  Delete "$INSTDIR\icons\*.ico"
  RMDir "$INSTDIR\icons"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\DEEIX\winc"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\winc"
  !insertmacro ForEachFileType UnregisterFileType
  !insertmacro UnregisterProjectType
  !insertmacro ForEachRetiredFileType UnregisterFileType
  !insertmacro UnregisterContextMenu
  DeleteRegKey HKCU "Software\Classes\Applications\winc.exe"
  DeleteRegKey HKCU "Software\Classes\winc"
  !insertmacro NotifyAssociationsChanged
SectionEnd
