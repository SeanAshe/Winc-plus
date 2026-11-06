# Adds a directory to the current user's Path or removes it. Reads and writes the registry value
# directly: the Environment class expands %VAR% entries and would rewrite them. Nothing changes when
# the entry is already as requested, and a Path that this leaves empty is deleted.
param(
    [Parameter(Mandatory)][ValidateSet('Add', 'Remove')][string]$Action,
    [Parameter(Mandatory)][string]$Directory
)
$ErrorActionPreference = 'Stop'
$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
try {
    $options = [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    $current = [string]$key.GetValue('Path', '', $options)
    $wanted = $Directory.TrimEnd('\')
    $all = @($current -split ';' | Where-Object { $_ })
    $others = @($all | Where-Object { $_.TrimEnd('\') -ne $wanted })
    $present = $others.Count -ne $all.Count
    if (($Action -eq 'Add') -eq $present) { return }
    if ($Action -eq 'Add') { $others += $wanted }
    if ($others.Count -eq 0) {
        $key.DeleteValue('Path', $false)
    } else {
        $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
        if ($key.GetValueNames() -contains 'Path') { $kind = $key.GetValueKind('Path') }
        $key.SetValue('Path', ($others -join ';'), $kind)
    }
} finally {
    $key.Close()
}
# Deleting an absent variable changes nothing and tells running programs to reread the environment.
[Environment]::SetEnvironmentVariable('WINC_PATH_CHANGED', $null, 'User')
