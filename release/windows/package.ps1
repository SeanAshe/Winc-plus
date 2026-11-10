param(
    [Parameter(Mandatory)][ValidateSet('installers', 'portable')][string]$Phase,
    [string]$Target = 'x86_64-pc-windows-msvc'
)
$ErrorActionPreference = 'Stop'
if ($Target -ne 'x86_64-pc-windows-msvc') { throw "unsupported Windows target: $Target" }
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$version = (& node "$PSScriptRoot/../common/version.mjs").Trim()
if ($LASTEXITCODE -ne 0) { throw 'cannot read the Cargo.toml version' }
$config = Get-Content "$env:RUNNER_TEMP/build-config.json" -Raw | ConvertFrom-Json
if ($config.version -ne $version) { throw 'build configuration does not match the Cargo.toml version' }
$release = "$root/target/$Target/release"
$bundle = "$release/bundle"
$stage = "$bundle/stage"
$stem = "Winc-$version-windows-x64"
New-Item -ItemType Directory -Force $stage | Out-Null
foreach ($name in @('winc.exe', 'winc-pty.exe', 'conpty.dll', 'OpenConsole.exe')) {
    if (-not (Test-Path "$release/$name")) { throw "missing $name" }
    Copy-Item "$release/$name" "$stage/$name" -Force
}
Copy-Item "$root/LICENSE", "$root/NOTICE", "$bundle/icons/winc.ico" $stage -Force
# WSL runs bin/winc with sh, which rejects CRLF line endings however the source was checked out.
New-Item -ItemType Directory -Force "$stage/bin" | Out-Null
$launcher = [IO.File]::ReadAllText("$PSScriptRoot/winc").Replace("`r`n", "`n")
[IO.File]::WriteAllText("$stage/bin/winc", $launcher, (New-Object Text.UTF8Encoding $false))
Copy-Item "$PSScriptRoot/winc.cmd" "$stage/bin/winc.cmd" -Force
Copy-Item "$PSScriptRoot/path.ps1" $stage -Force
Copy-Item "$root/target/x86_64-unknown-linux-musl/release/winc-watch" $stage -Force
if ($Phase -eq 'portable') {
    $portable = "$bundle/portable"
    New-Item -ItemType Directory -Force $portable | Out-Null
    # A data folder beside winc.exe keeps settings and state out of the user profile; the readme
    # inside also keeps Compress-Archive from dropping the folder as empty.
    New-Item -ItemType Directory -Force "$stage/data" | Out-Null
    Copy-Item "$PSScriptRoot/portable-data-readme.txt" "$stage/data/README.txt" -Force
    # Enumerate the files: installer markers never enter a portable archive.
    Compress-Archive -Path "$stage/winc.exe", "$stage/winc-pty.exe", "$stage/conpty.dll", "$stage/OpenConsole.exe", "$stage/winc-watch", "$stage/bin", "$stage/data", "$stage/LICENSE", "$stage/NOTICE", "$stage/winc.ico" -DestinationPath "$portable/$stem-portable.zip" -Force
    return
}

New-Item -ItemType Directory -Force "$stage/nsis", "$stage/msi", "$bundle/nsis", "$bundle/msi" | Out-Null
[IO.File]::WriteAllText("$stage/nsis/winc-installation", "nsis`n")
[IO.File]::WriteAllText("$stage/msi/winc-installation", "msi`n")
$makensis = "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
$wix = "${env:ProgramFiles(x86)}\WiX Toolset v3.14\bin"
if (-not (Test-Path $makensis) -or -not (Test-Path "$wix/candle.exe")) { throw 'NSIS 3.11 and WiX 3.14.1 are required' }
$numericVersion = (($config.msiVersion.Split('.') + @('0', '0', '0', '0'))[0..3] -join '.')
$nsis = "$bundle/nsis/$stem-setup.exe"
$nsisArgs = @('/V2', '/INPUTCHARSET', 'UTF8', "/DVERSION=$version", "/DVIVERSION=$numericVersion", "/DSOURCE=$stage", "/DOUT_FILE=$nsis")
if ($env:WINDOWS_CERTIFICATE_THUMBPRINT) { $nsisArgs += "/DSIGN_SCRIPT=$PSScriptRoot/sign.ps1" }
& $makensis @nsisArgs "$root/release/windows/installer.nsi"
if ($LASTEXITCODE -ne 0) { throw "makensis failed with $LASTEXITCODE" }

# WiX's license dialog consumes RTF; escape the plain Apache license without a new dependency.
$license = (Get-Content "$root/LICENSE" -Raw).Replace('\', '\\').Replace('{', '\{').Replace('}', '\}').Replace("`r", '').Replace("`n", '\par ')
[IO.File]::WriteAllText("$stage/license.rtf", "{\rtf1\ansi $license}")
$msi = "$bundle/msi/$stem.msi"
& "$wix/candle.exe" -nologo -arch x64 "-dVersion=$($config.msiVersion)" "-dSource=$stage" -out "$stage/installer.wixobj" "$root/release/windows/installer.wxs"
if ($LASTEXITCODE -ne 0) { throw "candle failed with $LASTEXITCODE" }
& "$wix/light.exe" -nologo -ext WixUIExtension -out $msi "$stage/installer.wixobj"
if ($LASTEXITCODE -ne 0) { throw "light failed with $LASTEXITCODE" }
& "$PSScriptRoot/sign.ps1" -Target $Target -FilePath @($nsis, $msi)
