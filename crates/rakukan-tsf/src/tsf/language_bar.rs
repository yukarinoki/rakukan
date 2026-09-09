//! 言語バー（システムトレイ）インジケーター

use windows::Win32::{
    Foundation::HINSTANCE,
    UI::TextServices::{
        GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, GUID_LBI_INPUTMODE, ITfCompartmentMgr,
        ITfLangBarItemButton, ITfLangBarItemMgr, ITfThreadMgr, TF_LANGBARITEMINFO,
        TF_LBI_STYLE_BTN_BUTTON, TF_LBI_STYLE_SHOWNINTRAY,
    },
    UI::WindowsAndMessaging::{HICON, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED, LoadImageW},
};
use windows_core::Interface;

use crate::globals::{DllModule, GUID_TEXT_SERVICE};

pub const LANGBAR_SINK_COOKIE: u32 = 0xACA_CACA;

pub fn make_langbar_info() -> TF_LANGBARITEMINFO {
    let mut info = TF_LANGBARITEMINFO {
        clsidService: GUID_TEXT_SERVICE,
        // GUID_LBI_INPUTMODE: Windows 標準の入力モードボタン。
        // キーボードレイアウト表示の隣に統合される（mozc と同じ方式）。
        // クリック時のポップアップメニューは OnClick 側で明示的に表示する。
        guidItem: GUID_LBI_INPUTMODE,
        dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
        ulSort: 0,
        szDescription: [0; 32],
    };
    let desc: Vec<u16> = "rakukan".encode_utf16().collect();
    for (i, &c) in desc.iter().take(31).enumerate() {
        info.szDescription[i] = c;
    }
    info
}

pub unsafe fn langbar_add(
    thread_mgr: &ITfThreadMgr,
    item: &ITfLangBarItemButton,
) -> anyhow::Result<()> {
    thread_mgr
        .cast::<ITfLangBarItemMgr>()
        .map_err(|e| anyhow::anyhow!("cast ITfLangBarItemMgr: {e}"))?
        .AddItem(item)
        .map_err(|e| anyhow::anyhow!("AddItem: {e}"))?;
    Ok(())
}

pub unsafe fn langbar_remove(
    thread_mgr: &ITfThreadMgr,
    item: &ITfLangBarItemButton,
) -> anyhow::Result<()> {
    let _ = thread_mgr
        .cast::<ITfLangBarItemMgr>()
        .map_err(|e| anyhow::anyhow!("cast ITfLangBarItemMgr: {e}"))?
        .RemoveItem(item);
    Ok(())
}

// ─── コンパートメント操作 ────────────────────────────────────────────────────
// windows_core::VARIANT::from(i32) を使う（内部で VT_I4 を正しく設定する）

pub unsafe fn set_open_close(
    thread_mgr: &ITfThreadMgr,
    tid: u32,
    open: bool,
) -> anyhow::Result<()> {
    let mgr = thread_mgr
        .cast::<ITfCompartmentMgr>()
        .map_err(|e| anyhow::anyhow!("ITfCompartmentMgr cast: {e}"))?;
    let comp = mgr
        .GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
        .map_err(|e| anyhow::anyhow!("GetCompartment: {e}"))?;
    // windows_core::VARIANT::from(i32) は内部で VT_I4 を正しく設定する
    let var = windows_core::VARIANT::from(if open { 1i32 } else { 0i32 });
    comp.SetValue(tid, &var)
        .map_err(|e| anyhow::anyhow!("SetValue hr={e}"))?;
    Ok(())
}

#[allow(dead_code)]
pub unsafe fn toggle_open_close(thread_mgr: &ITfThreadMgr, tid: u32) -> anyhow::Result<()> {
    // 現在値を取得してトグル
    let mgr = thread_mgr
        .cast::<ITfCompartmentMgr>()
        .map_err(|e| anyhow::anyhow!("ITfCompartmentMgr cast: {e}"))?;
    let comp = mgr
        .GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
        .map_err(|e| anyhow::anyhow!("GetCompartment: {e}"))?;
    let current = comp
        .GetValue()
        .ok()
        .and_then(|v| i32::try_from(&v).ok())
        .unwrap_or(1);
    let var = windows_core::VARIANT::from(if current == 0 { 1i32 } else { 0i32 });
    comp.SetValue(tid, &var)
        .map_err(|e| anyhow::anyhow!("SetValue hr={e}"))?;
    Ok(())
}

pub fn get_open_close(thread_mgr: &ITfThreadMgr) -> bool {
    unsafe {
        let Ok(mgr) = thread_mgr.cast::<ITfCompartmentMgr>() else {
            return true;
        };
        let Ok(comp) = mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) else {
            return true;
        };
        comp.GetValue()
            .ok()
            .and_then(|v| i32::try_from(&v).ok())
            .map(|n| n != 0)
            .unwrap_or(true)
    }
}

// ─── アイコン ────────────────────────────────────────────────────────────────

pub unsafe fn load_tray_icon() -> windows::core::Result<HICON> {
    let hinst: HINSTANCE = DllModule::get()
        .ok()
        .and_then(|m| m.hinst)
        .map(|h| unsafe { std::mem::transmute(h) })
        .unwrap_or_default();
    let handle = LoadImageW(
        hinst,
        windows::core::PCWSTR(std::ptr::dangling_mut::<u16>()),
        IMAGE_ICON,
        0,
        0,
        LR_DEFAULTSIZE | LR_SHARED,
    )?;
    Ok(HICON(handle.0))
}

// ─── モード別アイコン動的生成 ────────────────────────────────────────────────

use std::ptr::null_mut;
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

/// テーマ (ライト/ダーク) を検出する。判定不能時はダークと見なす。
pub fn is_light_mode() -> bool {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_READ, REG_DWORD, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    unsafe {
        let mut hkey = Default::default();
        let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return false;
        }
        let val_name: Vec<u16> = "SystemUsesLightTheme"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut data = 0u32;
        let mut size = 4u32;
        let mut kind = REG_DWORD;
        let result = if RegQueryValueExW(
            hkey,
            windows::core::PCWSTR(val_name.as_ptr()),
            None,
            Some(&mut kind),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        )
        .is_ok()
        {
            data != 0
        } else {
            false
        };
        let _ = RegCloseKey(hkey);
        result
    }
}

/// Render at the taskbar's actual small-icon resolution, even inside a DPI-unaware app.
fn mode_icon_size() -> i32 {
    use windows::Win32::UI::{
        HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi},
        WindowsAndMessaging::{FindWindowW, SM_CXSMICON},
    };
    unsafe {
        let dpi = FindWindowW(windows::core::w!("Shell_TrayWnd"), None)
            .ok()
            .map(|hwnd| GetDpiForWindow(hwnd))
            .filter(|dpi| *dpi != 0)
            .unwrap_or_else(|| GetDpiForSystem().max(96));
        GetSystemMetricsForDpi(SM_CXSMICON, dpi).clamp(16, 128)
    }
}

fn bitmap_info(size: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Draw a monochrome coverage mask at 4x resolution, then form premultiplied BGRA.
/// GDI's alpha bytes are undefined; coverage comes from RGB, never from GDI alpha.
fn mode_icon_pixels(text: &str, size: i32, light: bool) -> windows::core::Result<Vec<u8>> {
    const SCALE: i32 = 4;
    let high = size * SCALE;
    unsafe {
        let dc = CreateCompatibleDC(HDC::default());
        if dc.is_invalid() {
            return Err(windows::core::Error::from_win32());
        }
        let mut bits = null_mut();
        let bitmap =
            match CreateDIBSection(dc, &bitmap_info(high), DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(error);
                }
            };
        let old_bitmap = SelectObject(dc, bitmap);
        let font = CreateFontW(
            -(high * 14 / 16),
            0,
            0,
            0,
            500,
            0,
            0,
            0,
            1,
            0,
            0,
            windows::Win32::Graphics::Gdi::NONANTIALIASED_QUALITY.0 as u32,
            0,
            windows::core::w!("Yu Gothic UI"),
        );
        let old_font = SelectObject(dc, font);
        let result = (|| {
            if bits.is_null() || font.is_invalid() {
                return Err(windows::core::Error::from_win32());
            }
            std::ptr::write_bytes(bits.cast::<u8>(), 0, (high * high * 4) as usize);
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, windows::Win32::Foundation::COLORREF(0x00ffffff));
            let mut rect = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: high,
                bottom: high,
            };
            let mut text: Vec<u16> = text.encode_utf16().collect();
            if DrawTextW(
                dc,
                &mut text,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            ) == 0
            {
                return Err(windows::core::Error::from_win32());
            }
            // Complete any batched GDI writes before reading the DIB memory.
            let _ = windows::Win32::Graphics::Gdi::GdiFlush();
            let source = std::slice::from_raw_parts(bits.cast::<u8>(), (high * high * 4) as usize);
            let mut pixels = vec![0u8; (size * size * 4) as usize];
            for y in 0..size {
                for x in 0..size {
                    let mut coverage = 0u32;
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            coverage += u32::from(
                                source[(((y * SCALE + dy) * high + x * SCALE + dx) * 4) as usize],
                            );
                        }
                    }
                    let alpha = ((coverage + 8) / 16) as u8;
                    let offset = ((y * size + x) * 4) as usize;
                    let color = if light { 0 } else { alpha };
                    pixels[offset..offset + 4].copy_from_slice(&[color, color, color, alpha]);
                }
            }
            Ok(pixels)
        })();
        let _ = SelectObject(dc, old_font);
        let _ = DeleteObject(font);
        let _ = SelectObject(dc, old_bitmap);
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(dc);
        result
    }
}

/// The caller owns the returned HICON, as required by ITfLangBarItemButton::GetIcon.
pub fn create_mode_icon(text: &str) -> windows::core::Result<HICON> {
    let size = mode_icon_size();
    let pixels = mode_icon_pixels(text, size, is_light_mode())?;
    unsafe {
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(
            HDC::default(),
            &bitmap_info(size),
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
        // Legacy AND mask: 1 = transparent. Rows are WORD aligned, including 24px icons.
        let stride = ((size + 15) / 16 * 2) as usize;
        let mut mask_bits = vec![0xffu8; stride * size as usize];
        for y in 0..size as usize {
            for x in 0..size as usize {
                if pixels[(y * size as usize + x) * 4 + 3] != 0 {
                    mask_bits[y * stride + x / 8] &= !(0x80 >> (x % 8));
                }
            }
        }
        let mask = windows::Win32::Graphics::Gdi::CreateBitmap(
            size,
            size,
            1,
            1,
            Some(mask_bits.as_ptr().cast()),
        );
        let result = if mask.is_invalid() {
            Err(windows::core::Error::from_win32())
        } else {
            CreateIconIndirect(&ICONINFO {
                fIcon: true.into(),
                hbmMask: mask,
                hbmColor: bitmap,
                ..Default::default()
            })
        };
        let _ = DeleteObject(mask);
        let _ = DeleteObject(bitmap);
        result
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;
    #[test]
    fn transparent_icons_have_smooth_premultiplied_edges_at_all_scales() {
        for size in [16, 20, 24, 32, 48, 64] {
            for text in ["A", "あ", "ア", "ー"] {
                for light in [false, true] {
                    let pixels = mode_icon_pixels(text, size, light).unwrap();
                    assert_eq!(pixels.len(), (size * size * 4) as usize);
                    assert_eq!(pixels[3], 0, "transparent corner: {text} {size}");
                    assert!(
                        pixels.chunks_exact(4).any(|p| p[3] == 255),
                        "visible glyph: {text} {size}"
                    );
                    assert!(
                        pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255),
                        "antialiasing: {text} {size}"
                    );
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
                    );
                    if let Some(directory) = std::env::var_os("RAKUKAN_ICON_PREVIEW") {
                        std::fs::create_dir_all(&directory).unwrap();
                        std::fs::write(
                            std::path::PathBuf::from(directory)
                                .join(format!("{text}-{size}-{light}.bgra")),
                            &pixels,
                        )
                        .unwrap();
                    }
                }
            }
        }
        let icon = create_mode_icon("あ").unwrap();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon).unwrap();
        }
    }
}
