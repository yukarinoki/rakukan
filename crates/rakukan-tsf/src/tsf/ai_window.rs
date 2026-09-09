//! Non-activating, opaque inline preview. The document is untouched until Enter.
use std::{cell::RefCell, time::Instant};
use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::w,
};

#[derive(Clone, Default)]
pub(crate) struct Geometry {
    pub caret: RECT,
    pub lines: Vec<RECT>,
    pub viewport: RECT,
}
#[derive(Clone)]
pub(crate) struct View {
    pub geometry: Geometry,
    pub text: String,
    pub replace: bool,
    pub busy: bool,
    pub editing: bool,
}
struct Surface {
    hwnd: HWND,
    text: String,
    text_rect: RECT,
    font_height: i32,
    busy: bool,
    editing: bool,
    began: Instant,
}
thread_local! { static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) }; }

fn color(seconds: f32, busy: bool) -> COLORREF {
    // A gentle 1.6 second breath; no abrupt white flashes.
    let amount = if busy {
        (0.5 - 0.5 * (seconds * std::f32::consts::TAU / 1.6).cos()) * 0.72
    } else {
        0.0
    };
    let mix = |base: f32| (base + (255.0 - base) * amount) as u32;
    COLORREF(mix(59.0) | (mix(130.0) << 8) | (mix(246.0) << 16))
}
unsafe fn font(height: i32) -> HFONT {
    CreateFontW(
        height,
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        1,
        0,
        0,
        5,
        0,
        w!("Yu Gothic UI"),
    )
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            draw(hwnd, dc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe fn draw(hwnd: HWND, dc: HDC) {
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let snapshot = SURFACE.with(|s| {
        s.borrow().as_ref().map(|s| {
            (
                s.text.clone(),
                s.text_rect,
                s.font_height,
                color(s.began.elapsed().as_secs_f32(), s.busy),
                s.editing,
            )
        })
    });
    if let Some((text, mut rect, height, bg, editing)) = snapshot {
        let brush = CreateSolidBrush(bg);
        FillRect(dc, &client, brush);
        let _ = DeleteObject(brush);
        let font = font(height);
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00301B0B));
        let mut text: Vec<u16> = text.encode_utf16().collect();
        if editing {
            text.push('│' as u16);
        }
        DrawTextW(dc, &mut text, &mut rect, DT_WORDBREAK | DT_NOPREFIX);
        SelectObject(dc, old);
        let _ = DeleteObject(font);
    }
}
pub(crate) fn hide() {
    // Taking ownership before DestroyWindow is essential: it can reenter wndproc.
    if let Some(s) = SURFACE.with(|s| s.borrow_mut().take()) {
        unsafe {
            let _ = DestroyWindow(s.hwnd);
        }
    }
}
pub(crate) fn show(view: View) {
    show_impl(view, true);
}
fn show_impl(view: View, visible: bool) {
    unsafe {
        let mut hwnd = SURFACE.with(|s| s.borrow().as_ref().map(|s| s.hwnd));
        if hwnd.is_none() {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("RakukanAiPuddle"),
                ..Default::default()
            };
            RegisterClassW(&class);
            hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                class.lpszClassName,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                instance,
                None,
            )
            .ok();
        }
        let Some(hwnd) = hwnd else { return };
        let g = &view.geometry;
        let anchor = if view.replace {
            g.lines.first().copied().unwrap_or(g.caret)
        } else {
            g.caret
        };
        // TSF rectangles already use the host's coordinate space: do not apply DPI twice.
        let height = (anchor.bottom - anchor.top).clamp(12, 120);
        let padding = (height / 4).max(3);
        let width = (g.viewport.right - anchor.left - padding)
            .min(height * 24)
            .max(height * 2);
        let font_height = (height * 4 / 5).max(10);
        let dc = GetDC(hwnd);
        let f = font(font_height);
        let old = SelectObject(dc, f);
        let mut buffer: Vec<u16> = view.text.encode_utf16().collect();
        if view.editing {
            buffer.push('│' as u16);
        }
        let mut measured = RECT {
            right: width,
            ..Default::default()
        };
        DrawTextW(
            dc,
            &mut buffer,
            &mut measured,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        let _ = DeleteObject(f);
        ReleaseDC(hwnd, dc);
        let mut pool = RECT {
            left: anchor.left,
            top: anchor.top,
            right: anchor.left + measured.right.max(height * 5).min(width) + padding * 2,
            bottom: anchor.top + measured.bottom.max(height) + padding * 2,
        };
        // Keep the floating input visible near screen edges, without moving source masks.
        let monitor = MonitorFromPoint(
            POINT {
                x: anchor.left,
                y: anchor.top,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let dx = (pool.right - info.rcWork.right).max(0);
            pool.left -= dx;
            pool.right -= dx;
            let dy = (pool.bottom - info.rcWork.bottom).max(0);
            pool.top -= dy;
            pool.bottom -= dy;
        }
        let mut masks = if view.replace {
            g.lines
                .iter()
                .map(|r| {
                    let edge = (padding / 2).max(2);
                    RECT {
                        left: r.left - edge,
                        top: r.top - edge,
                        right: r.right + edge,
                        bottom: r.bottom + edge,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        masks.push(pool);
        let bounds = masks.iter().fold(pool, |a, b| RECT {
            left: a.left.min(b.left),
            top: a.top.min(b.top),
            right: a.right.max(b.right),
            bottom: a.bottom.max(b.bottom),
        });
        let region = CreateRectRgn(0, 0, 0, 0);
        for (index, r) in masks.iter().enumerate() {
            // Round outside the source bounds, keeping every original glyph opaque.
            let rounding = if index + 1 == masks.len() {
                padding * 3
            } else {
                padding.max(4)
            };
            let part = CreateRoundRectRgn(
                r.left - bounds.left,
                r.top - bounds.top,
                r.right - bounds.left + 1,
                r.bottom - bounds.top + 1,
                rounding,
                rounding,
            );
            CombineRgn(region, region, part, RGN_OR);
            let _ = DeleteObject(part);
        }
        let text_rect = RECT {
            left: pool.left - bounds.left + padding,
            top: pool.top - bounds.top + padding,
            right: pool.right - bounds.left - padding,
            bottom: pool.bottom - bounds.top - padding,
        };
        SURFACE.with(|s| {
            let mut s = s.borrow_mut();
            let began = s
                .as_ref()
                .filter(|s| s.busy == view.busy)
                .map(|s| s.began)
                .unwrap_or_else(Instant::now);
            *s = Some(Surface {
                hwnd,
                text: view.text,
                text_rect,
                font_height,
                busy: view.busy,
                editing: view.editing,
                began,
            });
        });
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left + 1,
            bounds.bottom - bounds.top + 1,
            SWP_NOACTIVATE,
        );
        if SetWindowRgn(hwnd, region, BOOL(1)) == 0 {
            let _ = DeleteObject(region);
        }
        if visible {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let _ = InvalidateRect(hwnd, None, BOOL(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a Windows desktop; creates only hidden windows"]
    fn inline_masks_and_dpi_desktop_regression() {
        unsafe {
            let foreground = GetForegroundWindow();
            for scale in [1, 2, 3] {
                let h = 20 * scale;
                let original = RECT {
                    left: 100,
                    top: 100,
                    right: 100 + 200 * scale,
                    bottom: 100 + h,
                };
                let second = RECT {
                    left: 100,
                    top: 100 + h * 3,
                    right: 100 + 150 * scale,
                    bottom: 100 + h * 4,
                };
                let geometry = Geometry {
                    caret: RECT {
                        left: second.right,
                        right: second.right + 1,
                        top: second.top,
                        bottom: second.bottom,
                    },
                    lines: vec![original, second],
                    viewport: RECT {
                        left: 0,
                        top: 0,
                        right: 1800,
                        bottom: 1000,
                    },
                };
                for replace in [false, true] {
                    show_impl(
                        View {
                            geometry: geometry.clone(),
                            text: "明日は晴れるでしょう。".into(),
                            replace,
                            busy: false,
                            editing: !replace,
                        },
                        false,
                    );
                    let hwnd = SURFACE.with(|s| s.borrow().as_ref().unwrap().hwnd);
                    assert!(!IsWindowVisible(hwnd).as_bool());
                    assert_eq!(foreground, GetForegroundWindow());
                    let mut bounds = RECT::default();
                    GetWindowRect(hwnd, &mut bounds).unwrap();
                    let region = CreateRectRgn(0, 0, 0, 0);
                    assert_ne!(GetWindowRgn(hwnd, region).0, ERROR);
                    for source in [original, second] {
                        for (x, y) in [
                            (source.left, source.top),
                            (source.right - 1, source.bottom - 1),
                        ] {
                            assert_eq!(
                                PtInRegion(region, x - bounds.left, y - bounds.top).as_bool(),
                                replace
                            );
                        }
                    }
                    // A gap outside the candidate itself must not cover unrelated document text.
                    assert!(
                        !PtInRegion(
                            region,
                            original.right - bounds.left - 1,
                            (original.bottom + second.top) / 2 - bounds.top
                        )
                        .as_bool()
                    );
                    if replace {
                        let width = bounds.right - bounds.left;
                        let height = bounds.bottom - bounds.top;
                        let dc = CreateCompatibleDC(None);
                        let info = BITMAPINFO {
                            bmiHeader: BITMAPINFOHEADER {
                                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                                biWidth: width,
                                biHeight: -height,
                                biPlanes: 1,
                                biBitCount: 32,
                                ..Default::default()
                            },
                            ..Default::default()
                        };
                        let mut bits = std::ptr::null_mut();
                        let bitmap =
                            CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0)
                                .unwrap();
                        let old = SelectObject(dc, bitmap);
                        let canvas = RECT {
                            right: width,
                            bottom: height,
                            ..Default::default()
                        };
                        let white = CreateSolidBrush(COLORREF(0xFFFFFF));
                        FillRect(dc, &canvas, white);
                        let _ = DeleteObject(white);
                        SelectClipRgn(dc, region);
                        draw(hwnd, dc);
                        assert_eq!(
                            GetPixel(
                                dc,
                                second.left - bounds.left + 2,
                                second.top - bounds.top + 2
                            ),
                            color(0.0, false)
                        );
                        if let Ok(directory) = std::env::var("RAKUKAN_AI_PREVIEW") {
                            std::fs::create_dir_all(&directory).unwrap();
                            let pixels = std::slice::from_raw_parts(
                                bits as *const u8,
                                (width * height * 4) as usize,
                            );
                            let mut bmp = Vec::new();
                            bmp.extend_from_slice(b"BM");
                            bmp.extend_from_slice(&(54u32 + pixels.len() as u32).to_le_bytes());
                            bmp.extend_from_slice(&[0; 4]);
                            bmp.extend_from_slice(&54u32.to_le_bytes());
                            bmp.extend_from_slice(&40u32.to_le_bytes());
                            bmp.extend_from_slice(&width.to_le_bytes());
                            bmp.extend_from_slice(&(-height).to_le_bytes());
                            bmp.extend_from_slice(&1u16.to_le_bytes());
                            bmp.extend_from_slice(&32u16.to_le_bytes());
                            bmp.extend_from_slice(&[0; 24]);
                            bmp.extend_from_slice(pixels);
                            std::fs::write(
                                std::path::Path::new(&directory)
                                    .join(format!("puddle-{scale}.bmp")),
                                bmp,
                            )
                            .unwrap();
                        }
                        SelectObject(dc, old);
                        let _ = DeleteObject(bitmap);
                        let _ = DeleteDC(dc);
                    }
                    let _ = DeleteObject(region);
                    hide();
                    assert!(!IsWindow(hwnd).as_bool());
                }
            }
        }
    }
    #[test]
    fn pulse_returns_to_blue_and_remains_blue_without_work() {
        assert_eq!(color(0.0, true), color(1.6, true));
        assert_ne!(color(0.0, true), color(0.8, true));
        assert_eq!(color(0.0, false), color(0.8, false));
    }
}
