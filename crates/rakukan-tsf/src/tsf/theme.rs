//! Windows has separate app and taskbar themes. Never change the host process's theme.
use windows::Win32::Foundation::COLORREF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    App,
    Taskbar,
}

impl Surface {
    fn value_name(self) -> &'static str {
        match self {
            Self::App => "AppsUseLightTheme",
            Self::Taskbar => "SystemUsesLightTheme",
        }
    }
    fn default_light(self) -> bool {
        self == Self::App
    }
}

pub fn is_light(surface: Surface) -> bool {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, REG_DWORD, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::{PCWSTR, w};
    unsafe {
        let mut key = Default::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
        .is_err()
        {
            return surface.default_light();
        }
        let name: Vec<u16> = surface.value_name().encode_utf16().chain(Some(0)).collect();
        let mut data = 0u32;
        let mut size = 4;
        let mut kind = REG_DWORD;
        let valid = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
        .is_ok()
            && kind == REG_DWORD
            && size == 4;
        let _ = RegCloseKey(key);
        if valid {
            data != 0
        } else {
            surface.default_light()
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: COLORREF,
    pub foreground: COLORREF,
    pub selected_background: COLORREF,
    pub selected_foreground: COLORREF,
    pub secondary: COLORREF,
    pub pager_background: COLORREF,
    pub status_background: COLORREF,
}

pub fn palette(light: bool) -> Palette {
    if light {
        Palette {
            background: COLORREF(0xffffff),
            foreground: COLORREF(0x202020),
            selected_background: COLORREF(0xb44e20),
            selected_foreground: COLORREF(0xffffff),
            secondary: COLORREF(0x606060),
            pager_background: COLORREF(0xf0f0f0),
            status_background: COLORREF(0xf8f8f8),
        }
    } else {
        Palette {
            background: COLORREF(0x202020),
            foreground: COLORREF(0xf3f3f3),
            selected_background: COLORREF(0xffc480),
            selected_foreground: COLORREF(0x101010),
            secondary: COLORREF(0xb8b8b8),
            pager_background: COLORREF(0x292929),
            status_background: COLORREF(0x272727),
        }
    }
}

// A hidden top-level window receives theme broadcasts even before conversion.
// HWND_MESSAGE would not receive WM_SETTINGCHANGE broadcasts.
mod observer {
    use std::cell::{Cell, RefCell};
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                TextServices::{ITfLangBarItemSink, TF_LBI_ICON},
                WindowsAndMessaging::*,
            },
        },
        core::w,
    };
    thread_local! {
        static WINDOW: Cell<isize> = const { Cell::new(0) };
        static SINK: RefCell<Option<ITfLangBarItemSink>> = const { RefCell::new(None) };
        static LAST_LIGHT: Cell<bool> = const { Cell::new(false) };
    }
    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if matches!(msg, WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE) {
            let light = super::is_light(super::Surface::Taskbar);
            if LAST_LIGHT.with(|v| v.replace(light)) != light {
                let sink = SINK.with(|s| s.borrow().clone());
                if let Some(sink) = sink {
                    let _ = sink.OnUpdate(TF_LBI_ICON);
                }
            }
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
    pub fn watch(sink: ITfLangBarItemSink) {
        SINK.with(|s| *s.borrow_mut() = Some(sink));
        LAST_LIGHT.with(|v| v.set(super::is_light(super::Surface::Taskbar)));
        if WINDOW.with(|h| h.get()) != 0 {
            return;
        }
        unsafe {
            static REGISTER: std::sync::Once = std::sync::Once::new();
            let module = GetModuleHandleW(None).unwrap_or_default();
            REGISTER.call_once(|| {
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: module.into(),
                    lpszClassName: w!("RakukanThemeObserver"),
                    ..Default::default()
                });
            });
            match CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("RakukanThemeObserver"),
                None,
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                module,
                None,
            ) {
                Ok(hwnd) => WINDOW.with(|h| h.set(hwnd.0 as isize)),
                Err(e) => tracing::warn!("theme observer creation failed: {e}"),
            }
        }
    }
    pub fn stop() {
        SINK.with(|s| *s.borrow_mut() = None);
        let hwnd = WINDOW.with(|h| h.replace(0));
        if hwnd != 0 {
            unsafe {
                let _ = DestroyWindow(HWND(hwnd as *mut _));
            }
        }
    }
}
pub use observer::{stop, watch};

#[cfg(test)]
mod tests {
    use super::*;
    fn luminance(color: COLORREF) -> f64 {
        let component = |shift: u32| {
            let c = ((color.0 >> shift) & 255u32) as f64 / 255.;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * component(0) + 0.7152 * component(8) + 0.0722 * component(16)
    }
    #[test]
    fn app_and_taskbar_use_independent_settings() {
        assert_eq!(Surface::App.value_name(), "AppsUseLightTheme");
        assert_eq!(Surface::Taskbar.value_name(), "SystemUsesLightTheme");
        assert!(Surface::App.default_light());
        assert!(!Surface::Taskbar.default_light());
    }
    #[test]
    fn both_palettes_have_readable_text_in_every_row() {
        for light in [true, false] {
            let p = palette(light);
            for (fg, bg) in [
                (p.foreground, p.background),
                (p.selected_foreground, p.selected_background),
                (p.secondary, p.pager_background),
                (p.secondary, p.status_background),
            ] {
                let (a, b) = (luminance(fg), luminance(bg));
                assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
            }
        }
    }
}
