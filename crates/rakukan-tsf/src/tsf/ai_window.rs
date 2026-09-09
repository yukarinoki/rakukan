//! Non-activating translucent instruction puddle and inline preview. The document is untouched until Enter.
use std::{cell::RefCell, time::Instant};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
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
    pool: RECT,
}
thread_local! { static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) }; }

// Three logical pixels: keep the puddle close without touching the caret.
fn caret_gap(dpi: u32) -> i32 {
    ((3 * dpi.max(96) + 48) / 96) as i32
}
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
// DrawTextW may dereference lpchText even when cchText is zero. An empty Rust
// Vec has a dangling (usually 0x2) pointer, not a readable empty UTF-16 string.
unsafe fn draw_text(dc: HDC, text: &mut [u16], rect: &mut RECT, flags: DRAW_TEXT_FORMAT) -> i32 {
    if text.is_empty() {
        if flags.contains(DT_CALCRECT) {
            rect.right = rect.left;
            rect.bottom = rect.top;
        }
        return 0;
    }
    unsafe { DrawTextW(dc, text, rect, flags) }
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
        draw_text(dc, &mut text, &mut rect, DT_WORDBREAK | DT_NOPREFIX);
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
                WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TRANSPARENT
                    | WS_EX_LAYERED,
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
        let mut anchor = if view.replace {
            g.lines.first().copied().unwrap_or(g.caret)
        } else {
            g.caret
        };
        if !view.replace {
            anchor.left = g.caret.right + caret_gap(GetDpiForWindow(hwnd));
        }
        let preview = !view.editing && !view.busy;
        // TSF rectangles already use the host's coordinate space: do not apply DPI twice.
        let height = (anchor.bottom - anchor.top).clamp(12, 120);
        let padding = (height / 6).max(2);
        let width = (g.viewport.right - anchor.left - padding)
            .min(height * 24)
            .max(height * 2);
        let font_height = if preview {
            height
        } else {
            (height * 2 / 3).max(8)
        };
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
        draw_text(
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
            right: anchor.left + measured.right.max(height / 3).min(width) + padding * 2,
            bottom: anchor.top + measured.bottom.max(height),
        };
        // Ease horizontal expansion while keeping the caret edge and line height fixed.
        // Hidden native tests use the final geometry, without animation timing.
        if visible
            && !preview
            && let Some(previous) = SURFACE.with(|s| s.borrow().as_ref().map(|s| s.pool))
            && previous.left == pool.left
            && previous.top == pool.top
            && previous.bottom == pool.bottom
        {
            let delta = pool.right - previous.right;
            if delta.abs() > 2 {
                pool.right = previous.right + delta / 2;
            }
        }
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
                height
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
            top: pool.top - bounds.top + (height - font_height).max(0) / 2,
            right: pool.right - bounds.left - padding,
            bottom: pool.bottom - bounds.top,
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
                pool,
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
        // Keep replacement masks opaque so original glyphs do not bleed through.
        // The instruction water and append preview remain translucent.
        let _ = SetLayeredWindowAttributes(
            hwnd,
            COLORREF(0),
            if view.replace { 255 } else { 185 },
            LWA_ALPHA,
        );
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
    fn instruction_grows_right_at_text_height_desktop_regression() {
        for height in [20, 30, 40] {
            let geometry = Geometry {
                caret: RECT {
                    left: 100,
                    right: 102,
                    top: 100,
                    bottom: 100 + height,
                },
                lines: vec![],
                viewport: RECT {
                    left: 0,
                    top: 0,
                    right: 1000,
                    bottom: 800,
                },
            };
            let mut previous_width = 0;
            for text in ["", "つ", "つづ", "つづき"] {
                show_impl(
                    View {
                        geometry: geometry.clone(),
                        text: text.into(),
                        busy: false,
                        editing: true,
                        replace: false,
                    },
                    false,
                );
                let (hwnd, font_height) = SURFACE.with(|s| {
                    let s = s.borrow();
                    let s = s.as_ref().unwrap();
                    (s.hwnd, s.font_height)
                });
                let mut rect = RECT::default();
                unsafe {
                    GetWindowRect(hwnd, &mut rect).unwrap();
                }
                assert_eq!(
                    rect.left,
                    geometry.caret.right + caret_gap(unsafe { GetDpiForWindow(hwnd) })
                );
                assert_eq!(rect.top, geometry.caret.top);
                assert_eq!(rect.bottom - rect.top, height + 1);
                let width = rect.right - rect.left;
                assert!(
                    width > previous_width,
                    "instruction={text} width={width} previous={previous_width}"
                );
                if text.is_empty() {
                    assert!(width <= height);
                }
                assert!(font_height < height);
                let mut alpha = 0u8;
                unsafe {
                    GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), None).unwrap();
                }
                assert_eq!(alpha, 185);
                previous_width = width;
            }
            show_impl(
                View {
                    geometry,
                    text: "候補".into(),
                    busy: false,
                    editing: false,
                    replace: false,
                },
                false,
            );
            assert_eq!(
                SURFACE.with(|s| s.borrow().as_ref().unwrap().font_height),
                height
            );
            hide();
        }
    }
    #[test]
    fn empty_text_never_enters_gdi_and_has_zero_measured_extent() {
        let mut rect = RECT {
            left: 10,
            top: 20,
            right: 400,
            bottom: 200,
        };
        unsafe {
            assert_eq!(
                draw_text(
                    HDC::default(),
                    &mut [],
                    &mut rect,
                    DT_CALCRECT | DT_WORDBREAK
                ),
                0
            );
            assert_eq!(rect.right, rect.left);
            assert_eq!(rect.bottom, rect.top);
            let before = rect;
            assert_eq!(
                draw_text(HDC::default(), &mut [], &mut rect, DT_WORDBREAK),
                0
            );
            assert_eq!(rect, before);
        }
    }
    #[test]
    #[ignore = "requires a Windows desktop; creates only hidden windows"]
    fn empty_initial_generation_and_empty_reply_desktop_regression() {
        let geometry = Geometry {
            caret: RECT {
                left: 100,
                top: 100,
                right: 101,
                bottom: 130,
            },
            lines: vec![RECT {
                left: 40,
                top: 100,
                right: 100,
                bottom: 130,
            }],
            viewport: RECT {
                left: 0,
                top: 0,
                right: 1000,
                bottom: 800,
            },
        };
        for (text, busy, editing, replace) in [
            ("", true, false, false), // entering AI: default generation, no instruction yet
            ("", false, false, false), // empty append response
            ("", false, false, true), // empty replacement response
            ("", false, true, false), // clear/backspace: instruction caret only
            ("えいご", true, false, false),
            ("English", false, false, true),
        ] {
            show_impl(
                View {
                    geometry: geometry.clone(),
                    text: text.into(),
                    busy,
                    editing,
                    replace,
                },
                false,
            );
            let hwnd = SURFACE.with(|s| s.borrow().as_ref().unwrap().hwnd);
            unsafe {
                assert!(!IsWindowVisible(hwnd).as_bool());
                let dc = GetDC(hwnd);
                assert!(!dc.is_invalid());
                draw(hwnd, dc);
                ReleaseDC(hwnd, dc);
            }
        }
        hide();
    }
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
