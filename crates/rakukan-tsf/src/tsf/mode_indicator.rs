//! モードインジケーター（カーソル近くに一時表示）
//!
//! IME のモード切替時にキャレット付近に「あ」「ア」「A」を短時間表示する。
//! mozc の IndicatorWindow に相当する機能。
//!
//! # 表示仕様
//! - WS_POPUP + WS_EX_TOPMOST + WS_EX_NOACTIVATE（フォーカスを奪わない）
//! - モード文字を 1 文字表示（100% で 32x32、ウィンドウ DPI に追従）
//! - 表示後 1.5 秒でフェードアウト開始、約 0.5 秒で完全に消える
//! - キー入力があれば即非表示

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
    Win32::{
        Foundation::{BOOL, COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            BACKGROUND_MODE, BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DT_CENTER,
            DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect,
            GetMonitorInfoW, HDC, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
            MonitorFromPoint, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::HiDpi::GetDpiForWindow,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GUITHREADINFO, GetClientRect,
            GetGUIThreadInfo, HMENU, HWND_TOPMOST, KillTimer, RegisterClassW, SW_HIDE,
            SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOREDRAW, SetTimer, SetWindowPos, ShowWindow,
            WM_DPICHANGED, WM_ERASEBKGND, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
    core::PCWSTR,
};

// ─── 定数 ────────────────────────────────────────────────────────────────────

const WIN_SIZE: i32 = 32;
const FONT_HEIGHT: i32 = 22;
const CARET_HEIGHT_ESTIMATE: i32 = 24;

/// フェードアウト開始までの待機時間 (ms)
const FADE_START_MS: u32 = 1500;
/// フェードアウト用タイマー間隔 (ms)  ― 未使用 (Layered Window 非使用のため hide で即消去)
const HIDE_TIMER_ID: usize = 100;

const COLOR_BG_LIGHT: COLORREF = COLORREF(0x00_FF_FF_FF);
const COLOR_FG_LIGHT: COLORREF = COLORREF(0x00_55_55_55);
const COLOR_BG_DARK: COLORREF = COLORREF(0x00_33_33_33);
const COLOR_FG_DARK: COLORREF = COLORREF(0x00_FF_FF_FF);

// ─── スレッドローカル状態 ──────────────────────────────────────────────────────

thread_local! {
    static TL_HWND: Cell<isize> = const { Cell::new(0) };
    static TL_TEXT: Cell<&'static str> = const { Cell::new("あ") };
    static TL_LIGHT: Cell<bool> = const { Cell::new(false) };
}

/// 表示中フラグ（キー入力で即非表示にするためアトミック）
static VISIBLE: AtomicBool = AtomicBool::new(false);

// ─── ウィンドウクラス ─────────────────────────────────────────────────────────

static CLASS_NAME_UTF16: &[u16] = &[
    b'R' as u16,
    b'a' as u16,
    b'k' as u16,
    b'u' as u16,
    b'k' as u16,
    b'a' as u16,
    b'n' as u16,
    b'M' as u16,
    b'o' as u16,
    b'd' as u16,
    b'e' as u16,
    0,
];

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

unsafe fn ensure_class_registered() {
    if CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }
    let hmod = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hmod.into(),
        lpszClassName: PCWSTR(CLASS_NAME_UTF16.as_ptr()),
        ..Default::default()
    };
    RegisterClassW(&wc);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_SETTINGCHANGE
        | windows::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED
        | windows::Win32::UI::WindowsAndMessaging::WM_SYSCOLORCHANGE => {
            TL_LIGHT.with(|l| l.set(super::theme::is_light(super::theme::Surface::App)));
            let _ = InvalidateRect(hwnd, None, BOOL(0));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !hdc.is_invalid() {
                draw(hwnd, hdc);
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // The suggested rectangle is in this window's coordinate space.
            if lparam.0 != 0 {
                let rect = &*(lparam.0 as *const RECT);
                let size = scaled(WIN_SIZE, (wparam.0 & 0xffff) as u32);
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    rect.left,
                    rect.top,
                    size,
                    size,
                    SWP_NOACTIVATE,
                );
                let _ = InvalidateRect(hwnd, None, BOOL(0));
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            if wparam.0 == HIDE_TIMER_ID {
                hide();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ─── 描画 ─────────────────────────────────────────────────────────────────────

unsafe fn draw(hwnd: HWND, hdc: HDC) {
    let text = TL_TEXT.with(|t| t.get());
    let light = TL_LIGHT.with(|l| l.get());

    let (bg, fg) = if light {
        (COLOR_BG_LIGHT, COLOR_FG_LIGHT)
    } else {
        (COLOR_BG_DARK, COLOR_FG_DARK)
    };

    let bg_brush = CreateSolidBrush(bg);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    FillRect(hdc, &rc, bg_brush);
    let _ = DeleteObject(bg_brush);

    let face: Vec<u16> = "Yu Gothic UI\0".encode_utf16().collect();
    let font = CreateFontW(
        scaled(FONT_HEIGHT, GetDpiForWindow(hwnd)),
        0,
        0,
        0,
        700,
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
        PCWSTR(face.as_ptr()),
    );
    let old_font = SelectObject(hdc, font);
    SetBkMode(hdc, BACKGROUND_MODE(1)); // TRANSPARENT
    SetTextColor(hdc, fg);

    let mut wbuf: Vec<u16> = text.encode_utf16().collect();
    // Measure the actual glyph: Latin A is narrower than あ/ア.
    DrawTextW(
        hdc,
        &mut wbuf,
        &mut rc,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(font);
}

// ─── 公開 API ────────────────────────────────────────────────────────────────

/// OS からキャレット位置を取得する（スクリーン座標）。
/// TSF の GetTextExt で取得できなかった場合の二次フォールバック。
/// 取得できない場合は None を返し、インジケーターは表示しない（mozc 準拠）。
fn get_caret_screen_pos() -> Option<(i32, i32)> {
    unsafe {
        // GetGUIThreadInfo でキャレット位置を取得
        // TSF はアプリの UI スレッド上で動くので、現在のスレッド ID を指定する。
        let tid = windows::Win32::System::Threading::GetCurrentThreadId();
        let mut gti = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(tid, &mut gti).is_ok() {
            let hwnd_caret = gti.hwndCaret;
            if !hwnd_caret.0.is_null() {
                let mut pt = POINT {
                    x: gti.rcCaret.left,
                    y: gti.rcCaret.bottom,
                };
                if ClientToScreen(hwnd_caret, &mut pt).as_bool() {
                    return Some((pt.x, pt.y));
                }
            }
        }

        // マウスカーソルへのフォールバックは行わない（mozc 準拠）。
        // 位置が特定できない場合はインジケーターを表示しない。
        None
    }
}

/// モードインジケーターを表示する。
///
/// `mode_char`: 表示文字（"あ", "ア", "A"）
/// `x`, `y`: キャレット位置（スクリーン座標、y はキャレット下端）
pub fn show(mode_char: &'static str, x: i32, y: i32) {
    // キャレット位置が未設定 (0,0) の場合は OS API で取得する
    let (x, y) = if x == 0 && y == 0 {
        match get_caret_screen_pos() {
            Some((cx, cy)) => (cx, cy),
            None => {
                tracing::debug!("mode_indicator: no caret position available");
                return;
            }
        }
    } else {
        (x, y)
    };

    let light = super::theme::is_light(super::theme::Surface::App);
    TL_TEXT.with(|t| t.set(mode_char));
    TL_LIGHT.with(|l| l.set(light));

    unsafe {
        let mut hwnd = TL_HWND.with(|h| HWND(h.get() as *mut _));
        if !is_valid(hwnd) {
            ensure_class_registered();
            let hmod = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            hwnd = match CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                PCWSTR(CLASS_NAME_UTF16.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                x,
                y,
                1,
                1,
                HWND::default(),
                HMENU::default(),
                hmod,
                None,
            ) {
                Ok(hwnd) if is_valid(hwnd) => hwnd,
                _ => {
                    tracing::warn!("mode_indicator::create: failed");
                    return;
                }
            };
            TL_HWND.with(|h| h.set(hwnd.0 as isize));
        }
        // Move before querying DPI, including when reusing a window on another monitor.
        // Use the HWND's DPI so unaware hosts do not get scaled twice by Windows.
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            1,
            1,
            SWP_NOACTIVATE | SWP_NOREDRAW,
        );
        let dpi = GetDpiForWindow(hwnd);
        let size = scaled(WIN_SIZE, dpi);
        let (win_x, win_y) = calc_window_position(x, y, dpi);
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, win_x, win_y, size, size, SWP_NOACTIVATE);
        let _ = InvalidateRect(hwnd, None, BOOL(0));
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = KillTimer(hwnd, HIDE_TIMER_ID);
        let _ = SetTimer(hwnd, HIDE_TIMER_ID, FADE_START_MS, None);
    }
    VISIBLE.store(true, Ordering::Release);
}

/// モードインジケーターを非表示にする。
pub fn hide() {
    if !VISIBLE.load(Ordering::Acquire) {
        return;
    }
    let hwnd = TL_HWND.with(|h| HWND(h.get() as *mut _));
    if is_valid(hwnd) {
        unsafe {
            let _ = KillTimer(hwnd, HIDE_TIMER_ID);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    VISIBLE.store(false, Ordering::Release);
}

/// モードインジケーターを破棄する（Deactivate 時）。
pub fn destroy() {
    let hwnd = TL_HWND.with(|h| HWND(h.get() as *mut _));
    if is_valid(hwnd) {
        unsafe {
            let _ = KillTimer(hwnd, HIDE_TIMER_ID);
            let _ = DestroyWindow(hwnd);
        }
    }
    TL_HWND.with(|h| h.set(0));
    VISIBLE.store(false, Ordering::Release);
}

/// 表示中かどうか
#[allow(dead_code)]
pub fn is_visible() -> bool {
    VISIBLE.load(Ordering::Acquire)
}

// ─── ヘルパー ─────────────────────────────────────────────────────────────────

fn is_valid(hwnd: HWND) -> bool {
    !hwnd.0.is_null()
}

fn scaled(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32
}

fn position_in_work_area(x: i32, caret_bottom: i32, dpi: u32, work: RECT) -> (i32, i32) {
    let size = scaled(WIN_SIZE, dpi);
    let y = if caret_bottom + size > work.bottom {
        caret_bottom - scaled(CARET_HEIGHT_ESTIMATE + 4, dpi) - size
    } else {
        caret_bottom
    };
    (
        x.clamp(work.left, (work.right - size).max(work.left)),
        y.clamp(work.top, (work.bottom - size).max(work.top)),
    )
}

unsafe fn calc_window_position(x: i32, caret_bottom: i32, dpi: u32) -> (i32, i32) {
    let hmon = MonitorFromPoint(POINT { x, y: caret_bottom }, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut mi).as_bool() {
        position_in_work_area(x, caret_bottom, dpi, mi.rcWork)
    } else {
        (x, caret_bottom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a Windows desktop; run explicitly"]
    fn mode_indicator_dpi_desktop_regression() {
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            DPI_AWARENESS_CONTEXT_SYSTEM_AWARE, DPI_AWARENESS_CONTEXT_UNAWARE,
            SetThreadDpiAwarenessContext,
        };
        struct Restore(DPI_AWARENESS_CONTEXT);
        impl Drop for Restore {
            fn drop(&mut self) {
                destroy();
                unsafe {
                    SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
        for context in [
            DPI_AWARENESS_CONTEXT_UNAWARE,
            DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        ] {
            unsafe {
                let old = SetThreadDpiAwarenessContext(context);
                assert!(!old.0.is_null());
                let _restore = Restore(old);
                for text in ["A", "あ", "ア"] {
                    show(text, 100, 100);
                    let hwnd = TL_HWND.with(|h| HWND(h.get() as *mut _));
                    assert!(is_valid(hwnd));
                    let dpi = GetDpiForWindow(hwnd);
                    assert!(dpi >= 96);
                    let mut rect = RECT::default();
                    GetClientRect(hwnd, &mut rect).unwrap();
                    assert_eq!(rect.right - rect.left, scaled(WIN_SIZE, dpi));
                    assert_eq!(rect.bottom - rect.top, scaled(WIN_SIZE, dpi));
                    println!(
                        "mode={text} dpi={dpi} size={} font={}",
                        rect.right,
                        scaled(FONT_HEIGHT, dpi)
                    );
                    hide();
                }
            }
        }
    }

    #[test]
    fn scales_indicator_and_keeps_it_inside_work_area() {
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        for (dpi, size, font) in [(96, 32, 22), (144, 48, 33), (192, 64, 44)] {
            assert_eq!(scaled(WIN_SIZE, dpi), size);
            assert_eq!(scaled(FONT_HEIGHT, dpi), font);
            assert_eq!(position_in_work_area(-500, 300, dpi, work), (-500, 300));
            let (x, y) = position_in_work_area(-1, 1070, dpi, work);
            assert_eq!(x + size, work.right);
            assert!(y + size < 1070);
            assert!(y >= work.top);
        }
        assert_eq!(scaled(WIN_SIZE, 0), 32);
    }
}
