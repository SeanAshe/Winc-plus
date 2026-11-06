# Runs outside the application so Windows can replace both native executables.
param(
    [Parameter(Mandatory)][int]$ParentId,
    [Parameter(Mandatory)][string]$Setup,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][ValidateSet('nsis', 'msi')][string]$Installer,
    [ValidateSet('0', '1')][string]$Relaunch = '0'
)
$ErrorActionPreference = 'Stop'

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

$directory = Split-Path -Parent $Executable
try {
    $parent = Get-Process -Id $ParentId -ErrorAction SilentlyContinue
    if ($parent) { $parent.WaitForExit() }
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
    $_ | Out-String | Set-Content -Path (Join-Path (Split-Path -Parent $Setup) 'install-error.log')
    if ($Relaunch -eq '1' -and (Test-Path -LiteralPath $Executable)) {
        Start-Process -FilePath $Executable -WorkingDirectory $directory
    }
    exit 1
}
