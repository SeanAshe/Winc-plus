# Places the pinned conpty.dll and OpenConsole.exe in a directory, normally beside winc-pty.exe.
# They come from Microsoft's NuGet package and are not committed to this repository.
param([Parameter(Mandatory)][string]$Destination)
$ErrorActionPreference = 'Stop'
$version = '1.25.260930003'
$sha256 = '02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee'
$files = @{
    'runtimes/win-x64/native/conpty.dll'        = 'conpty.dll'
    'build/native/runtimes/x64/OpenConsole.exe' = 'OpenConsole.exe'
}
$id = 'microsoft.windows.console.conpty'
$url = "https://api.nuget.org/v3-flatcontainer/$id/$version/$id.$version.nupkg"
$package = Join-Path ([IO.Path]::GetTempPath()) "$id.$version.$PID.nupkg"
try {
    Invoke-WebRequest -Uri $url -OutFile $package -MaximumRetryCount 3 -RetryIntervalSec 5
    if ((Get-FileHash $package -Algorithm SHA256).Hash -ne $sha256) { throw "unexpected SHA-256 for $url" }
    New-Item -ItemType Directory -Force $Destination | Out-Null
    $directory = (Resolve-Path $Destination).Path
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($package)
    try {
        foreach ($entry in $files.GetEnumerator()) {
            $file = $archive.GetEntry($entry.Key)
            if (-not $file) { throw "$url has no $($entry.Key)" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($file, (Join-Path $directory $entry.Value), $true)
        }
    } finally {
        $archive.Dispose()
    }
} finally {
    Remove-Item $package -ErrorAction SilentlyContinue
}
