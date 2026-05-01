use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use anyhow::{Result, anyhow};
use crossbeam_channel::Sender;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_1, VK_2, VK_3, VK_4, VK_5, VK_6, VK_7, VK_8, VK_9, VK_CONTROL, VK_LEFT,
    VK_LSHIFT, VK_LWIN, VK_RIGHT, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, SetWindowsHookExW, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// Команды от hook → worker thread.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    SwitchPrev,
    SwitchNext,
    SwitchTo(u32),
    MovePrev,
    MoveNext,
}

// hook state — читается также из wheel.rs
pub static WIN_PRESSED: AtomicBool = AtomicBool::new(false);
pub static SHIFT_PRESSED: AtomicBool = AtomicBool::new(false);
/// Флаг "Win был использован для нашего хоткея" — при keyup гасим открытие Пуска
pub static CONSUMED_WIN: AtomicBool = AtomicBool::new(false);
pub static SENDER: OnceLock<Sender<Action>> = OnceLock::new();

// Для cooldown при auto-repeat зажатой стрелки
static LAST_KEY_SWITCH_MS: AtomicU64 = AtomicU64::new(0);
static COOLDOWN_MS: AtomicU32 = AtomicU32::new(150);

#[inline]
pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}
// HHOOK не Sync — не храним глобально; Windows сама снимет хук при завершении процесса.

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let kbd = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };

    // игнорируем то что сами инжектируем (dummy Ctrl и т.п.)
    if kbd.flags.0 & LLKHF_INJECTED.0 != 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let vk = VIRTUAL_KEY(kbd.vkCode as u16);
    let event = wparam.0 as u32;
    let is_down = event == WM_KEYDOWN || event == WM_SYSKEYDOWN;
    let is_up = event == WM_KEYUP || event == WM_SYSKEYUP;

    // Win key — следим за нажатием/отпусканием
    if vk == VK_LWIN || vk == VK_RWIN {
        if is_down {
            WIN_PRESSED.store(true, Ordering::Relaxed);
        } else if is_up {
            WIN_PRESSED.store(false, Ordering::Relaxed);
            // Если Win использовался как модификатор — гасим Start menu
            if CONSUMED_WIN.swap(false, Ordering::Relaxed) {
                inject_dummy_ctrl();
            }
        }
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // Shift — для Win+Shift+стрелки
    if vk == VK_LSHIFT || vk == VK_RSHIFT || vk == VK_SHIFT {
        if is_down {
            SHIFT_PRESSED.store(true, Ordering::Relaxed);
        } else if is_up {
            SHIFT_PRESSED.store(false, Ordering::Relaxed);
        }
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // Дальше — только при зажатой Win
    if !WIN_PRESSED.load(Ordering::Relaxed) {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let shift = SHIFT_PRESSED.load(Ordering::Relaxed);

    // Определяем action по клавише (только наши целевые клавиши)
    let action = match vk {
        VK_LEFT => Some(if shift { Action::MovePrev } else { Action::SwitchPrev }),
        VK_RIGHT => Some(if shift { Action::MoveNext } else { Action::SwitchNext }),
        VK_1 => Some(Action::SwitchTo(0)),
        VK_2 => Some(Action::SwitchTo(1)),
        VK_3 => Some(Action::SwitchTo(2)),
        VK_4 => Some(Action::SwitchTo(3)),
        VK_5 => Some(Action::SwitchTo(4)),
        VK_6 => Some(Action::SwitchTo(5)),
        VK_7 => Some(Action::SwitchTo(6)),
        VK_8 => Some(Action::SwitchTo(7)),
        VK_9 => Some(Action::SwitchTo(8)),
        _ => None,
    };

    let Some(act) = action else {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    };

    // На KEYDOWN — отправляем action с cooldown'ом (защита от auto-repeat),
    // но блокируем и DOWN, и UP — чтобы Windows не видел Win+стрелка и не тригерил Snap
    if is_down {
        let now = now_ms();
        let last = LAST_KEY_SWITCH_MS.load(Ordering::Relaxed);
        let cooldown = COOLDOWN_MS.load(Ordering::Relaxed) as u64;

        if now.saturating_sub(last) >= cooldown {
            if let Some(tx) = SENDER.get() {
                let _ = tx.try_send(act);
            }
            LAST_KEY_SWITCH_MS.store(now, Ordering::Relaxed);
        }
        // Отмечаем что Win использовался — даже если cooldown отфильтровал этот конкретный раз
        CONSUMED_WIN.store(true, Ordering::Relaxed);
    }
    // 1 = заблокировать дальнейшую обработку (не пропускать к ОС)
    let _ = is_up;
    LRESULT(1)
}

/// Посылает виртуальный VK_CONTROL down/up чтобы погасить активацию Start menu
/// после того как мы перехватили Win+что-то.
fn inject_dummy_ctrl() {
    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_CONTROL,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_CONTROL,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Устанавливает глобальный low-level keyboard hook. Должен вызываться из потока,
/// в котором будет крутиться message loop (`GetMessageW`).
pub fn install(sender: Sender<Action>, cooldown_ms: u32) -> Result<()> {
    SENDER
        .set(sender)
        .map_err(|_| anyhow!("hotkey hook already installed"))?;
    COOLDOWN_MS.store(cooldown_ms, Ordering::Relaxed);

    unsafe {
        let hmod = GetModuleHandleW(None)?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hmod.into()), 0)?;
    }
    Ok(())
}

/// Обновляет cooldown без перезапуска hook'а (для reload config).
pub fn set_cooldown(ms: u32) {
    COOLDOWN_MS.store(ms, Ordering::Relaxed);
}
