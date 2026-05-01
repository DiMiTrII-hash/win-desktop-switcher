//! Индикатор переключения стола: точки и номер поверх экрана.
//! Layered-окно с fade/slide анимацией, исключено из BitBlt-захвата slide_overlay.

use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering};

use anyhow::Result;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateRoundRectRgn,
    CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH, DEFAULT_QUALITY, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, Ellipse, EndPaint, FF_DONTCARE,
    FW_BOLD, FillRect, FrameRgn, HBRUSH, HDC, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    PS_NULL, SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, GetClientRect, GetSystemMetrics,
    HWND_TOPMOST, IDC_ARROW, KillTimer, LWA_ALPHA, LoadCursorW, PostMessageW, RegisterClassW,
    SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE,
    SetLayeredWindowAttributes, SetTimer, SetWindowDisplayAffinity, SetWindowPos, ShowWindow,
    WDA_EXCLUDEFROMCAPTURE, WM_PAINT, WM_TIMER, WM_USER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

const WM_OVERLAY_SHOW: u32 = WM_USER + 1;
const TIMER_ANIM: usize = 1;
const TICK_MS: u32 = 16;

const WIN_W: i32 = 520;
const WIN_H: i32 = 110;
const RADIUS: i32 = 22;

// Пропорции фаз анимации (% от duration)
const FADE_IN_PCT: u64 = 25;
const FADE_OUT_PCT: u64 = 35;
const SLIDE_PX: i32 = 28;
const MAX_ALPHA: u8 = 235;

// Точки
const DOT_NORMAL: i32 = 12;
const DOT_ACTIVE: i32 = 22;
const DOT_GAP: i32 = 14;

// Цвета BGR (COLORREF = 0x00BBGGRR)
const BG_COLOR: u32 = 0x00181C22;
const BORDER_COLOR: u32 = 0x00353A45;
const DOT_INACTIVE_COLOR: u32 = 0x00606060;
const DOT_ACTIVE_COLOR: u32 = 0x00FFFFFF;
const TEXT_COLOR: u32 = 0x00DDDDDD;

// Состояние для отрисовки
static CURRENT_INDEX: AtomicU32 = AtomicU32::new(0);
static TOTAL_DESKTOPS: AtomicU32 = AtomicU32::new(0);
static DIRECTION: AtomicI32 = AtomicI32::new(0);

static ANIM_START_MS: AtomicU64 = AtomicU64::new(0);
static ANIM_DURATION_MS: AtomicU32 = AtomicU32::new(1100);
static BASE_X: AtomicI32 = AtomicI32::new(0);
static BASE_Y: AtomicI32 = AtomicI32::new(0);

static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_OVERLAY_SHOW => {
            let current = (wp.0 & 0xFFFF) as u32;
            let total = ((wp.0 >> 16) & 0xFFFF) as u32;
            // минимум 900мс, чтобы пользователь успел прочитать
            let duration = (lp.0 as u32).max(900);

            CURRENT_INDEX.store(current, Ordering::Relaxed);
            TOTAL_DESKTOPS.store(total, Ordering::Relaxed);
            ANIM_DURATION_MS.store(duration, Ordering::Relaxed);
            ANIM_START_MS.store(unsafe { GetTickCount64() }, Ordering::Relaxed);

            unsafe {
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = InvalidateRect(Some(hwnd), None, true);
                let _ = KillTimer(Some(hwnd), TIMER_ANIM);
                SetTimer(Some(hwnd), TIMER_ANIM, TICK_MS, None);
            }
            unsafe { tick(hwnd) };
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_ANIM => {
            unsafe { tick(hwnd) };
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                let hdc = BeginPaint(hwnd, &mut ps);
                paint(hwnd, hdc);
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

unsafe fn tick(hwnd: HWND) {
    let now = unsafe { GetTickCount64() };
    let start = ANIM_START_MS.load(Ordering::Relaxed);
    let duration = ANIM_DURATION_MS.load(Ordering::Relaxed) as u64;
    let elapsed = now.saturating_sub(start);

    if elapsed >= duration {
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_ANIM);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        return;
    }

    let fade_in = (duration * FADE_IN_PCT / 100).max(1);
    let fade_out_start = duration.saturating_sub(duration * FADE_OUT_PCT / 100);
    let fade_out_dur = (duration - fade_out_start).max(1);

    let alpha = if elapsed < fade_in {
        let t = elapsed as f32 / fade_in as f32;
        (ease_out_cubic(t) * MAX_ALPHA as f32) as u8
    } else if elapsed < fade_out_start {
        MAX_ALPHA
    } else {
        let t = (elapsed - fade_out_start) as f32 / fade_out_dur as f32;
        (MAX_ALPHA as f32 * (1.0 - ease_in_cubic(t))) as u8
    };

    // slide по X только в фазе fade-in, в направлении переключения
    let direction = DIRECTION.load(Ordering::Relaxed);
    let offset_x = if direction != 0 && elapsed < fade_in {
        let t = elapsed as f32 / fade_in as f32;
        let remaining = 1.0 - ease_out_cubic(t);
        (direction as f32 * SLIDE_PX as f32 * remaining) as i32
    } else {
        0
    };

    let base_x = BASE_X.load(Ordering::Relaxed);
    let base_y = BASE_Y.load(Ordering::Relaxed);

    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            base_x + offset_x,
            base_y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[inline]
fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let p = 1.0 - t;
    1.0 - p * p * p
}

#[inline]
fn ease_in_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

unsafe fn paint(hwnd: HWND, hdc: HDC) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;

    unsafe {
        // фон (регион окна уже скруглён — углы обрезаны)
        let bg = CreateSolidBrush(COLORREF(BG_COLOR));
        FillRect(hdc, &rc, bg);
        let _ = DeleteObject(bg.into());

        // тонкая обводка по рамке региона
        let border_rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, RADIUS * 2, RADIUS * 2);
        let border_brush = CreateSolidBrush(COLORREF(BORDER_COLOR));
        let _ = FrameRgn(hdc, border_rgn, border_brush, 1, 1);
        let _ = DeleteObject(border_rgn.into());
        let _ = DeleteObject(border_brush.into());

        let cur = CURRENT_INDEX.load(Ordering::Relaxed) as i32;
        let total = TOTAL_DESKTOPS.load(Ordering::Relaxed) as i32;
        if total <= 0 {
            return;
        }

        // точки в верхней половине
        let total_dots_w = (total - 1) * DOT_NORMAL + DOT_ACTIVE + (total - 1) * DOT_GAP;
        let dots_start_x = (w - total_dots_w) / 2;
        let dots_center_y = h / 2 - 14;

        let null_pen = CreatePen(PS_NULL, 0, COLORREF(0));
        let old_pen = SelectObject(hdc, null_pen.into());

        let brush_active = CreateSolidBrush(COLORREF(DOT_ACTIVE_COLOR));
        let brush_inactive = CreateSolidBrush(COLORREF(DOT_INACTIVE_COLOR));

        let mut x = dots_start_x;
        for i in 0..total {
            let active = i == cur;
            let size = if active { DOT_ACTIVE } else { DOT_NORMAL };
            let brush = if active { brush_active } else { brush_inactive };
            let old_brush = SelectObject(hdc, brush.into());
            let top = dots_center_y - size / 2;
            let _ = Ellipse(hdc, x, top, x + size, top + size);
            SelectObject(hdc, old_brush);
            x += size + DOT_GAP;
        }

        let _ = DeleteObject(brush_active.into());
        let _ = DeleteObject(brush_inactive.into());
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(null_pen.into());

        // текст "current / total" в нижней половине
        let text = format!("{} / {}", cur + 1, total);
        let mut wide: Vec<u16> = text.encode_utf16().collect();

        let mut text_rc = RECT {
            left: rc.left,
            right: rc.right,
            top: h / 2 + 4,
            bottom: rc.bottom - 6,
        };

        let font = CreateFontW(
            30,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Segoe UI"),
        );
        let old_font = SelectObject(hdc, font.into());

        SetTextColor(hdc, COLORREF(TEXT_COLOR));
        SetBkMode(hdc, TRANSPARENT);

        DrawTextW(
            hdc,
            &mut wide,
            &mut text_rc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        SelectObject(hdc, old_font);
        let _ = DeleteObject(font.into());
    }
}

pub fn install() -> Result<()> {
    let class_name = w!("WdsOverlayClass");
    let title = w!("WdsOverlay");

    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let cursor = LoadCursorW(None, IDC_ARROW)?;

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: class_name,
            hCursor: cursor,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        // bottom-center, ~36% от низа (как было)
        let x = (screen_w - WIN_W) / 2;
        let y = screen_h - (screen_h * 36 / 100) - WIN_H / 2;
        BASE_X.store(x, Ordering::Relaxed);
        BASE_Y.store(y, Ordering::Relaxed);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE,
            class_name,
            title,
            WS_POPUP,
            x,
            y,
            WIN_W,
            WIN_H,
            None,
            None,
            Some(hinst.into()),
            None,
        )?;

        // скруглённые углы
        let rgn = CreateRoundRectRgn(0, 0, WIN_W + 1, WIN_H + 1, RADIUS * 2, RADIUS * 2);
        SetWindowRgn(hwnd, Some(rgn), true);

        // не попадать в BitBlt slide_overlay
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);

        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);

        OVERLAY_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
    }
    Ok(())
}

/// Показать overlay. `duration_ms` — минимум 900мс (иначе будет 900).
pub fn show(current_index: u32, total: u32, duration_ms: u32, direction: i32) {
    let raw = OVERLAY_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut _);
    DIRECTION.store(direction.signum(), Ordering::Relaxed);

    let cur = current_index.min(0xFFFF);
    let tot = total.min(0xFFFF);
    let wp = WPARAM(((tot as usize) << 16) | (cur as usize));
    let lp = LPARAM(duration_ms as isize);

    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_OVERLAY_SHOW, wp, lp);
    }
}
