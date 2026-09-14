/// IME state. Optional half-width katakana and full-width alphanumeric modes
/// are enabled individually in InputConfig; both keep KEYBOARD_OPENCLOSE open.
#[derive(Default, Copy, Clone, PartialEq, Eq, Debug)]
pub enum ImeMode {
    #[default]
    On,
    Off,
    HalfKatakana,
    FullAlphanumeric,
}

impl ImeMode {
    pub fn is_direct(self) -> bool {
        matches!(self, Self::HalfKatakana | Self::FullAlphanumeric)
    }

    pub fn allowed(self, input: &super::config::InputConfig) -> bool {
        match self {
            Self::HalfKatakana => input.half_katakana_mode_enabled,
            Self::FullAlphanumeric => input.full_alphanumeric_mode_enabled,
            _ => true,
        }
    }

    #[inline]
    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// コンパートメント値（開=true）から変換する。
    #[inline]
    pub fn from_open(open: bool) -> Self {
        if open { Self::On } else { Self::Off }
    }

    #[inline]
    pub fn toggled(self) -> Self {
        match self {
            Self::On | Self::HalfKatakana | Self::FullAlphanumeric => Self::Off,
            Self::Off => Self::On,
        }
    }

    /// 言語バー / インジケーターの表示文字。
    pub fn label(self) -> &'static str {
        match self {
            Self::On => "あ",
            Self::Off => "A",
            Self::HalfKatakana => "ｶ",
            Self::FullAlphanumeric => "Ａ",
        }
    }

    /// 診断イベント用の名前。
    pub fn name(self) -> &'static str {
        match self {
            Self::On => "On",
            Self::Off => "Off",
            Self::HalfKatakana => "HalfKatakana",
            Self::FullAlphanumeric => "FullAlphanumeric",
        }
    }
}
