# Runs outside the application so Windows can replace both native executables.
# Passed with -Command; the parameters come from WINC_UPDATE_* environment variables.
$ErrorActionPreference = 'Stop'
$ParentId = [int]$env:WINC_UPDATE_PARENT
$Setup = $env:WINC_UPDATE_SETUP
$Executable = $env:WINC_UPDATE_EXE
$Installer = $env:WINC_UPDATE_KIND
$Relaunch = $env:WINC_UPDATE_RELAUNCH
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
    $elevate = -not (Test-CanReplace $Executable)
    if ($Installer -eq 'msi') {
        $code = Invoke-Setup "$env:SystemRoot\System32\msiexec.exe" "/i `"$Setup`" /qn /norestart INSTALLFOLDER=`"$directory`"" $elevate
    } else {
        # NSIS requires /D to be last and unquoted; it consumes the remaining command line.
        $code = Invoke-Setup $Setup "/S /D=$directory" $elevate
    }
    if ($code -notin @(0, 1641, 3010)) { throw "installer failed with $code" }
    if ($Relaunch -eq '1') { Start-Process -FilePath $Executable -WorkingDirectory $directory }
    exit 0
} catch {
    # Writing the log must not replace the error being reported: `$Setup` is empty when the
    # caller passed no environment variables, and `Split-Path -Parent $null` would throw here.
    if ($Log) {
        try { $_ | Out-String | Set-Content -Path $Log -ErrorAction Stop }
        catch { Write-Error "could not write $Log" -ErrorAction Continue }
    }
    if ($Relaunch -eq '1' -and $Executable -and (Test-Path -LiteralPath $Executable)) {
        Start-Process -FilePath $Executable -WorkingDirectory (Split-Path -Parent $Executable)
    }
    exit 1
}
