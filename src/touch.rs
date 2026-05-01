//! Touch injection — симулируем 4-пальцевый свайп, чтобы триггерить
//! нативную slide-анимацию Windows 11 при переключении виртуальных столов.
//!
//! Работает если в Windows включено:
//! Settings → Bluetooth & devices → Touchpad → Four-finger gestures
//! → "Switch desktops and show desktop".

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::UI::Input::Pointer::{
    InitializeTouchInjection, InjectTouchInput, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_INRANGE, POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_FLAGS, POINTER_INFO,
    POINTER_TOUCH_INFO, TOUCH_FEEDBACK_INDIRECT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, PT_TOUCH, SM_CXSCREEN, SM_CYSCREEN, TOUCH_MASK_CONTACTAREA,
    TOUCH_MASK_ORIENTATION, TOUCH_MASK_PRESSURE,
};

use crate::config::Touch as TouchCfg;

/// Количество контактов (пальцев).
const FINGERS: usize = 4;
/// Расстояние между пальцами по горизонтали (px).
const FINGER_SPACING: i32 = 60;

// runtime-параметры из config
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
static DISTANCE: AtomicI32 = AtomicI32::new(650);
static STEPS: AtomicU32 = AtomicU32::new(28);
static STEP_DELAY_MS: AtomicU64 = AtomicU64::new(11);
static HOLD_BEFORE_MS: AtomicU64 = AtomicU64::new(15);
static HOLD_AFTER_MS: AtomicU64 = AtomicU64::new(35);
/// 0 = linear, 1 = ease-in, 2 = ease-out, 3 = ease-in-out
static EASING: AtomicU8 = AtomicU8::new(1);

/// Инициализирует touch-инжектор и записывает параметры. Один раз на процесс.
pub fn init(cfg: &TouchCfg) -> Result<()> {
    set_params(cfg);

    if !cfg.enabled {
        return Ok(());
    }
    if INITIALIZED.load(Ordering::Relaxed) {
        return Ok(());
    }
    unsafe {
        InitializeTouchInjection(FINGERS as u32, TOUCH_FEEDBACK_INDIRECT)
            .context("InitializeTouchInjection (требуется поддержка touch или admin)")?;
    }
    INITIALIZED.store(true, Ordering::Relaxed);
    Ok(())
}

/// Обновляет параметры без переинициализации (для reload config).
pub fn set_params(cfg: &TouchCfg) {
    ENABLED.store(cfg.enabled, Ordering::Relaxed);
    DISTANCE.store(cfg.swipe_distance_px.max(50), Ordering::Relaxed);
    STEPS.store(cfg.swipe_steps.max(2), Ordering::Relaxed);
    STEP_DELAY_MS.store(cfg.step_delay_ms.max(1), Ordering::Relaxed);
    HOLD_BEFORE_MS.store(cfg.hold_before_ms, Ordering::Relaxed);
    HOLD_AFTER_MS.store(cfg.hold_after_ms, Ordering::Relaxed);
    EASING.store(parse_easing(&cfg.easing), Ordering::Relaxed);
}

fn parse_easing(s: &str) -> u8 {
    match s.trim().to_ascii_lowercase().as_str() {
        "ease-in" => 1,
        "ease-out" => 2,
        "ease-in-out" => 3,
        _ => 0,
    }
}

#[inline]
fn apply_easing(code: u8, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match code {
        1 => t * t * t, // ease-in cubic
        2 => {
            let p = 1.0 - t;
            1.0 - p * p * p
        }
        3 => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                let p = -2.0 * t + 2.0;
                1.0 - p * p * p / 2.0
            }
        }
        _ => t, // linear
    }
}

/// Симулирует 4-пальцевый свайп для переключения стола.
/// `direction`: -1 = предыдущий стол (swipe ВПРАВО), +1 = следующий (swipe ВЛЕВО).
pub fn swipe(direction: i32) -> Result<()> {
    if !ENABLED.load(Ordering::Relaxed) || !INITIALIZED.load(Ordering::Relaxed) {
        return Err(anyhow!("touch injection disabled or not initialized"));
    }
    if direction == 0 {
        return Ok(());
    }

    let distance = DISTANCE.load(Ordering::Relaxed);
    let steps = STEPS.load(Ordering::Relaxed);
    let step_delay = STEP_DELAY_MS.load(Ordering::Relaxed);
    let hold_before = HOLD_BEFORE_MS.load(Ordering::Relaxed);
    let hold_after = HOLD_AFTER_MS.load(Ordering::Relaxed);
    let easing = EASING.load(Ordering::Relaxed);

    // Знак dx: prev (-1) = swipe вправо (dx +), next (+1) = swipe влево (dx -)
    let total_dx = -direction.signum() * distance;

    // Базовая точка (центр primary monitor)
    let (sw, sh) = unsafe {
        (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
    };
    let cx = sw / 2;
    let cy = sh / 2;

    let start_x: [i32; FINGERS] = [
        cx - FINGER_SPACING * 3 / 2,
        cx - FINGER_SPACING / 2,
        cx + FINGER_SPACING / 2,
        cx + FINGER_SPACING * 3 / 2,
    ];
    let start_y: i32 = cy;

    let mut contacts: [POINTER_TOUCH_INFO; FINGERS] =
        unsafe { std::mem::zeroed() };

    // DOWN
    for i in 0..FINGERS {
        contacts[i] = make_contact(
            i as u32,
            start_x[i],
            start_y,
            POINTER_FLAG_DOWN | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT,
        );
    }
    unsafe {
        InjectTouchInput(&contacts).context("InjectTouchInput DOWN")?;
    }
    if hold_before > 0 {
        sleep(Duration::from_millis(hold_before));
    }

    // MOVE с easing
    for step in 1..=steps {
        let progress = step as f32 / steps as f32;
        let eased = apply_easing(easing, progress);
        let dx = (total_dx as f32 * eased) as i32;

        for i in 0..FINGERS {
            contacts[i] = make_contact(
                i as u32,
                start_x[i] + dx,
                start_y,
                POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT,
            );
        }
        unsafe {
            // ошибка на одном кадре не должна прервать свайп
            let _ = InjectTouchInput(&contacts);
        }
        sleep(Duration::from_millis(step_delay));
    }

    // HOLD — держим позицию в конце
    if hold_after > 0 {
        // закрепляем последнюю позицию с UPDATE + INCONTACT ещё одним кадром
        for i in 0..FINGERS {
            contacts[i] = make_contact(
                i as u32,
                start_x[i] + total_dx,
                start_y,
                POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT,
            );
        }
        unsafe {
            let _ = InjectTouchInput(&contacts);
        }
        sleep(Duration::from_millis(hold_after));
    }

    // UP
    for i in 0..FINGERS {
        contacts[i] = make_contact(
            i as u32,
            start_x[i] + total_dx,
            start_y,
            POINTER_FLAG_UP,
        );
    }
    unsafe {
        InjectTouchInput(&contacts).context("InjectTouchInput UP")?;
    }

    Ok(())
}

fn make_contact(id: u32, x: i32, y: i32, flags: POINTER_FLAGS) -> POINTER_TOUCH_INFO {
    let mut info: POINTER_TOUCH_INFO = unsafe { std::mem::zeroed() };
    info.pointerInfo = POINTER_INFO {
        pointerType: PT_TOUCH,
        pointerId: id,
        pointerFlags: flags,
        ptPixelLocation: POINT { x, y },
        ..unsafe { std::mem::zeroed() }
    };
    info.touchFlags = 0;
    info.touchMask = TOUCH_MASK_CONTACTAREA | TOUCH_MASK_ORIENTATION | TOUCH_MASK_PRESSURE;
    info.rcContact = RECT {
        left: x - 2,
        top: y - 2,
        right: x + 2,
        bottom: y + 2,
    };
    info.orientation = 90;
    info.pressure = 32000;
    info
}
