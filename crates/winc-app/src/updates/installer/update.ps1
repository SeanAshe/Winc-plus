# Runs outside the application so Windows can replace both native executables.
# Passed with -Command; the parameters come from WINC_UPDATE_* environment variables.
$ErrorActionPreference = 'Stop'
$ParentId = [int]$env:WINC_UPDATE_PARENT
$Setup = $env:WINC_UPDATE_SETUP
$Executable = $env:WINC_UPDATE_EXE
$Installer = $env:WINC_UPDATE_KIND
$Relaunch = $env:WINC_UPDATE_RELAUNCH
$Expected = $env:WINC_UPDATE_SHA256
$held = $null
function Get-Sha256([IO.Stream]$stream) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { -join ($sha.ComputeHash($stream) | ForEach-Object { $_.ToString('x2') }) }
    finally { $sha.Dispose() }
}
# Kept for the catch block: the environment variables are removed before the installer runs.
$Log = if ($Setup) { Join-Path (Split-Path -Parent $Setup) 'install-error.log' }

# Program Files is not writable by a normal user. An access check, not the admin role, decides
# elevation: a non-elevated administrator still cannot replace winc.exe there.
function Test-CanReplace([string]$Path) {
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try {
            $stream = [System.IO.File]::Open($Path, 'Open', 'ReadWrite', 'ReadWrite')
            $stream.Dispose()
            return $true
        } catch [System.UnauthorizedAccessException] {
            return $false
        } catch {
            Start-Sleep -Milliseconds 250
        }
    }
    return $true
}

# -Verb RunAs with -Wait does not report the elevated process's exit code. Wait on the handle.
function Invoke-Setup([string]$FilePath, [string]$ArgumentList, [bool]$Elevate) {
    $start = @{ FilePath = $FilePath; ArgumentList = $ArgumentList; PassThru = $true }
    if ($Elevate) { $start.Verb = 'RunAs' }
    $process = Start-Process @start
    if (-not $process) { throw 'installer did not start' }
    $process.WaitForExit()
    $code = $null
    try { $code = $process.ExitCode } catch { }
    if ($null -eq $code) { throw 'installer did not report an exit code' }
    return $code
}

try {
    # The installer and the relaunched application inherit this environment.
    Remove-Item -Path Env:WINC_UPDATE_*
    $parent = Get-Process -Id $ParentId -ErrorAction SilentlyContinue
    if ($parent) { $parent.WaitForExit() }
    $directory = Split-Path -Parent $Executable
    # The package was verified when it was downloaded and has sat in the temporary directory
    # since. Open it without write or delete sharing, check it is still those bytes, and keep
    # it open until the installer has finished: nothing can replace it after this check.
    $held = [IO.File]::Open($Setup, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    if (-not $Expected -or (Get-Sha256 $held) -ne $Expected) {
        throw 'the update package changed after it was verified'
    }
    $before = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
    $elevate = -not (Test-CanReplace $Executable)
    if ($Installer -eq 'msi') {
        $code = Invoke-Setup "$env:SystemRoot\System32\msiexec.exe" "/i `"$Setup`" /qn /norestart INSTALLFOLDER=`"$directory`"" $elevate
    } else {
        # NSIS requires /D to be last and unquoted; it consumes the remaining command line.
        $code = Invoke-Setup $Setup "/S /D=$directory" $elevate
    }
    if ($code -notin @(0, 1641, 3010)) { throw "installer failed with $code" }
    # The silent NSIS installer skips a file that is still in use and reports success all the
    # same. Windows Installer replaces such a file at the next restart and reports 1641 or 3010.
    # A same-build MSI install is maintenance and may leave files unchanged.
    $after = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
    if ($Installer -ne 'msi' -and $code -eq 0 -and $after -eq $before) {
        throw 'the installer left the application unchanged'
    }
    $held.Dispose()
    if ($Relaunch -eq '1') { Start-Process -FilePath $Executable -WorkingDirectory $directory }
    exit 0
} catch {
    if ($held) { $held.Dispose() }
    # Writing the log must not replace the error being reported: `$Setup` is empty when the
    # caller passed no environment variables, and `Split-Path -Parent $null` would throw here.
    if ($Log) {
        try { $_ | Out-String | Set-Content -Path $Log -ErrorAction Stop }
        catch { Write-Error "could not write $Log" -ErrorAction Continue }
    }
    # The user asked for the application to come back: start the copy that is still installed.
    if ($Relaunch -eq '1' -and $Executable -and (Test-Path -LiteralPath $Executable)) {
        try { Start-Process -FilePath $Executable -WorkingDirectory (Split-Path -Parent $Executable) }
        catch { Write-Error 'could not restart the application' -ErrorAction Continue }
    }
    exit 1
}
