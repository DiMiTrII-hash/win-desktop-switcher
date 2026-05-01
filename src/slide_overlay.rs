//! Slide-push («плёнка»): два стола соединены в широком layered-окне и едут вместе.
//!
//! Чтобы снять скриншот соседнего стола без видимого flicker'а:
//! сначала BitBlt текущего (A), потом показываем cover с этим скриншотом
//! (с WDA_EXCLUDEFROMCAPTURE), переключаем стол под cover'ом, ждём отрисовки
//! и BitBlt второй раз — cover в захват не попадает, получаем чистый B.

use std::sync::OnceLock;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_OVER, BLENDFUNCTION, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC,
    DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, IDC_ARROW, LoadCursorW,
    RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE,
    SetWindowDisplayAffinity, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow,
    WDA_EXCLUDEFROMCAPTURE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

static CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn ensure_class() {
    CLASS_REGISTERED.get_or_init(|| unsafe {
        let hinst = GetModuleHandleW(None).expect("GetModuleHandleW");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: w!("WdsSlideOverlay"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
    });
}

/// Проигрывает «плёнку» для переключения на соседний стол. `switch_fn` вызывается
/// когда cover уже закрыл экран — пользователь не видит системного переключения.
pub fn play_slide_push<F>(
    direction: i32,
    duration_ms: u32,
    easing: &str,
    switch_fn: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    if direction == 0 {
        return switch_fn();
    }
    ensure_class();

    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            anyhow::bail!("invalid screen metrics");
        }

        let screen_dc = GetDC(None);
        if screen_dc.0.is_null() {
            anyhow::bail!("GetDC(None) null");
        }

        // снимаем текущий экран
        let mem_a = CreateCompatibleDC(Some(screen_dc));
        let bmp_a = CreateCompatibleBitmap(screen_dc, width, height);
        let prev_a = SelectObject(mem_a, HGDIOBJ(bmp_a.0));
        BitBlt(mem_a, 0, 0, width, height, Some(screen_dc), 0, 0, SRCCOPY)
            .context("BitBlt A")?;

        // сcover-окно с A поверх экрана, исключено из capture
        let cover = create_layered(width, height).context("create cover")?;
        let _ = SetWindowDisplayAffinity(cover, WDA_EXCLUDEFROMCAPTURE);

        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: 0,
        };
        let src_pos = POINT { x: 0, y: 0 };
        let size_one = SIZE { cx: width, cy: height };
        let cover_pos = POINT { x: 0, y: 0 };
        UpdateLayeredWindow(
            cover,
            Some(screen_dc),
            Some(&cover_pos),
            Some(&size_one),
            Some(mem_a),
            Some(&src_pos),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
        .context("UpdateLayeredWindow cover")?;
        let _ = ShowWindow(cover, SW_SHOWNOACTIVATE);

        // переключаем стол под cover'ом
        if let Err(e) = switch_fn() {
            // откат: уничтожаем всё и пробрасываем ошибку
            let _ = DestroyWindow(cover);
            SelectObject(mem_a, prev_a);
            let _ = DeleteObject(HGDIOBJ(bmp_a.0));
            let _ = DeleteDC(mem_a);
            ReleaseDC(None, screen_dc);
            return Err(e);
        }

        // даём время DWM отрисовать новый стол и снимаем B (cover не попадёт в capture)
        sleep(Duration::from_millis(60));
        let mem_b = CreateCompatibleDC(Some(screen_dc));
        let bmp_b = CreateCompatibleBitmap(screen_dc, width, height);
        let prev_b = SelectObject(mem_b, HGDIOBJ(bmp_b.0));
        BitBlt(mem_b, 0, 0, width, height, Some(screen_dc), 0, 0, SRCCOPY)
            .context("BitBlt B")?;

        // собираем широкий bitmap 2W×H из A и B
        let wide_mem = CreateCompatibleDC(Some(screen_dc));
        let wide_bmp = CreateCompatibleBitmap(screen_dc, 2 * width, height);
        let prev_wide = SelectObject(wide_mem, HGDIOBJ(wide_bmp.0));

        // NEXT: [A|B] едет влево (0 → -W); PREV: [B|A] едет вправо (-W → 0)
        if direction > 0 {
            BitBlt(wide_mem, 0, 0, width, height, Some(mem_a), 0, 0, SRCCOPY)?;
            BitBlt(wide_mem, width, 0, width, height, Some(mem_b), 0, 0, SRCCOPY)?;
        } else {
            BitBlt(wide_mem, 0, 0, width, height, Some(mem_b), 0, 0, SRCCOPY)?;
            BitBlt(wide_mem, width, 0, width, height, Some(mem_a), 0, 0, SRCCOPY)?;
        }

        // широкое layered-окно
        let wide_hwnd = create_layered(2 * width, height).context("create wide")?;
        let _ = SetWindowDisplayAffinity(wide_hwnd, WDA_EXCLUDEFROMCAPTURE);

        let initial_x = if direction > 0 { 0 } else { -width };
        let final_x = if direction > 0 { -width } else { 0 };

        let size_wide = SIZE { cx: 2 * width, cy: height };
        let wide_pos = POINT { x: initial_x, y: 0 };
        UpdateLayeredWindow(
            wide_hwnd,
            Some(screen_dc),
            Some(&wide_pos),
            Some(&size_wide),
            Some(wide_mem),
            Some(&src_pos),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
        .context("UpdateLayeredWindow wide")?;
        let _ = ShowWindow(wide_hwnd, SW_SHOWNOACTIVATE);

        // wide уже сверху — cover больше не нужен
        let _ = DestroyWindow(cover);

        // анимация
        let dur = duration_ms.max(16) as f32;
        let start_xf = initial_x as f32;
        let delta = (final_x - initial_x) as f32;
        let t0 = Instant::now();
        loop {
            let ms = t0.elapsed().as_secs_f32() * 1000.0;
            let t = ms / dur;
            if t >= 1.0 {
                break;
            }
            let eased = apply_easing(easing, t);
            let x = (start_xf + delta * eased) as i32;
            let _ = SetWindowPos(
                wide_hwnd,
                None,
                x,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE,
            );
            sleep(Duration::from_millis(8));
        }
        let _ = SetWindowPos(
            wide_hwnd,
            None,
            final_x,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE,
        );

        // cleanup
        let _ = DestroyWindow(wide_hwnd);

        SelectObject(wide_mem, prev_wide);
        let _ = DeleteObject(HGDIOBJ(wide_bmp.0));
        let _ = DeleteDC(wide_mem);

        SelectObject(mem_b, prev_b);
        let _ = DeleteObject(HGDIOBJ(bmp_b.0));
        let _ = DeleteDC(mem_b);

        SelectObject(mem_a, prev_a);
        let _ = DeleteObject(HGDIOBJ(bmp_a.0));
        let _ = DeleteDC(mem_a);

        ReleaseDC(None, screen_dc);
    }
    Ok(())
}

unsafe fn create_layered(w: i32, h: i32) -> Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None).context("GetModuleHandleW")?;
        CreateWindowExW(
            WS_EX_LAYERED
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE
                | WS_EX_TRANSPARENT,
            w!("WdsSlideOverlay"),
            w!(""),
            WS_POPUP,
            0,
            0,
            w,
            h,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .context("CreateWindowExW")
    }
}

pub fn apply_easing(s: &str, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let s = s.trim().to_ascii_lowercase();

    if s == "linear" {
        return t;
    }
    if let Some(rest) = s.strip_prefix("cubic-bezier(") {
        if let Some(inner) = rest.strip_suffix(')') {
            let nums: Vec<f32> = inner
                .split(',')
                .filter_map(|x| x.trim().parse::<f32>().ok())
                .collect();
            if nums.len() == 4 {
                return cubic_bezier(nums[0], nums[1], nums[2], nums[3], t);
            }
        }
        return ease_out_cubic(t);
    }
    if s.contains("ease-in-out") {
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            let p = -2.0 * t + 2.0;
            1.0 - p * p * p / 2.0
        }
    } else if s.contains("ease-out") {
        ease_out_cubic(t)
    } else if s.contains("ease-in") {
        t * t * t
    } else {
        ease_out_cubic(t)
    }
}

#[inline]
fn ease_out_cubic(t: f32) -> f32 {
    let p = 1.0 - t;
    1.0 - p * p * p
}

fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let mut u = x;
    for _ in 0..6 {
        let bx = bez(x1, x2, u);
        let dx = bez_d(x1, x2, u);
        if dx.abs() < 1e-6 {
            break;
        }
        u = (u - (bx - x) / dx).clamp(0.0, 1.0);
    }
    bez(y1, y2, u)
}

#[inline]
fn bez(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
}

#[inline]
fn bez_d(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

