use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

use anyhow::{Result, anyhow};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetSystemMetrics, LLMHF_INJECTED, MSLLHOOKSTRUCT, SM_CYSCREEN,
    SetWindowsHookExW, WH_MOUSE_LL, WM_MOUSEWHEEL,
};

use crate::hotkey::{Action, CONSUMED_WIN, SENDER, WIN_PRESSED, now_ms};

// Параметры из config (выставляются один раз в install())
static THRESHOLD: AtomicI32 = AtomicI32::new(120);
static COOLDOWN_MS: AtomicU32 = AtomicU32::new(150);
/// Хранится как `(damping * 1000) as u32` чтобы избежать AtomicF32
static DAMPING_X1000: AtomicU32 = AtomicU32::new(400);
/// Высота hot-zone внизу экрана в px. 0 = выключено.
static HOT_ZONE_PX: AtomicI32 = AtomicI32::new(60);

// Runtime-состояние
static ACCUMULATOR: AtomicI32 = AtomicI32::new(0);
static LAST_EVENT_MS: AtomicU64 = AtomicU64::new(0);
static LAST_SWITCH_MS: AtomicU64 = AtomicU64::new(0);
static BURST_COUNT: AtomicI32 = AtomicI32::new(0);

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // нас интересует только колесо
    if wparam.0 as u32 != WM_MOUSEWHEEL {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let ms = unsafe { *(lparam.0 as *const MSLLHOOKSTRUCT) };

    // игнорируем события которые сами инжектируем (на будущее)
    if ms.flags & LLMHF_INJECTED != 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let win_pressed = WIN_PRESSED.load(Ordering::Relaxed);

    // Hot-zone: если курсор в нижней полоске экрана — переключаем без Win
    let hot_zone = HOT_ZONE_PX.load(Ordering::Relaxed);
    let in_hot_zone = if hot_zone > 0 {
        let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        ms.pt.y >= screen_h - hot_zone
    } else {
        false
    };

    // Активируемся только если Win зажат ИЛИ курсор в hot-zone
    if !win_pressed && !in_hot_zone {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // wheel delta лежит в high word поля mouseData как signed int16
    let raw = (ms.mouseData >> 16) as i16;
    let delta = raw as i32;
    if delta == 0 {
        // wheel не двигался — ничего не делаем, но всё равно подавляем (Win зажат)
        return LRESULT(1);
    }

    // если активировались через Win — отмечаем что он был использован,
    // чтобы при его отпускании не открылся Пуск
    if win_pressed {
        CONSUMED_WIN.store(true, Ordering::Relaxed);
    }

    let now = now_ms();
    let last_switch = LAST_SWITCH_MS.load(Ordering::Relaxed);
    let cooldown = COOLDOWN_MS.load(Ordering::Relaxed) as u64;

    // Во время cooldown — полностью игнорируем event (никакого накопления)
    if now.saturating_sub(last_switch) < cooldown {
        return LRESULT(1);
    }

    // Velocity damping — повышаем threshold при быстрых сериях
    let last_event = LAST_EVENT_MS.load(Ordering::Relaxed);
    let burst = if now.saturating_sub(last_event) < 50 {
        BURST_COUNT.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        BURST_COUNT.store(0, Ordering::Relaxed);
        0
    };
    LAST_EVENT_MS.store(now, Ordering::Relaxed);

    let base_threshold = THRESHOLD.load(Ordering::Relaxed);
    let damping = DAMPING_X1000.load(Ordering::Relaxed) as f32 / 1000.0;
    let eff_threshold = (base_threshold as f32 * (1.0 + damping * burst as f32)) as i32;

    // Реверс направления — сбрасываем накопитель
    let acc_now = ACCUMULATOR.load(Ordering::Relaxed);
    if acc_now != 0 && acc_now.signum() != delta.signum() {
        ACCUMULATOR.store(0, Ordering::Relaxed);
    }

    let new_acc = ACCUMULATOR.fetch_add(delta, Ordering::Relaxed) + delta;

    if new_acc.abs() >= eff_threshold {
        // В Windows wheel: положительный delta = вверх = "предыдущий стол",
        // отрицательный = вниз = "следующий стол". Это естественно для пользователя.
        let direction = new_acc.signum();
        let action = if direction > 0 {
            Action::SwitchPrev
        } else {
            Action::SwitchNext
        };

        ACCUMULATOR.store(0, Ordering::Relaxed);
        LAST_SWITCH_MS.store(now, Ordering::Relaxed);

        if let Some(tx) = SENDER.get() {
            let _ = tx.try_send(action);
        }
    }

    // подавляем wheel чтобы он не скроллил окно под курсором
    LRESULT(1)
}

/// Устанавливает global low-level mouse hook. Вызывать из потока с message loop.
pub fn install(threshold: i32, cooldown_ms: u32, damping: f32, hot_zone_px: i32) -> Result<()> {
    set_params(threshold, cooldown_ms, damping, hot_zone_px);

    if SENDER.get().is_none() {
        return Err(anyhow!(
            "hotkey::install must be called before wheel::install (SENDER not set)"
        ));
    }

    unsafe {
        let hmod = GetModuleHandleW(None)?;
        SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(hmod.into()), 0)?;
    }
    Ok(())
}

/// Обновляет параметры wheel без перезапуска hook'а (для reload config).
pub fn set_params(threshold: i32, cooldown_ms: u32, damping: f32, hot_zone_px: i32) {
    THRESHOLD.store(threshold, Ordering::Relaxed);
    COOLDOWN_MS.store(cooldown_ms, Ordering::Relaxed);
    DAMPING_X1000.store((damping.clamp(0.0, 5.0) * 1000.0) as u32, Ordering::Relaxed);
    HOT_ZONE_PX.store(hot_zone_px.max(0), Ordering::Relaxed);
}
