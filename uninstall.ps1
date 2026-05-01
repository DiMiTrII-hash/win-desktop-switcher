# Win Desktop Switcher - uninstaller
# Usage: irm https://github.com/DiMiTrII-hash/win-desktop-switcher/releases/latest/download/uninstall.ps1 | iex

$ErrorActionPreference = 'SilentlyContinue'

$target = Join-Path $env:LOCALAPPDATA 'WinDesktopSwitcher'
$exe    = Join-Path $target 'win_desktop_swither.exe'

Write-Host "[1/3] stopping running instance..."
Get-Process win_desktop_swither -ErrorAction SilentlyContinue | Stop-Process -Force

Write-Host "[2/3] removing autostart..."
if (Test-Path $exe) { & $exe --disable-autostart | Out-Null }

Write-Host "[3/3] deleting files..."
Start-Sleep -Milliseconds 500
if (Test-Path $target) { Remove-Item -Recurse -Force $target }

Write-Host "Uninstalled."
