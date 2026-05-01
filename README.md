# win-desktop-switcher

Smooth virtual desktop switching for Windows 11 with a "film strip" slide animation.

## Install

Open PowerShell and run:

```powershell
irm https://github.com/DiMiTrII-hash/win-desktop-switcher/releases/latest/download/install.ps1 | iex
```

That downloads the binary into `%LOCALAPPDATA%\WinDesktopSwitcher`, enables
autostart and launches it in the background. A tray icon appears.

## Uninstall

```powershell
irm https://github.com/DiMiTrII-hash/win-desktop-switcher/releases/latest/download/uninstall.ps1 | iex
```

## Hotkeys

| Keys             | Action                              |
|------------------|-------------------------------------|
| `Win` + `←`/`→`  | switch desktop with slide animation |
| `Win` + wheel    | switch with the mouse wheel         |
| `Win` + `1..9`   | jump to a specific desktop          |
| wheel in bottom 60 px (no `Win`) | also switches              |

## Configuration

`%LOCALAPPDATA%\WinDesktopSwitcher\config.toml`:

```toml
[animation]
mode = "slide-push"      # "slide-push" | "touch" | "instant"
duration_ms = 320
easing = "cubic-bezier(0.4, 0.0, 0.2, 1)"

[overlay]
enabled = true           # show "2 / 5" indicator on switch

[wheel]
hot_zone_height_px = 60  # bottom strip where wheel works without Win
```

After editing — right click the tray icon → *Reload config*.

## Build from source

```
cargo build --release
```

Resulting `target\release\win_desktop_swither.exe` is a single self-contained binary.

## Requirements

Windows 11 (build 22000+). Uses the [winvd](https://crates.io/crates/winvd) crate
which depends on internal `IVirtualDesktopManager` COM interfaces.
