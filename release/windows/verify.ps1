param([string]$Target = 'x86_64-pc-windows-msvc')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$version = (& node "$PSScriptRoot/../common/version.mjs").Trim()
if ($LASTEXITCODE -ne 0) { throw 'cannot read the VERSION file' }
$bundle = "$root/target/$Target/release/bundle"
$work = Join-Path $env:RUNNER_TEMP 'winc-installer-verification'
New-Item -ItemType Directory -Force $work | Out-Null
$update = "$root/crates/winc-app/src/updates/installer/update.ps1"
# The NSIS installer changes the user's Path and the MSI changes the machine's.
function Get-CommandPath([string]$Kind) {
    $scope = if ($Kind -eq 'nsis') { 'User' } else { 'Machine' }
    @([Environment]::GetEnvironmentVariable('Path', $scope) -split ';' | ForEach-Object { $_.TrimEnd('\') })
}
# The executable carries the application icon and one icon per file type. Explorer and the
# file-properties header use a file-type icon; Open with uses the application icon.
$shell = Add-Type -Name Shell -Namespace WincVerify -PassThru -MemberDefinition '[DllImport("shell32.dll", CharSet = CharSet.Unicode)] public static extern uint ExtractIconEx(string file, int index, IntPtr large, IntPtr small, uint count);'
if ($shell::ExtractIconEx("$root/target/$Target/release/winc.exe", -1, [IntPtr]::Zero, [IntPtr]::Zero, 0) -lt 2) { throw 'winc.exe is missing the application icon or the file-type icons' }
foreach ($kind in @('nsis', 'msi')) {
    $directory = Join-Path $work "$kind installation with spaces"
    $suffix = if ($kind -eq 'nsis') { '-setup.exe' } else { '.msi' }
    # msiexec cannot open a package path written with forward slashes (error 1619).
    $setup = (Resolve-Path "$bundle/$kind/Winc-$version-windows-x64$suffix").Path
    $classes = if ($kind -eq 'nsis') { 'HKCU:\Software\Classes' } else { 'HKLM:\Software\Classes' }
    # The Start menu and Windows' app lists show the name Winc. The NSIS shortcut goes in the user's
    # Start menu, the MSI one in a Winc folder of the machine's; an existing lowercase shortcut must
    # not keep its name.
    $menu = if ($kind -eq 'nsis') { [Environment]::GetFolderPath('Programs') } else { Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'Winc' }
    if ($kind -eq 'nsis') {
        Set-Content -LiteralPath (Join-Path $menu 'winc.lnk') -Value ''
        $listing = "$classes\.json\OpenWithProgids"
        if (-not (Test-Path -LiteralPath $listing)) { New-Item -Force -Path $listing | Out-Null }
        New-ItemProperty -Force -LiteralPath $listing -Name 'winc.JSON' -Value '' | Out-Null
        # An earlier version also added the context menu to each type; installing must remove it.
        New-Item -Force -Path "$classes\SystemFileAssociations\.json\shell\winc\command" | Out-Null
    }
    try {
        if ($kind -eq 'nsis') {
            $process = Start-Process -FilePath $setup -ArgumentList "/S /D=$directory" -Wait -PassThru
        } else {
            $process = Start-Process msiexec.exe -ArgumentList "/i `"$setup`" /qn /norestart INSTALLFOLDER=`"$directory`"" -Wait -PassThru
        }
        if ($process.ExitCode -notin @(0, 1641, 3010)) { throw "$kind installation failed: $($process.ExitCode)" }
        foreach ($file in @('winc.exe', 'winc-pty.exe', 'conpty.dll', 'OpenConsole.exe', 'winc-installation', 'winc-watch', 'winc-server-Linux-x86_64', 'winc-server-Linux-aarch64', 'bin/winc', 'bin/winc.cmd')) {
            if (-not (Test-Path "$directory/$file")) { throw "${kind}: missing $file" }
        }
        if ((Get-Content "$directory/bin/winc" -Raw) -match "`r") { throw "${kind}: bin/winc has CRLF line endings" }
        if ((Get-CommandPath $kind) -notcontains "$directory\bin") { throw "${kind}: bin is not on the Path" }
        if ((Get-Content "$directory/winc-installation" -Raw).Trim() -ne $kind) { throw 'incorrect installation marker' }
        # Each type's ProgID shows the document icon of its group.
        foreach ($pair in @(@('JSON', 'json'), @('MD', 'markdown'), @('RS', 'rust'), @('SQL', 'code'), @('TEX', 'markup'), @('PDF', 'text'), @('WINC-PROJECT', 'project'))) {
            $icon = (Get-ItemProperty -LiteralPath "$classes\winc.$($pair[0])\DefaultIcon").'(default)'
            if ($icon -ne "$directory\icons\$($pair[1]).ico") { throw "${kind}: wrong icon for $($pair[0]): $icon" }
            if (-not (Test-Path -LiteralPath $icon)) { throw "${kind}: missing $icon" }
        }
        # .winc-project is Winc's own type and opens with it by default.
        $project = (Get-ItemProperty -LiteralPath "$classes\.winc-project").'(default)'
        if ($project -ne 'winc.WINC-PROJECT') { throw "${kind}: .winc-project does not open with winc: $project" }
        $command = (Get-ItemProperty -LiteralPath "$classes\winc.JSON\shell\open\command").'(default)'
        if ($command -ne "`"$directory\winc.exe`" `"%1`"") { throw "${kind}: wrong open command: $command" }
        if ($null -eq (Get-ItemProperty -LiteralPath "$classes\Applications\winc.exe\SupportedTypes").'.json') { throw "${kind}: json is not a supported type" }
        if ((Get-ItemProperty -LiteralPath "$classes\Applications\winc.exe").FriendlyAppName -cne 'Winc') { throw "${kind}: the application is not named Winc" }
        # A new installation does not add the Explorer context menu, and no version adds it per type.
        if (Test-Path -LiteralPath "$classes\SystemFileAssociations\.json\shell\winc") { throw "${kind}: the context menu is still added per type" }
        if (Test-Path -LiteralPath "$classes\*\shell\winc") { throw "${kind}: the context menu is added by default" }
        # The winc: URL scheme hands the whole link to winc.exe.
        if ($null -eq (Get-ItemProperty -LiteralPath "$classes\winc").'URL Protocol') { throw "${kind}: winc is not a URL scheme" }
        $command = (Get-ItemProperty -LiteralPath "$classes\winc\shell\open\command").'(default)'
        if ($command -ne "`"$directory\winc.exe`" `"%1`"") { throw "${kind}: wrong URL scheme command: $command" }
        $link = Get-ChildItem -LiteralPath $menu -Filter 'winc.lnk'
        if (@($link).Count -ne 1 -or $link.Name -cne 'Winc.lnk') { throw "${kind}: the Start menu shortcut is not named Winc" }
        if ($kind -eq 'msi' -and (Get-Item -LiteralPath $menu).Name -cne 'Winc') { throw 'msi: the Start menu folder is not named Winc' }
        if ($kind -eq 'nsis' -and (Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\winc').DisplayName -cne 'Winc') { throw 'nsis: the installed app is not listed as Winc' }
        # A new installation lists winc under the types it opens. For types Windows runs on
        # double-click (BAT, CMD) a listed handler would replace running the script with a question,
        # so those are never listed or registered.
        $listed = { param($type) $null -ne (Get-ItemProperty -LiteralPath "$classes\.$type\OpenWithProgids" -ErrorAction SilentlyContinue)."winc.$type" }
        foreach ($type in @('json', 'txt')) {
            if (-not (& $listed $type)) { throw "${kind}: winc is not listed under .$type" }
        }
        foreach ($type in @('bat', 'cmd')) {
            if (& $listed $type) { throw "${kind}: winc is listed under .$type" }
            if (Test-Path -LiteralPath "$classes\winc.$type") { throw "${kind}: $type must not be registered" }
        }
        $inMenu = { Test-Path -LiteralPath "$classes\*\shell\winc" }
        if ($kind -eq 'nsis') {
            # Both are the user's choice: once made it survives a later silent installation, which is
            # how updates run, until the other value is passed.
            $steps = @(
                @('/FILETYPES=0', $false, $false), @('', $false, $false), @('/FILETYPES=1', $true, $false), @('', $true, $false),
                @('/CONTEXTMENU=1', $true, $true), @('', $true, $true), @('/CONTEXTMENU=0', $true, $false), @('', $true, $false)
            )
            foreach ($step in $steps) {
                $process = Start-Process -FilePath $setup -ArgumentList "/S $($step[0]) /D=$directory" -Wait -PassThru
                if ($process.ExitCode -ne 0) { throw "nsis installation with '$($step[0])' failed: $($process.ExitCode)" }
                if ((& $listed 'json') -ne $step[1]) { throw "nsis: after '$($step[0])' the json listing is not $($step[1])" }
                if ((& $inMenu) -ne $step[2]) { throw "nsis: after '$($step[0])' the context menu is not $($step[2])" }
            }
            # An installation from before the choices were recorded keeps what it had: the context
            # menu, which was always added, and no listing.
            Remove-ItemProperty -LiteralPath 'HKCU:\Software\DEEIX\winc' -Name 'ContextMenu', 'FileTypes'
            $process = Start-Process -FilePath $setup -ArgumentList "/S /D=$directory" -Wait -PassThru
            if ($process.ExitCode -ne 0) { throw "nsis installation over an earlier version failed: $($process.ExitCode)" }
            if ((& $listed 'json') -or -not (& $inMenu)) { throw 'nsis: an earlier installation did not keep its context menu and listing' }
        } else {
            $process = Start-Process msiexec.exe -ArgumentList "/i `"$setup`" /qn /norestart ADDLOCAL=ContextMenu INSTALLFOLDER=`"$directory`"" -Wait -PassThru
            if ($process.ExitCode -notin @(0, 1641, 3010)) { throw "msi: adding the context menu failed: $($process.ExitCode)" }
        }
        # Explorer context menu, once chosen: one entry for every file opens the file, folders and
        # their empty space open the folder. The label follows the installer's language.
        foreach ($verb in @(@('*', '%1'), @('Directory', '%V'), @('Directory\Background', '%V'))) {
            $key = "$classes\$($verb[0])\shell\winc"
            if (-not (Get-ItemProperty -LiteralPath $key -ErrorAction SilentlyContinue).'(default)') { throw "${kind}: no context menu label under $($verb[0])" }
            $command = (Get-ItemProperty -LiteralPath "$key\command").'(default)'
            if ($command -ne "`"$directory\winc.exe`" `"$($verb[1])`"") { throw "${kind}: wrong context menu command under $($verb[0]): $command" }
        }
        # A short-lived parent exercises the updater's wait before reinstalling.
        $parent = Start-Process powershell.exe -ArgumentList '-NoProfile -Command "Start-Sleep -Seconds 2"' -PassThru -WindowStyle Hidden
        # The updater is started the way the application starts it: `-Command` with the script
        # inlined, and its parameters in `WINC_UPDATE_*` environment variables. Passing them as
        # `-File` arguments would reach a script that no longer declares `param`, leaving every
        # parameter empty. The script re-hashes the package and refuses to run without that
        # digest; the application passes the digest it recorded when the download was verified.
        # The package under test is the one just installed, so winc.exe already matches it. The
        # updater treats a successful install that leaves that file unchanged as a failure,
        # because a silent installer skips a file that is still open. A skipped file must fail
        # here, and a real install must put the packaged bytes back.
        $installed = Join-Path $directory 'winc.exe'
        $bytes = [IO.File]::ReadAllBytes($installed)
        if ($bytes.Length -lt 1) { throw "${kind}: installed winc.exe is empty" }
        if ($kind -eq 'msi') {
            # This MSI is already installed, and its ProductCode does not change until the next
            # build. A second /i therefore does not copy files. A published update is a new
            # ProductCode and installs cleanly. Remove this package first, then leave an older
            # winc.exe so the install has a file to replace.
            $removed = Start-Process msiexec.exe -ArgumentList "/x `"$setup`" /qn /norestart" -Wait -PassThru
            if ($removed.ExitCode -notin @(0, 1605, 1641, 3010)) {
                throw "msi: uninstall before update failed: $($removed.ExitCode)"
            }
            $aged = $false
            for ($i = 0; $i -le $bytes.Length - 24; $i++) {
                $versionInfo = $bytes[$i] -eq 0xBD -and $bytes[$i + 1] -eq 0x04 -and $bytes[$i + 2] -eq 0xEF -and $bytes[$i + 3] -eq 0xFE -and $bytes[$i + 4] -eq 0 -and $bytes[$i + 5] -eq 0 -and $bytes[$i + 6] -eq 1 -and $bytes[$i + 7] -eq 0
                if ($versionInfo) {
                    for ($j = 8; $j -lt 16; $j++) { $bytes[$i + $j] = 0 }
                    $bytes[$i + 12] = 1
                    $aged = $true
                    break
                }
            }
            if (-not $aged) { throw 'msi: winc.exe has no file version to age' }
            New-Item -ItemType Directory -Force -Path $directory | Out-Null
        } else {
            $bytes[0] = [byte]($bytes[0] -bxor 0xFF)
        }
        [IO.File]::WriteAllBytes($installed, $bytes)
        $env:WINC_UPDATE_PARENT = "$($parent.Id)"
        $env:WINC_UPDATE_SETUP = $setup
        $env:WINC_UPDATE_SHA256 = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
        $env:WINC_UPDATE_EXE = "$directory/winc.exe"
        $env:WINC_UPDATE_KIND = $kind
        $env:WINC_UPDATE_RELAUNCH = '0'
        & powershell.exe -NoProfile -NonInteractive -Command (Get-Content -LiteralPath $update -Raw)
        $status = $LASTEXITCODE
        Remove-Item Env:WINC_UPDATE_*
        if ($status -ne 0) {
            $log = Join-Path (Split-Path -Parent $setup) 'install-error.log'
            $detail = if (Test-Path -LiteralPath $log) { (Get-Content -LiteralPath $log -Raw).Trim() } else { "exit $status" }
            throw "$kind updater failed: $detail"
        }
        foreach ($file in @('winc.exe', 'winc-pty.exe')) {
            if ((Get-FileHash "$directory/$file").Hash -ne (Get-FileHash "$root/target/$Target/release/$file").Hash) {
                throw "$kind update changed $file"
            }
        }
    } finally {
        if ($kind -eq 'nsis' -and (Test-Path "$directory/uninstall.exe")) {
            $uninstall = Start-Process "$directory/uninstall.exe" -ArgumentList '/S' -Wait -PassThru
        } elseif ($kind -eq 'msi') {
            $uninstall = Start-Process msiexec.exe -ArgumentList "/x `"$setup`" /qn /norestart" -Wait -PassThru
        }
    }
    foreach ($file in @('conpty.dll', 'OpenConsole.exe')) {
        if (Test-Path "$directory/$file") { throw "${kind}: uninstall left $file behind" }
    }
    if ((Get-CommandPath $kind) -contains "$directory\bin") { throw "${kind}: uninstall left bin on the Path" }
    if (Test-Path -LiteralPath "$classes\winc.JSON") { throw "${kind}: uninstall left the file type behind" }
    if (Test-Path "$directory/icons") { throw "${kind}: uninstall left the icons behind" }
    if (Test-Path -LiteralPath "$classes\Applications\winc.exe") { throw "${kind}: uninstall left the application behind" }
    foreach ($key in @('*', 'SystemFileAssociations\.json', 'Directory', 'Directory\Background')) {
        if (Test-Path -LiteralPath "$classes\$key\shell\winc") { throw "${kind}: uninstall left the context menu under $key" }
    }
    if (Test-Path -LiteralPath "$classes\winc") { throw "${kind}: uninstall left the URL scheme behind" }
    if (Test-Path -LiteralPath (Join-Path $menu 'Winc.lnk')) { throw "${kind}: uninstall left the Start menu shortcut behind" }
}
Expand-Archive "$bundle/portable/Winc-$version-windows-x64-portable.zip" "$work/portable" -Force
if ((Test-Path "$work/portable/winc-installation") -or -not (Test-Path "$work/portable/winc-pty.exe") -or -not (Test-Path "$work/portable/conpty.dll") -or -not (Test-Path "$work/portable/OpenConsole.exe") -or -not (Test-Path "$work/portable/bin/winc") -or -not (Test-Path "$work/portable/winc-watch") -or -not (Test-Path "$work/portable/winc-server-Linux-x86_64") -or -not (Test-Path "$work/portable/winc-server-Linux-aarch64") -or -not (Test-Path "$work/portable/data/README.txt")) { throw 'invalid portable layout' }
