//! Own the session-wide accessibility caret width in one resident process.
//! No SPIF_UPDATEINIFILE: do not overwrite the user's saved Windows preference.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SPI_GETCARETWIDTH, SPI_SETCARETWIDTH,
    SPIF_SENDCHANGE, SystemParametersInfoW,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct Appearance {
    caret_width_enabled: bool,
    caret_on_width: u32,
    caret_off_width: u32,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            caret_width_enabled: false,
            caret_on_width: 4,
            caret_off_width: 1,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Config {
    appearance: Appearance,
}

fn desired(config: &Appearance, mode: u64, foreground: u32) -> Option<u32> {
    if !config.caret_width_enabled || foreground == 0 || (mode >> 32) as u32 != foreground {
        return None;
    }
    Some(
        if mode & 0x100 != 0 {
            config.caret_on_width
        } else {
            config.caret_off_width
        }
        .clamp(1, 20),
    )
}

pub fn current_width() -> windows::core::Result<u32> {
    let mut width = 0u32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETCARETWIDTH,
            0,
            Some((&mut width as *mut u32).cast()),
            Default::default(),
        )?;
    }
    Ok(width)
}
fn set_width(width: u32) -> windows::core::Result<()> {
    unsafe {
        SystemParametersInfoW(
            SPI_SETCARETWIDTH,
            0,
            Some(width as usize as *mut _),
            SPIF_SENDCHANGE,
        )
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
struct Saved {
    original: u32,
    applied: u32,
}

pub struct Controller {
    config: Appearance,
    checked: Option<Instant>,
    saved: Option<Saved>,
    journal: PathBuf,
}
impl Controller {
    pub fn new() -> Self {
        let journal = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("rakukan")
            .join("caret-width-restore.toml");
        let saved = fs::read_to_string(&journal)
            .ok()
            .and_then(|text| toml::from_str(&text).ok());
        let mut this = Self {
            config: Appearance::default(),
            checked: None,
            saved,
            journal,
        };
        // Recover after a crash, but only if nobody has changed the setting since us.
        this.restore();
        this
    }

    pub fn update(&mut self, mode: u64) {
        if self
            .checked
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
        {
            self.checked = Some(Instant::now());
            let config = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("rakukan").join("config.toml"))
                .and_then(|p| fs::read_to_string(p).ok())
                .and_then(|text| toml::from_str::<Config>(&text).ok());
            self.config = config.map(|c| c.appearance).unwrap_or_default();
        }
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        }
        self.apply(desired(&self.config, mode, pid));
    }

    fn apply(&mut self, desired: Option<u32>) {
        let Some(width) = desired else {
            self.restore();
            return;
        };
        let Ok(current) = current_width() else {
            return;
        };
        if current == width {
            return;
        }
        let original = self
            .saved
            .filter(|s| s.applied == current)
            .map_or(current, |s| s.original);
        let saved = Saved {
            original,
            applied: width,
        };
        // Record before changing the system so a restart can restore after interruption.
        let Ok(text) = toml::to_string(&saved) else {
            return;
        };
        if let Some(parent) = self.journal.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let temporary = self.journal.with_extension("tmp");
        if fs::write(&temporary, text)
            .and_then(|_| fs::rename(&temporary, &self.journal))
            .is_err()
        {
            return;
        }
        if set_width(width).is_ok() {
            self.saved = Some(saved);
        } else if let Some(previous) = self.saved {
            // Restore the previous recovery record if the Windows call failed.
            if let Ok(text) = toml::to_string(&previous) {
                let _ = fs::write(&self.journal, text);
            }
        } else {
            let _ = fs::remove_file(&self.journal);
        }
    }

    fn restore(&mut self) {
        let Some(saved) = self.saved else {
            return;
        };
        let Ok(current) = current_width() else {
            return;
        };
        if current == saved.applied && set_width(saved.original).is_err() {
            return;
        }
        self.saved = None;
        let _ = fs::remove_file(&self.journal);
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_width_respects_enabled_foreground_and_limits() {
        let mut cfg = Appearance {
            caret_width_enabled: true,
            ..Default::default()
        };
        assert_eq!(desired(&cfg, (123u64 << 32) | 0x100, 123), Some(4));
        assert_eq!(desired(&cfg, 123u64 << 32, 123), Some(1));
        assert_eq!(desired(&cfg, 123u64 << 32, 124), None);
        assert_eq!(desired(&cfg, 0, 0), None);
        cfg.caret_on_width = 100;
        cfg.caret_off_width = 0;
        assert_eq!(desired(&cfg, (123u64 << 32) | 0x100, 123), Some(20));
        assert_eq!(desired(&cfg, 123u64 << 32, 123), Some(1));
        cfg.caret_width_enabled = false;
        assert_eq!(desired(&cfg, 123u64 << 32, 123), None);
        assert!(
            !toml::from_str::<Config>("[appearance]\ncandidate_font_height=17")
                .unwrap()
                .appearance
                .caret_width_enabled
        );
    }

    #[test]
    #[ignore = "temporarily changes the Windows accessibility caret width; run explicitly"]
    fn windows_width_switch_and_restore() {
        let original = current_width().unwrap();
        struct Restore(u32);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = set_width(self.0);
            }
        }
        let _restore = Restore(original);
        let journal =
            std::env::temp_dir().join(format!("rakukan-caret-test-{}.toml", std::process::id()));
        let mut controller = Controller {
            config: Appearance::default(),
            checked: None,
            saved: None,
            journal,
        };
        controller.apply(Some(4));
        assert_eq!(current_width().unwrap(), 4);
        controller.apply(Some(1));
        assert_eq!(current_width().unwrap(), 1);
        controller.apply(None);
        assert_eq!(current_width().unwrap(), original);
        controller.apply(Some(if original == 4 { 5 } else { 4 }));
        // Reconstruct ownership from the on-disk recovery record after a forced exit.
        controller.saved = toml::from_str(&fs::read_to_string(&controller.journal).unwrap()).ok();
        controller.restore();
        assert_eq!(current_width().unwrap(), original);
        controller.apply(Some(4));
        // A Windows settings change must not be overwritten when disabling.
        set_width(7).unwrap();
        controller.apply(None);
        assert_eq!(current_width().unwrap(), 7);
        println!(
            "Windows caret width: 4 -> 1 -> original ({original}), external change preserved: PASS"
        );
    }
}
