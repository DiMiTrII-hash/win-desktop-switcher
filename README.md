# win-desktop-switcher

Плавное переключение виртуальных рабочих столов в Windows 11 с анимацией «плёнки» — два стола едут вместе с чёткой границей, как кадры на киноплёнке.

## Установка

Открой PowerShell и вставь одну команду:

```powershell
irm https://github.com/DiMiTrII-hash/win-desktop-switcher/releases/latest/download/install.ps1 | iex
```

Что произойдёт:
- скачает бинарь и конфиг в `%LOCALAPPDATA%\WinDesktopSwitcher`
- включит автозапуск при входе в систему
- запустит в фоне — появится иконка в трее

Прав администратора не требуется.

## Удаление

```powershell
irm https://github.com/DiMiTrII-hash/win-desktop-switcher/releases/latest/download/uninstall.ps1 | iex
```

## Горячие клавиши

| Комбинация            | Действие                              |
|-----------------------|---------------------------------------|
| `Win` + `←` / `→`     | переключить стол с анимацией плёнки   |
| `Win` + колесо мыши   | переключение колесом                  |
| `Win` + `1..9`        | прыжок на конкретный стол             |
| колесо в нижних 60px  | переключение без зажатого `Win`       |

## Настройка

Файл `%LOCALAPPDATA%\WinDesktopSwitcher\config.toml`:

```toml
[animation]
mode = "slide-push"      # "slide-push" | "touch" | "instant"
duration_ms = 320        # длительность плёнки, мс
easing = "cubic-bezier(0.4, 0.0, 0.2, 1)"

[overlay]
enabled = true           # показывать индикатор "2 / 5" при переключении

[wheel]
hot_zone_height_px = 60  # высота нижней зоны для колеса (0 — выключить)

[navigation]
wrap = true              # циклично (с последнего на первый)
```

После правки — правый клик по иконке в трее → *Reload config*.

## Сборка из исходников

```
cargo build --release
```

На выходе один самодостаточный exe в `target\release\win_desktop_swither.exe`.

## Требования

Windows 11 (build 22000+). Используется крейт [winvd](https://crates.io/crates/winvd),
который опирается на внутренние COM-интерфейсы `IVirtualDesktopManager`.

## Как это работает (вкратце)

Анимация «плёнки» без видимых артефактов делается так: снимаем скриншот текущего
стола, накрываем экран layered-окном с этим скриншотом (с флагом `WDA_EXCLUDEFROMCAPTURE`,
чтобы не попадать в последующие BitBlt), под ним переключаем стол через winvd,
ждём отрисовки, снимаем скриншот нового стола (наше окно невидимо для захвата),
склеиваем оба в широкий bitmap и анимируем его сдвиг.

---

## English

Smooth virtual desktop switching for Windows 11 with a film-strip animation.

**Install** (PowerShell):
```powershell
irm https://github.com/DiMiTrII-hash/win-desktop-switcher/releases/latest/download/install.ps1 | iex
```

Hotkeys: `Win`+`←/→`, `Win`+wheel, `Win`+`1..9`. Tray icon for control.
Config: `%LOCALAPPDATA%\WinDesktopSwitcher\config.toml`.

Build: `cargo build --release`.
