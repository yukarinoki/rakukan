//! Selection-based reconversion. Never insert at a fallback document position.
use crate::{
    engine::state::{self, CandidateViewSource},
    tsf::{candidate_window, edit_session::EditSession},
};
use std::{cell::RefCell, rc::Rc};
use windows::{
    Win32::{Foundation::*, UI::TextServices::*},
    core::Interface,
};

thread_local! {
    static ORIGINAL: RefCell<Option<(ITfComposition, String)>> = const { RefCell::new(None) };
}
pub(super) fn clear() {
    ORIGINAL.with(|v| v.borrow_mut().take());
}

fn valid_text(text: &str) -> bool {
    !text.is_empty() && text.chars().count() <= 128 && !text.chars().any(char::is_control)
}

unsafe fn selection(ctx: &ITfContext, ec: u32) -> windows::core::Result<(ITfRange, String)> {
    unsafe {
        if ctx.GetStatus()?.dwDynamicFlags & TS_SD_READONLY != 0 {
            return Err(E_FAIL.into());
        }
        let mut sel = [TF_SELECTION::default()];
        let mut fetched = 0;
        let result = ctx.GetSelection(ec, TF_DEFAULT_SELECTION, &mut sel, &mut fetched);
        let range = std::mem::ManuallyDrop::take(&mut sel[0].range);
        result?;
        let range = range
            .filter(|_| fetched == 1)
            .ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let text = range_text(&range, ec)?;
        Ok((range, text))
    }
}
unsafe fn range_text(range: &ITfRange, ec: u32) -> windows::core::Result<String> {
    let mut buffer = [0u16; 257];
    let mut count = 0;
    unsafe {
        range.GetText(ec, 0, &mut buffer, &mut count)?;
    }
    let text = String::from_utf16(&buffer[..count as usize])
        .map_err(|_| windows::core::Error::from(E_FAIL))?;
    if !valid_text(&text) {
        return Err(E_FAIL.into());
    }
    Ok(text)
}

pub(super) fn shortcut(vk: u16) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
    let down = |key| unsafe { GetKeyState(key) as u16 & 0x8000 != 0 };
    (vk == 0x1c && !down(0x11) && !down(0x10) && !down(0x12))
        || (vk == 0x52 && down(0x11) && down(0x10) && !down(0x12))
}

pub(super) fn selected(ctx: &ITfContext, tid: u32) -> Option<(ITfRange, String)> {
    if state::composition_clone().ok()?.is_some()
        || !matches!(*state::session_get().ok()?, state::SessionState::Idle)
    {
        return None;
    }
    let output = Rc::new(RefCell::new(None));
    let out = output.clone();
    let context = ctx.clone();
    let edit = EditSession::new(move |ec| {
        *out.borrow_mut() = unsafe { selection(&context, ec) }.ok();
        Ok(())
    });
    unsafe {
        ctx.RequestEditSession(tid, &edit, TF_ES_SYNC | TF_ES_READ)
            .ok()?
            .ok()
            .ok()?;
    }

    output.borrow_mut().take()
}

pub(super) fn begin(
    thread_mgr: ITfThreadMgr,
    ctx: ITfContext,
    tid: u32,
    sink: ITfCompositionSink,
    range: ITfRange,
    original: String,
) -> windows::core::Result<()> {
    let request = ctx.clone();
    let edit = EditSession::new(move |ec| unsafe {
        if thread_mgr.GetFocus()?.GetTop()?.as_raw() != ctx.as_raw() {
            return Ok(());
        }
        if state::composition_clone().ok().flatten().is_some() {
            return Ok(());
        }
        // The asynchronous edit session may run after a selection/focus change.
        let (current, text) = selection(&ctx, ec)?;
        if text != original
            || current.CompareStart(ec, &range, TF_ANCHOR_START)? != 0
            || current.CompareEnd(ec, &range, TF_ANCHOR_END)? != 0
        {
            return Ok(());
        }
        let mut guard =
            state::engine_try_get_or_create().map_err(|_| windows::core::Error::from(E_FAIL))?;
        let engine = guard
            .as_mut()
            .ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let _ = state::poll_dict_ready_cached(engine);
        let readings = engine.reverse_readings(&original);
        if readings.is_empty() {
            tracing::info!(
                "reconversion unavailable: no reading (chars={})",
                original.chars().count()
            );
            return Ok(());
        }
        let reading = readings[0].clone();
        let mut candidates = vec![original.clone()];
        // Round-robin alternative readings so suffix readings cannot crowd out はし etc.
        let groups: Vec<_> = readings
            .iter()
            .map(|r| engine.merge_candidates_for_reading(r, Vec::new(), 40))
            .collect();
        for i in 0..40 {
            for group in &groups {
                if let Some(c) = group.get(i)
                    && candidates.len() < 80
                    && !candidates.contains(c)
                {
                    candidates.push(c.clone());
                }
            }
        }
        let mut sess = state::session_get().map_err(|_| windows::core::Error::from(E_FAIL))?;
        if !matches!(*sess, state::SessionState::Idle) {
            return Ok(());
        }
        let cc: ITfContextComposition = ctx.cast()?;
        let dm = ctx.GetDocumentMgr()?.as_raw() as usize;
        let composition = cc.StartComposition(ec, &range, &sink)?;
        if state::composition_set_with_dm(Some(composition.clone()), dm).is_err() {
            let _ = composition.EndComposition(ec);
            return Err(E_FAIL.into());
        }
        ORIGINAL.with(|v| *v.borrow_mut() = Some((composition, original)));
        crate::tsf::live_session::conv_gen_bump();
        candidate_window::stop_live_timer();
        crate::tsf::live_session::queue_preview_clear();
        engine.bg_reclaim();
        engine.reset_preedit();
        engine.force_preedit(reading.clone());
        let _ = state::poll_model_ready_cached(engine);
        let pending = engine.is_kanji_ready() && engine.bg_start(state::get_num_candidates());
        let (x, y) = super::get_caret_pos_from_context(&ctx, ec).unwrap_or_else(|| {
            let r = state::caret_rect_get();
            (r.left, r.bottom)
        });
        sess.activate_selecting(candidates, reading, x, y, pending);
        sess.rebuild_selecting_candidate_views(CandidateViewSource::Reconversion);
        let page = sess.page_candidates().to_vec();
        let info = sess.page_info();
        drop(sess);
        drop(guard);
        candidate_window::show(&page, 0, &info, x, y);
        if pending {
            candidate_window::start_waiting_timer();
        }
        tracing::info!("reconversion started pending={pending}");
        Ok(())
    });
    unsafe {
        request
            .RequestEditSession(tid, &edit, TF_ES_READWRITE)?
            .ok()?;
    }
    Ok(())
}

pub(super) fn cancel(ctx: ITfContext, tid: u32) -> anyhow::Result<bool> {
    let current = state::composition_clone()?;
    let original = ORIGINAL.with(|v| {
        v.borrow().as_ref().and_then(|(comp, text)| {
            current
                .as_ref()
                .filter(|c| c.as_raw() == comp.as_raw())
                .map(|_| text.clone())
        })
    });
    let Some(original) = original else {
        return Ok(false);
    };
    let composition = current.unwrap();
    let request = ctx.clone();
    crate::tsf::live_session::conv_gen_bump();
    let edit = EditSession::new(move |ec| unsafe {
        if state::composition_clone()
            .ok()
            .flatten()
            .as_ref()
            .is_none_or(|c| c.as_raw() != composition.as_raw())
        {
            return Ok(());
        }
        let range = composition.GetRange()?;
        // Do not clear recovery state unless restoring the original text succeeds.
        range.SetText(ec, 0, &original.encode_utf16().collect::<Vec<_>>())?;
        let cursor = range.Clone()?;
        cursor.Collapse(ec, TF_ANCHOR_END)?;
        let mut sel = TF_SELECTION {
            range: std::mem::ManuallyDrop::new(Some(cursor)),
            style: TF_SELECTIONSTYLE {
                ase: TF_AE_NONE,
                fInterimChar: FALSE,
            },
        };
        let result = ctx.SetSelection(ec, std::slice::from_ref(&sel));
        std::mem::ManuallyDrop::drop(&mut sel.range);
        result?;
        composition.EndComposition(ec)?;
        let _ = state::composition_set(None);
        clear();
        if let Ok(mut guard) = state::engine_get()
            && let Some(e) = guard.as_mut()
        {
            e.bg_reclaim();
            e.reset_preedit();
        }
        if let Ok(mut sess) = state::session_get() {
            sess.set_idle();
        }
        candidate_window::hide();
        candidate_window::stop_waiting_timer();
        Ok(())
    });
    unsafe {
        request
            .RequestEditSession(tid, &edit, TF_ES_READWRITE)?
            .ok()?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_empty_multiline_and_oversized_selections() {
        assert!(valid_text("橋"));
        assert!(valid_text("日本の橋"));
        assert!(!valid_text(""));
        assert!(!valid_text("日本\n橋"));
        assert!(!valid_text(&"あ".repeat(129)));
    }
}
