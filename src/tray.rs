use std::sync::atomic::{AtomicIsize, Ordering};

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, ShellExecuteW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, GetCursorPos,
    HMENU, HWND_MESSAGE, IDI_APPLICATION, LoadIconW, MENU_ITEM_FLAGS, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOWNORMAL,
    SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu,
    WINDOW_EX_STYLE, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WM_USER,
    WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use crate::{autostart, config::Config, hotkey, touch, wheel};

const WM_TRAY: u32 = WM_USER + 100;
const TRAY_ICON_ID: u32 = 1;

const ID_RELOAD: u32 = 1001;
const ID_OPEN_CONFIG: u32 = 1002;
const ID_AUTOSTART: u32 = 1003;
const ID_QUIT: u32 = 1010;

static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = lp.0 as u32 & 0xFFFF;
            match event {
                WM_RBUTTONUP => unsafe {
                    show_menu(hwnd);
                },
                WM_LBUTTONDBLCLK => unsafe {
                    show_menu(hwnd);
                },
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xFFFF) as u32;
            handle_command(id);
            LRESULT(0)
        }
        WM_DESTROY => {
            remove_icon();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        let autostart_flag = if autostart::is_enabled() {
            MF_CHECKED.0
        } else {
            0
        };

        let _ = AppendMenuW(menu, MF_STRING, ID_RELOAD as usize, w!("Reload config"));
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN_CONFIG as usize, w!("Open config.toml"));
        let _ = AppendMenuW(
            menu,
            MENU_ITEM_FLAGS(MF_STRING.0 | autostart_flag),
            ID_AUTOSTART as usize,
            w!("Run at startup"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT as usize, w!("Quit"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);

        // Без этого меню не закроется по клику вне него
        let _ = SetForegroundWindow(hwnd);

        // TrackPopupMenu отправит WM_COMMAND обратно в наш WndProc
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );

        // Дёргаем в окно "пустое" сообщение чтобы исправить баг с залипанием меню
        let _ = PostMessageW(Some(hwnd), 0, WPARAM(0), LPARAM(0));

        let _ = DestroyMenu(menu);
        let _ = menu_unused(HMENU::default()); // anti dead_code helper
    }
}

fn menu_unused(_h: HMENU) {} // helper to keep HMENU import non-warning

fn handle_command(id: u32) {
    match id {
        ID_RELOAD => match reload_config() {
            Ok(()) => eprintln!("[tray] config reloaded"),
            Err(e) => eprintln!("[tray] reload error: {e:?}"),
        },
        ID_OPEN_CONFIG => open_config_file(),
        ID_AUTOSTART => match autostart::toggle() {
            Ok(now_on) => eprintln!("[tray] autostart = {now_on}"),
            Err(e) => eprintln!("[tray] autostart error: {e:?}"),
        },
        ID_QUIT => unsafe {
            PostQuitMessage(0);
        },
        _ => {}
    }
}

fn reload_config() -> Result<()> {
    let (cfg, _) = Config::load().context("load config")?;
    hotkey::set_cooldown(cfg.wheel.cooldown_ms);
    wheel::set_params(
        cfg.wheel.threshold,
        cfg.wheel.cooldown_ms,
        cfg.wheel.velocity_damping,
        cfg.wheel.hot_zone_height_px,
    );
    touch::set_params(&cfg.touch);
    Ok(())
}

fn open_config_file() {
    let path = crate::config::config_path();
    // если файла нет — создаём дефолт
    if !path.exists() {
        let _ = Config::default().save(Some(&path));
    }
    let path_wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain([0])
        .collect();

    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(path_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Создаёт иконку в трее. Должно вызываться из потока с message loop.
pub fn install() -> Result<()> {
    let class_name = w!("WdsTrayClass");

    unsafe {
        let hinst = GetModuleHandleW(None)?;

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("WdsTray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )?;

        TRAY_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

        let icon = LoadIconW(None, IDI_APPLICATION)?;

        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            ..Default::default()
        };

        // tooltip
        let tip = "Win Desktop Switcher";
        let tip_wide: Vec<u16> = tip.encode_utf16().chain([0]).collect();
        let max = nid.szTip.len().saturating_sub(1);
        for (i, &c) in tip_wide.iter().enumerate().take(max) {
            nid.szTip[i] = c;
        }

        Shell_NotifyIconW(NIM_ADD, &nid)
            .ok()
            .context("Shell_NotifyIconW NIM_ADD")?;
    }
    Ok(())
}

fn remove_icon() {
    let raw = TRAY_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut _);
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}
