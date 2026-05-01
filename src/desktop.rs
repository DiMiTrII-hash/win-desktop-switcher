use anyhow::Result;
use winvd::{get_current_desktop, get_desktop_count, get_desktops, switch_desktop};

/// Расширение для `Result<T, winvd::Error>` — даёт человекочитаемый `anyhow::Error`,
/// потому что `winvd::Error` не реализует `std::error::Error`.
trait WinvdExt<T> {
    fn ctx(self, what: &'static str) -> Result<T>;
}

impl<T> WinvdExt<T> for std::result::Result<T, winvd::Error> {
    fn ctx(self, what: &'static str) -> Result<T> {
        self.map_err(|e| anyhow::anyhow!("{}: {:?}", what, e))
    }
}

/// Тонкая обёртка над winvd с удобным API и навигацией с wrap.
pub struct DesktopManager;

impl DesktopManager {
    pub fn new() -> Result<Self> {
        // smoke probe
        let count = get_desktop_count().ctx("winvd::get_desktop_count")?;
        if count == 0 {
            anyhow::bail!("no virtual desktops detected (winvd returned 0)");
        }
        Ok(Self)
    }

    pub fn count(&self) -> Result<u32> {
        get_desktop_count().ctx("winvd::get_desktop_count")
    }

    pub fn current_index(&self) -> Result<u32> {
        let cur = get_current_desktop().ctx("winvd::get_current_desktop")?;
        cur.get_index().ctx("Desktop::get_index")
    }

    pub fn names(&self) -> Result<Vec<String>> {
        let desktops = get_desktops().ctx("winvd::get_desktops")?;
        desktops
            .iter()
            .map(|d| d.get_name().ctx("Desktop::get_name"))
            .collect()
    }

    /// Переключиться на конкретный индекс.
    pub fn switch_to(&self, index: u32) -> Result<()> {
        switch_desktop(index).ctx("winvd::switch_desktop")
    }

    /// Сдвиг относительно текущего стола. `delta` может быть отрицательным.
    /// Если `wrap = true` — циклично, иначе клампится в [0, count-1].
    pub fn switch_relative(&self, delta: i32, wrap: bool) -> Result<()> {
        let count = self.count()? as i32;
        if count == 0 {
            return Ok(());
        }
        let cur = self.current_index()? as i32;
        let target = if wrap {
            ((cur + delta) % count + count) % count
        } else {
            (cur + delta).clamp(0, count - 1)
        };
        if target != cur {
            self.switch_to(target as u32)?;
        }
        Ok(())
    }
}
