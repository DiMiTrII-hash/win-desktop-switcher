use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub display: Display,
    pub animation: Animation,
    pub overlay: Overlay,
    pub wheel: Wheel,
    pub touch: Touch,
    pub navigation: Navigation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    /// Показывать ли большой индикатор с точками и цифрой "2 / 5" при переключении
    pub enabled: bool,
}

impl Default for Overlay {
    fn default() -> Self { Self { enabled: true } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub mode: DisplayMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    Instant,
    Indicator,
    #[default]
    Overlay,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    /// Режим анимации переключения стола:
    /// "touch"      — нативная Windows slide через 4-finger swipe injection
    /// "slide-push" — снимок старого стола уезжает, новый виден в щели
    /// "instant"    — мгновенно, без анимации
    pub mode: String,
    pub duration_ms: u32,
    /// CSS-строка: "linear" | "ease-in" | "ease-out" | "ease-in-out" | "cubic-bezier(x1,y1,x2,y2)"
    pub easing: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Touch {
    /// Включить touch-injection для нативной slide-анимации. false = fallback winvd.
    pub enabled: bool,
    /// Общее перемещение пальцев за свайп (px). Больше = Windows покажет более длинный переход.
    pub swipe_distance_px: i32,
    /// Количество промежуточных кадров. Больше = плавнее, дольше.
    pub swipe_steps: u32,
    /// Пауза между кадрами (мс). Общее время = swipe_steps * step_delay_ms.
    pub step_delay_ms: u64,
    /// Кривая скорости: "linear" | "ease-in" | "ease-out" | "ease-in-out".
    /// "ease-in" = медленный старт + flick в конце (обычно лучше для commit).
    pub easing: String,
    /// Пауза после DOWN перед началом MOVE (мс). Даёт системе распознать touch.
    pub hold_before_ms: u64,
    /// Пауза после финального MOVE перед UP (мс). Даёт анимации "устаканиться".
    pub hold_after_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Wheel {
    pub threshold: i32,
    pub cooldown_ms: u32,
    pub velocity_damping: f32,
    /// Высота "горячей зоны" внизу экрана (px), при наведении в которую
    /// колесо переключает столы без зажатого Win. 0 = выключить.
    pub hot_zone_height_px: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Navigation {
    pub wrap: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            display: Display::default(),
            animation: Animation::default(),
            overlay: Overlay::default(),
            wheel: Wheel::default(),
            touch: Touch::default(),
            navigation: Navigation::default(),
        }
    }
}

impl Default for Touch {
    fn default() -> Self {
        Self {
            enabled: true,
            swipe_distance_px: 650,
            swipe_steps: 28,
            step_delay_ms: 11,
            easing: "ease-in".to_string(),
            hold_before_ms: 15,
            hold_after_ms: 35,
        }
    }
}

impl Default for Display {
    fn default() -> Self {
        Self { mode: DisplayMode::Overlay }
    }
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            mode: "slide-push".to_string(),
            duration_ms: 320,
            easing: "cubic-bezier(0.4, 0.0, 0.2, 1)".to_string(),
        }
    }
}

impl Default for Wheel {
    fn default() -> Self {
        Self {
            threshold: 120,
            cooldown_ms: 150,
            velocity_damping: 0.4,
            hot_zone_height_px: 60,
        }
    }
}

impl Default for Navigation {
    fn default() -> Self {
        Self { wrap: true }
    }
}

impl Config {
    /// Загружает конфиг: ищет `config.toml` сначала в cwd, потом рядом с .exe.
    /// Если не нашёл — возвращает defaults и логирует это.
    pub fn load() -> Result<(Self, Option<PathBuf>)> {
        if let Some(path) = find_config_path() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            let cfg: Config = toml::from_str(&raw)
                .with_context(|| format!("parse {}", path.display()))?;
            return Ok((cfg, Some(path)));
        }
        Ok((Self::default(), None))
    }

    /// Сохраняет конфиг по указанному пути (или рядом с .exe).
    pub fn save(&self, path: Option<&PathBuf>) -> Result<PathBuf> {
        let path = match path {
            Some(p) => p.clone(),
            None => exe_dir_config()?,
        };
        let raw = toml::to_string_pretty(self).context("serialize toml")?;
        std::fs::write(&path, raw).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }
}

/// Возвращает путь к существующему `config.toml` (cwd → exe-dir).
/// Если ни одного нет — возвращает путь рядом с exe для создания нового.
pub fn config_path() -> PathBuf {
    if let Some(p) = find_config_path() {
        return p;
    }
    exe_dir_config().unwrap_or_else(|_| PathBuf::from("config.toml"))
}

fn find_config_path() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("config.toml");
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(p) = exe_dir_config() {
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn exe_dir_config() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("current_exe")?;
    let dir = exe.parent().context("exe has no parent")?;
    Ok(dir.join("config.toml"))
}
