//! Optional input modes: local character conversion, without dictionary or LLM lookup.
use std::cell::RefCell;

use anyhow::Result;
use rakukan_romaji::RomajiConverter;
use windows::Win32::UI::TextServices::{ITfCompositionSink, ITfContext};
use windows::core::Interface;

use crate::engine::{ime_mode::ImeMode, state::session_get, text_util, user_action::UserAction};

struct Buffer {
    context: usize,
    mode: ImeMode,
    romaji: RomajiConverter,
    latin: String,
}

impl Buffer {
    fn new(context: usize, mode: ImeMode) -> Self {
        Self {
            context,
            mode,
            romaji: RomajiConverter::new(),
            latin: String::new(),
        }
    }

    fn push(&mut self, ch: char) {
        if self.mode == ImeMode::HalfKatakana {
            self.romaji.push(ch.to_ascii_lowercase());
        } else {
            self.latin.push(match ch {
                ' ' => '　',
                '!'..='~' => char::from_u32(ch as u32 + 0xfee0).unwrap_or(ch),
                _ => ch,
            });
        }
    }

    fn text(&self) -> String {
        if self.mode == ImeMode::HalfKatakana {
            text_util::to_half_katakana(&self.romaji.full_text())
        } else {
            self.latin.clone()
        }
    }

    fn finish(&mut self) -> String {
        if self.mode == ImeMode::HalfKatakana && self.romaji.buffer() == "n" {
            return format!("{}ﾝ", text_util::to_half_katakana(self.romaji.output()));
        }
        self.romaji.flush();
        self.text()
    }

    fn backspace(&mut self) {
        if self.mode == ImeMode::HalfKatakana {
            self.romaji.backspace();
        } else {
            self.latin.pop();
        }
    }
}

thread_local! { static BUFFER: RefCell<Option<Buffer>> = const { RefCell::new(None) }; }

impl super::TextServiceFactory_Impl {
    pub(super) fn focused_context(&self) -> Option<ITfContext> {
        let tm = self.inner.try_borrow().ok()?.thread_mgr.clone()?;
        unsafe { tm.GetFocus().ok()?.GetTop().ok() }
    }

    pub(super) fn finish_direct_mode(&self, ctx: ITfContext, tid: u32) -> Result<()> {
        let active = session_get().map(|s| !s.is_idle()).unwrap_or(false);
        let text = BUFFER.with(|b| {
            b.borrow_mut()
                .take()
                .filter(|b| b.context == ctx.as_raw() as usize && active)
                .map(|mut b| b.finish())
        });
        if let Some(text) = text {
            session_get()?.set_idle();
            super::end_composition(ctx, tid, text)?;
        }
        Ok(())
    }

    pub(super) fn handle_direct_mode(
        &self,
        action: &UserAction,
        ctx: ITfContext,
        tid: u32,
        sink: ITfCompositionSink,
        mode: ImeMode,
    ) -> Result<Option<bool>> {
        if matches!(
            action,
            UserAction::ImeOn | UserAction::ImeOff | UserAction::ImeToggle
        ) {
            self.finish_direct_mode(ctx.clone(), tid)?;
            let next = match action {
                UserAction::ImeOn => ImeMode::On,
                _ => ImeMode::Off,
            };
            return self.switch_ime(Some(ctx), tid, next).map(Some);
        }
        let idle = session_get()?.is_idle();
        if matches!(action, UserAction::CommitRaw) {
            if idle {
                return Ok(Some(false));
            }
            self.finish_direct_mode(ctx, tid)?;
            return Ok(Some(true));
        }
        if matches!(action, UserAction::Cancel | UserAction::CancelAll) {
            if idle {
                return Ok(Some(false));
            }
            BUFFER.with(|b| *b.borrow_mut() = None);
            session_get()?.set_idle();
            super::end_composition(ctx, tid, String::new())?;
            return Ok(Some(true));
        }
        if matches!(action, UserAction::Backspace) && idle {
            return Ok(Some(false));
        }
        let text = BUFFER.with(|slot| {
            let mut slot = slot.borrow_mut();
            if idle
                || slot
                    .as_ref()
                    .is_none_or(|b| b.context != ctx.as_raw() as usize || b.mode != mode)
            {
                *slot = Some(Buffer::new(ctx.as_raw() as usize, mode));
            }
            let b = slot.as_mut().unwrap();
            match action {
                UserAction::Input(ch) | UserAction::InputRaw(ch) | UserAction::Punctuate(ch) => {
                    b.push(*ch)
                }
                UserAction::Convert => b.push(' '),
                UserAction::FullWidthSpace => b.push('　'),
                UserAction::Backspace => b.backspace(),
                _ => return None,
            }
            Some(b.text())
        });
        if let Some(text) = text {
            crate::tsf::candidate_window::hide();
            crate::tsf::candidate_window::stop_live_timer();
            if text.is_empty() {
                session_get()?.set_idle();
                super::end_composition(ctx, tid, text)?;
            } else {
                session_get()?.set_preedit(text.clone());
                super::update_composition(ctx, tid, sink, text)?;
            }
            return Ok(Some(true));
        }
        // Keep character-conversion / candidate keys from starting dictionary conversion.
        Ok(Some(!idle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_katakana_typing_backspace_and_pending_n() {
        let mut b = Buffer::new(0, ImeMode::HalfKatakana);
        for c in "gakkou".chars() {
            b.push(c);
        }
        assert_eq!(b.text(), "ｶﾞｯｺｳ");
        b.backspace();
        assert_eq!(b.text(), "ｶﾞｯｺ");
        b.push('n');
        assert_eq!(b.finish(), "ｶﾞｯｺﾝ");
    }

    #[test]
    fn full_alphanumeric_preserves_case_and_converts_digits_symbols_space() {
        let mut b = Buffer::new(0, ImeMode::FullAlphanumeric);
        for c in "Abc12! ".chars() {
            b.push(c);
        }
        assert_eq!(b.text(), "Ａｂｃ１２！　");
        b.backspace();
        assert_eq!(b.finish(), "Ａｂｃ１２！");
    }

    #[test]
    fn optional_modes_are_independently_disabled_by_default() {
        let mut input = crate::engine::config::InputConfig::default();
        assert!(!ImeMode::HalfKatakana.allowed(&input));
        assert!(!ImeMode::FullAlphanumeric.allowed(&input));
        input.half_katakana_mode_enabled = true;
        assert!(ImeMode::HalfKatakana.allowed(&input));
        assert!(!ImeMode::FullAlphanumeric.allowed(&input));
        input.full_alphanumeric_mode_enabled = true;
        assert!(ImeMode::FullAlphanumeric.allowed(&input));
        assert!(ImeMode::On.allowed(&input) && ImeMode::Off.allowed(&input));
    }
}
