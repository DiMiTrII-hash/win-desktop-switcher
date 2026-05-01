# Win Desktop Switcher - one-line installer
# Usage: irm https://github.com/DiMiTrII-hash/win-desktop-switcher/releases/latest/download/install.ps1 | iex

$ErrorActionPreference = 'Stop'

$repo   = 'DiMiTrII-hash/win-desktop-switcher'
$target = Join-Path $env:LOCALAPPDATA 'WinDesktopSwitcher'
$base   = "https://github.com/$repo/releases/latest/download"

Write-Host "[1/4] target: $target"
New-Item -ItemType Directory -Force -Path $target | Out-Null

Write-Host "[2/4] stopping running instance (if any)..."
Get-Process win_desktop_swither -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

Write-Host "[3/4] downloading binary and default config..."
Invoke-WebRequest "$base/win_desktop_swither.exe" -OutFile (Join-Path $target 'win_desktop_swither.exe')
$cfgPath = Join-Path $target 'config.toml'
if (-not (Test-Path $cfgPath)) {
    Invoke-WebRequest "$base/config.toml" -OutFile $cfgPath
} else {
    Write-Host "      config.toml already exists, keeping user version"
}

Write-Host "[4/4] enabling autostart and launching..."
& (Join-Path $target 'win_desktop_swither.exe') --enable-autostart | Out-Null
Start-Process -FilePath (Join-Path $target 'win_desktop_swither.exe')

Write-Host ""
Write-Host "Installed. Running in background."
Write-Host "Hotkeys: Win+Left/Right, Win+Wheel, Win+1..9"
Write-Host "Exit via the tray icon menu."
