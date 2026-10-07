param(
    [Parameter(Mandatory = $true)][string]$InstallerPath,
    [Parameter(Mandatory = $true)][string]$BackupRoot
)
$ErrorActionPreference = 'Stop'
function Get-OptionalRegistryValue([string]$KeyPath, [string]$ValueName) {
    if (-not (Test-Path -LiteralPath $KeyPath)) { return $null }
    return (Get-Item -LiteralPath $KeyPath).GetValue($ValueName)
}
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$releaseVersion = (Get-Content -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
if ($releaseVersion -notmatch '^\d+\.\d+\.\d+$') { throw 'Expected a stable release version in tauri.conf.json.' }
if ([IO.Path]::GetFileName($installer) -ne "Sonora_${releaseVersion}_x64-setup.exe") { throw "Expected Sonora $releaseVersion x64 NSIS installer." }
if (-not (Test-Path -LiteralPath $BackupRoot -PathType Container)) { throw 'Backup root must already exist.' }
$legacyDirectory = Join-Path $env:LOCALAPPDATA 'Haumea Voice'
$legacyExe = Join-Path $legacyDirectory 'haumea-voice.exe'
$sonoraDirectory = Join-Path $env:LOCALAPPDATA 'Sonora'
$sonoraExe = Join-Path $sonoraDirectory 'sonora.exe'
$dataDirectory = Join-Path $env:APPDATA 'com.haumeavoice.app'
$localDataDirectory = Join-Path $env:LOCALAPPDATA 'com.haumeavoice.app'
$backup = Join-Path $BackupRoot ("Sonora-$releaseVersion-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
if (Test-Path -LiteralPath $backup) { throw 'Backup destination already exists.' }
$legacyRegistry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Haumea Voice'
$legacy = Get-ItemProperty -LiteralPath $legacyRegistry -ErrorAction SilentlyContinue
if ($legacy -and ($legacy.InstallLocation.Trim('"') -ne $legacyDirectory -or $legacy.MainBinaryName -ne 'haumea-voice.exe')) {
    throw 'Legacy installation does not match the supported per-user upgrade path.'
}

# Stop only the two known product executables, never a process selected by name alone.
Get-Process -Name 'haumea-voice','sonora' -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -in @($legacyExe, $sonoraExe) -or $_.Path -like "*\Sonora\sonora.exe" -or $_.Path -like "*\Haumea Voice\haumea-voice.exe" } |
    ForEach-Object { Stop-Process -Id $_.Id; $_.WaitForExit(10000) | Out-Null }
New-Item -ItemType Directory -Path $backup | Out-Null
foreach ($pair in @(@($dataDirectory,'RoamingData'), @($localDataDirectory,'LocalData'), @($legacyDirectory,'LegacyApplication'), @($sonoraDirectory,'SonoraApplication'))) {
    if (Test-Path -LiteralPath $pair[0]) { Copy-Item -LiteralPath $pair[0] -Destination (Join-Path $backup $pair[1]) -Recurse }
}
Copy-Item -LiteralPath $installer -Destination $backup
$before = @{}
if (Test-Path -LiteralPath $dataDirectory) {
    Get-ChildItem -LiteralPath $dataDirectory -Recurse -File -Force | ForEach-Object {
        $relative = $_.FullName.Substring($dataDirectory.Length).TrimStart('\', '/')
        $before[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
    }
}
$before | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $backup 'data-before.json') -Encoding utf8

$installed = Start-Process -FilePath $installer -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($installed.ExitCode -ne 0) { throw "Sonora installer failed: $($installed.ExitCode). Backup: $backup" }
if (-not (Test-Path -LiteralPath $sonoraExe)) {
    $redirected = $null
    if ($env:LOCALAPPDATA) {
        $packagesDir = Join-Path $env:LOCALAPPDATA 'Packages'
        if (Test-Path -LiteralPath $packagesDir) {
            $redirected = Get-ChildItem -Path $packagesDir -Filter 'sonora.exe' -Recurse -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -like "*\LocalCache\Local\Sonora\sonora.exe" } |
                Select-Object -ExpandProperty FullName -First 1
        }
    }
    if ($redirected -and (Test-Path -LiteralPath $redirected)) {
        $sonoraExe = $redirected
        $sonoraDirectory = Split-Path -Parent $redirected
    }
}
if (-not (Test-Path -LiteralPath $sonoraExe)) { throw "Sonora executable missing. Backup: $backup" }
$version = (Get-Item -LiteralPath $sonoraExe).VersionInfo.ProductVersion
if ($version -notin @($releaseVersion, "$releaseVersion.0")) { throw "Unexpected installed version: $version" }

# Retire the old installed program only after the new executable is verified.
# /UPDATE explicitly preserves AppData in Tauri's legacy NSIS uninstaller.
if ($legacy) {
    $uninstaller = Join-Path $legacyDirectory 'uninstall.exe'
    if (-not (Test-Path -LiteralPath $uninstaller)) { throw "Legacy uninstaller missing. Backup: $backup" }
    $removed = Start-Process -FilePath $uninstaller -ArgumentList "/S /UPDATE _?=$legacyDirectory" -WindowStyle Hidden -Wait -PassThru
    if ($removed.ExitCode -ne 0) { throw "Legacy uninstaller failed: $($removed.ExitCode). Backup: $backup" }
}

# Carry an existing autostart choice to the renamed executable, preserving other entries.
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
foreach ($name in @('HaumeaVoice', 'Haumea Voice')) {
    $value = Get-OptionalRegistryValue $runKey $name
    if ($value -and $value -eq ('"' + $legacyExe + '" --autostart')) {
        @{ name = $name; value = $value } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $backup ('autostart-' + $name + '.json'))
        Set-ItemProperty -LiteralPath $runKey -Name 'Sonora' -Value ('"' + $sonoraExe + '" --autostart')
        Remove-ItemProperty -LiteralPath $runKey -Name $name
    }
}

# Existing browser registrations retain their host ID and allowed origins.
$hostCount = 0
$manifests = @{}
foreach ($browser in @('Google\Chrome', 'Microsoft\Edge', 'BraveSoftware\Brave-Browser')) {
    $key = 'HKCU:\Software\' + $browser + '\NativeMessagingHosts\com.haumeavoice.context'
    $manifestPath = Get-OptionalRegistryValue $key ''
    if (-not $manifestPath -or $manifests.ContainsKey($manifestPath) -or -not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { continue }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.name -ne 'com.haumeavoice.context' -or $manifest.path -ne $legacyExe) { continue }
    Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $backup ("native-host-$hostCount.json"))
    $manifest.path = $sonoraExe
    $manifest.description = 'Sonora limited browser context bridge'
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 6))
    $manifests[$manifestPath] = $true
    $hostCount++
}

$shell = New-Object -ComObject WScript.Shell
foreach ($shortcut in @((Join-Path ([Environment]::GetFolderPath('Desktop')) 'Haumea Voice.lnk'), (Join-Path ([Environment]::GetFolderPath('Programs')) 'Haumea Voice.lnk'))) {
    if ((Test-Path -LiteralPath $shortcut) -and $shell.CreateShortcut($shortcut).TargetPath -eq $legacyExe) {
        Copy-Item -LiteralPath $shortcut -Destination $backup
        Remove-Item -LiteralPath $shortcut
    }
}

$changed = @($before.Keys | Where-Object {
    $path = Join-Path $dataDirectory $_
    -not (Test-Path -LiteralPath $path) -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $before[$_]
})
if ($changed.Count) { throw "$($changed.Count) original data files changed during installation. Backup: $backup" }
$registration = Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Sonora' -ErrorAction SilentlyContinue
if (-not $registration) {
    $registration = Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\com.haumeavoice.app' -ErrorAction SilentlyContinue
}
if (-not $registration) {
    $registration = Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue |
        ForEach-Object { Get-ItemProperty $_.PsPath } |
        Where-Object { $_.DisplayName -eq 'Sonora' } |
        Select-Object -First 1
}
if (-not $registration -or $registration.DisplayVersion -ne $releaseVersion) { throw 'Installed registry version does not match.' }
if (Test-Path -LiteralPath $legacyRegistry) { throw 'Legacy uninstall entry remains; inspect the backup and registration.' }
$report = @{ version = $version; installerExit = $installed.ExitCode; preservedDataFiles = $before.Count; nativeHostsUpdated = $hostCount; backup = $backup; executable = $sonoraExe }
$report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $backup 'installation-report.json') -Encoding utf8
try {
    Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $sonoraExe } | Out-Null
} catch {
    Start-Process -FilePath $sonoraExe
}
$report | ConvertTo-Json
