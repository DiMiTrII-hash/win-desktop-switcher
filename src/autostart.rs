use anyhow::{Context, Result};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::w;

const RUN_KEY: windows::core::PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: windows::core::PCWSTR = w!("WinDesktopSwitcher");

/// Включён ли автозапуск (есть ли наша запись в реестре).
pub fn is_enabled() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_READ, &mut hkey).is_err() {
            return false;
        }
        let mut size: u32 = 0;
        let res = RegQueryValueExW(hkey, VALUE_NAME, None, None, None, Some(&mut size));
        let _ = RegCloseKey(hkey);
        res.is_ok() && size > 0
    }
}

/// Включить автозапуск (записать путь к текущему exe в Run-ключ).
pub fn enable() -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let exe_str = exe.to_string_lossy();
    // оборачиваем в кавычки на случай пробелов в пути
    let value = format!("\"{exe_str}\"");
    let mut wide: Vec<u16> = value.encode_utf16().collect();
    wide.push(0); // null-terminator

    unsafe {
        let mut hkey = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_WRITE, &mut hkey)
            .ok()
            .context("RegOpenKeyExW Run")?;

        let bytes: &[u8] = std::slice::from_raw_parts(
            wide.as_ptr() as *const u8,
            wide.len() * std::mem::size_of::<u16>(),
        );
        let res = RegSetValueExW(hkey, VALUE_NAME, None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(hkey);
        res.ok().context("RegSetValueExW")?;
    }
    Ok(())
}

/// Удалить запись автозапуска.
pub fn disable() -> Result<()> {
    unsafe {
        let mut hkey = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_WRITE, &mut hkey)
            .ok()
            .context("RegOpenKeyExW Run")?;

        let res = RegDeleteValueW(hkey, VALUE_NAME);
        let _ = RegCloseKey(hkey);
        // если ключа не было — это ОК, считаем успехом
        if res.is_err() && is_enabled() {
            return Err(anyhow::anyhow!("RegDeleteValueW failed: {:?}", res));
        }
    }
    Ok(())
}

/// Переключить состояние. Возвращает новое состояние (true = включено).
pub fn toggle() -> Result<bool> {
    if is_enabled() {
        disable()?;
        Ok(false)
    } else {
        enable()?;
        Ok(true)
    }
}
