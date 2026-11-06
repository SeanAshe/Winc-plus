param([string]$Target = 'x86_64-pc-windows-msvc')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$version = (& node "$PSScriptRoot/../common/version.mjs").Trim()
if ($LASTEXITCODE -ne 0) { throw 'cannot read the Cargo.toml version' }
$bundle = "$root/target/$Target/release/bundle"
$work = Join-Path $env:RUNNER_TEMP 'winc-installer-verification'
New-Item -ItemType Directory -Force $work | Out-Null
$update = "$root/crates/winc-app/src/updates/installer/update.ps1"
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
    try {
        if ($kind -eq 'nsis') {
            $process = Start-Process -FilePath $setup -ArgumentList "/S /D=$directory" -Wait -PassThru
        } else {
            $process = Start-Process msiexec.exe -ArgumentList "/i `"$setup`" /qn /norestart INSTALLFOLDER=`"$directory`"" -Wait -PassThru
        }
        if ($process.ExitCode -notin @(0, 1641, 3010)) { throw "$kind installation failed: $($process.ExitCode)" }
        foreach ($file in @('winc.exe', 'winc-pty.exe', 'winc-installation')) {
            if (-not (Test-Path "$directory/$file")) { throw "${kind}: missing $file" }
        }
        if ((Get-Content "$directory/winc-installation" -Raw).Trim() -ne $kind) { throw 'incorrect installation marker' }
        $command = (Get-ItemProperty -LiteralPath "$classes\winc.JSON\shell\open\command").'(default)'
        if ($command -ne "`"$directory\winc.exe`" `"%1`"") { throw "${kind}: wrong open command: $command" }
        if ($null -eq (Get-ItemProperty -LiteralPath "$classes\.json\OpenWithProgids").'winc.JSON') { throw "${kind}: json is not registered" }
        # A short-lived parent exercises the updater's wait before reinstalling.
        $parent = Start-Process powershell.exe -ArgumentList '-NoProfile -Command "Start-Sleep -Seconds 2"' -PassThru -WindowStyle Hidden
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $update -ParentId $parent.Id -Setup $setup -Executable "$directory/winc.exe" -Installer $kind -Relaunch 0
        if ($LASTEXITCODE -ne 0) { throw "$kind updater failed" }
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
    if (Test-Path -LiteralPath "$classes\winc.JSON") { throw "${kind}: uninstall left the file type behind" }
}
Expand-Archive "$bundle/portable/Winc-$version-windows-x64-portable.zip" "$work/portable" -Force
if ((Test-Path "$work/portable/winc-installation") -or -not (Test-Path "$work/portable/winc-pty.exe")) { throw 'invalid portable layout' }
