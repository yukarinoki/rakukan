//! AI is an isolated editing session: no app text is changed until acceptance.
use crate::{
    engine::{state, user_action::UserAction},
    tsf::{ai_window, candidate_window, edit_session::EditSession},
};
use rakukan_romaji::RomajiConverter;
use serde::Deserialize;
use std::{
    cell::RefCell,
    io::Write,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime},
};
use windows::{
    Win32::{
        Foundation::*,
        UI::{Input::KeyboardAndMouse::GetKeyState, TextServices::*},
    },
    core::Interface,
};

#[derive(Clone, Deserialize)]
#[serde(default)]
struct Config {
    enabled: bool,
    key: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            key: "Henkan".into(),
        }
    }
}
#[derive(Deserialize)]
struct Reply {
    text: String,
    append: bool,
    elapsed_ms: u64,
    error: Option<String>,
    #[serde(default)]
    language: String,
}
struct Target {
    ctx: ITfContext,
    mgr: ITfThreadMgr,
    range: ITfRange,
    composition: Option<ITfComposition>,
    text: String,
    tid: u32,
}
struct Session {
    target: Option<Target>,
    input: RomajiConverter,
    instruction: String,
    result: Option<Reply>,
    previous: String,
    last_instruction: String,
    append: bool,
    language: String,
    rx: Option<mpsc::Receiver<Result<Reply, String>>>,
    cancel: Arc<AtomicBool>,
    message: String,
    geometry: Option<ai_window::Geometry>,
    geometry_checked: std::time::Instant,
    page: usize,
}

#[derive(Debug, PartialEq)]
enum EnterIntent {
    Generate,
    Accept,
    Wait,
}
fn enter_intent(has_instruction: bool, has_result: bool, busy: bool) -> EnterIntent {
    if has_instruction {
        EnterIntent::Generate
    } else if has_result {
        EnterIntent::Accept
    } else if busy {
        EnterIntent::Wait
    } else {
        EnterIntent::Generate
    }
}
thread_local! {
    static SESSION:RefCell<Option<Session>> = const {RefCell::new(None)};
    static CONFIG:RefCell<(Option<SystemTime>, Config, std::time::Instant)> = RefCell::new((None,Config::default(),std::time::Instant::now()-Duration::from_secs(2)));
}
fn config() -> Config {
    CONFIG.with(|c| {
        let mut c = c.borrow_mut();
        if c.2.elapsed() > Duration::from_millis(500) {
            c.2 = std::time::Instant::now();
            let path = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
                .join("rakukan/ai.json");
            let stamp = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            if stamp != c.0 {
                c.0 = stamp;
                c.1 = std::fs::read(path)
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
            }
        }
        c.1.clone()
    })
}
pub(super) fn shortcut(vk: u16) -> bool {
    let cfg = config();
    if !cfg.enabled {
        return false;
    }
    let Some(spec) = crate::engine::keymap::KeySpec::parse(&cfg.key) else {
        return false;
    };
    use crate::engine::keymap::ModReq;
    let down = |k| unsafe { GetKeyState(k) as u16 & 0x8000 != 0 };
    spec.vk == vk
        && (spec.ctrl != ModReq::Off) == down(0x11)
        && (spec.shift != ModReq::Off) == down(0x10)
        && (spec.alt != ModReq::Off) == down(0x12)
}
pub(in crate::tsf) fn active() -> bool {
    SESSION.with(|s| s.borrow().is_some())
}
pub(super) fn cancel() {
    let old = SESSION.with(|s| s.borrow_mut().take());
    if let Some(s) = old {
        s.cancel.store(true, Ordering::Relaxed);
        candidate_window::ai_timer(false);
        candidate_window::hide();
        ai_window::hide();
    }
}
unsafe fn text(range: &ITfRange, ec: u32) -> windows::core::Result<String> {
    let mut buf = vec![0u16; 8193];
    let mut count = 0;
    unsafe {
        range.GetText(ec, 0, &mut buf, &mut count)?;
    }
    if count > 8192 {
        return Err(E_FAIL.into());
    }
    String::from_utf16(&buf[..count as usize]).map_err(|_| E_FAIL.into())
}
unsafe fn selection(ctx: &ITfContext, ec: u32) -> windows::core::Result<ITfRange> {
    let mut sel = [TF_SELECTION::default()];
    let mut count = 0;
    let result = unsafe { ctx.GetSelection(ec, TF_DEFAULT_SELECTION, &mut sel, &mut count) };
    let range = unsafe { std::mem::ManuallyDrop::take(&mut sel[0].range) };
    result?;
    range.filter(|_| count == 1).ok_or_else(|| E_FAIL.into())
}
pub(super) fn begin(ctx: ITfContext, mgr: ITfThreadMgr, tid: u32) -> windows::core::Result<()> {
    cancel();
    let out = Rc::new(RefCell::new(None));
    let output = out.clone();
    let context = ctx.clone();
    let edit = EditSession::new(move |ec| unsafe {
        if context.GetStatus()?.dwDynamicFlags & TS_SD_READONLY != 0 {
            return Err(E_FAIL.into());
        }
        // Password fields commonly disable TSF; also reject explicit password input scopes.
        let property = context.GetAppProperty(&GUID_PROP_INPUTSCOPE)?;
        if let Ok(value) = property.GetValue(ec, &selection(&context, ec)?)
            && let Ok(unknown) = windows::core::IUnknown::try_from(&value)
            && let Ok(scope) = unknown.cast::<ITfInputScope>()
        {
            let mut scopes = std::ptr::null_mut();
            let mut count = 0;
            if scope.GetInputScopes(&mut scopes, &mut count).is_ok() {
                let password = !scopes.is_null()
                    && std::slice::from_raw_parts(scopes, count as usize).contains(&IS_PASSWORD);
                windows::Win32::System::Com::CoTaskMemFree(Some(scopes as *const _));
                if password {
                    return Err(E_FAIL.into());
                }
            }
        }
        let composition = state::composition_clone().ok().flatten();
        let range = match &composition {
            Some(comp) => comp.GetRange()?,
            None => selection(&context, ec)?,
        };
        let original = text(&range, ec)?;
        if original.trim().is_empty() {
            return Ok(());
        }
        let position = geometry(&context, &range, &original, ec)?;
        *output.borrow_mut() = Some((range, original, composition, position));
        Ok(())
    });
    let captured = unsafe {
        ctx.RequestEditSession(tid, &edit, TF_ES_SYNC | TF_ES_READ)
            .and_then(|r| r.ok())
    };
    let captured_target = out.borrow_mut().take();
    captured?;
    let Some((range, text, composition, position)) = captured_target else {
        return Ok(());
    };
    let target = Some(Target {
        ctx,
        mgr,
        range,
        composition,
        text,
        tid,
    });
    crate::tsf::live_session::conv_gen_bump();
    candidate_window::stop_live_timer();
    candidate_window::stop_waiting_timer();
    candidate_window::hide();
    let mut session = Session {
        target,
        input: RomajiConverter::new(),
        instruction: String::new(),
        result: None,
        previous: String::new(),
        last_instruction: String::new(),
        append: true,
        language: "japanese".into(),
        rx: None,
        cancel: Arc::new(AtomicBool::new(false)),
        message: String::new(),
        geometry: Some(position),
        geometry_checked: std::time::Instant::now(),
        page: 0,
    };
    start(&mut session);
    SESSION.with(|s| *s.borrow_mut() = Some(session));
    render();
    candidate_window::ai_timer(true);
    Ok(())
}
fn start(s: &mut Session) {
    let Some(target) = &s.target else { return };
    s.cancel.store(true, Ordering::Relaxed);
    s.cancel = Arc::new(AtomicBool::new(false));
    let cancel = s.cancel.clone();
    let (tx, rx) = mpsc::channel();
    s.rx = Some(rx);
    s.result = None;
    s.page = 0;
    s.message = "AI生成中… 文字入力で補正・Escで取消".into();
    let request = serde_json::json!({"text":target.text,"instruction":s.last_instruction,"previous":s.previous,"append":s.append,"language":s.language});
    std::thread::spawn(move || {
        let run = || -> Result<Reply, String> {
            let exe = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
                .join("rakukan/ai/rakukan-ai.exe");
            let mut child = Command::new(exe)
                .creation_flags(0x08000000)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| {
                    "AI実行プログラムが見つかりません。インストールを確認してください。".to_string()
                })?;
            if let Some(mut stdin) = child.stdin.take()
                && stdin.write_all(request.to_string().as_bytes()).is_err()
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err("AI要求を送信できませんでした。".into());
            }
            // Drain concurrently: a long output must never fill the child stdout pipe.
            let mut stdout = child.stdout.take().unwrap();
            let reader = std::thread::spawn(move || {
                use std::io::Read;
                let mut bytes = Vec::new();
                let _ = (&mut stdout).take(1048576).read_to_end(&mut bytes);
                bytes
            });
            let began = std::time::Instant::now();
            loop {
                if cancel.load(Ordering::Relaxed) || began.elapsed() > Duration::from_secs(610) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err("AI生成をキャンセルしました。".into());
                }
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                    Err(_) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("AIプロセスが終了しました。".into());
                    }
                }
            }
            let bytes = reader.join().unwrap_or_default();
            serde_json::from_slice(&bytes).map_err(|_| {
                "AI応答形式が不正です。AI実行プログラムの更新を確認してください。".into()
            })
        };
        let _ = tx.send(run());
    });
}
pub(in crate::tsf) fn poll() {
    let focus = SESSION.with(|s| {
        s.borrow()
            .as_ref()
            .and_then(|s| s.target.as_ref())
            .map(|t| (t.mgr.clone(), t.ctx.clone()))
    });
    let focused = focus.is_none_or(|(mgr, ctx)| unsafe {
        mgr.GetFocus()
            .and_then(|d| d.GetTop())
            .is_ok_and(|c| c.as_raw() == ctx.as_raw())
    });
    if !focused {
        cancel();
        return;
    }
    SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return false };
        let Some(reply) = s.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        s.rx = None;
        match reply {
            Ok(r) if r.error.is_none() => {
                s.append = r.append;
                s.language = r.language.clone();
                s.message = format!(
                    "{} ms ・Enter 採用・文字入力 補正・Tab 別候補",
                    r.elapsed_ms
                );
                tracing::info!("AI generation completed elapsed_ms={}", r.elapsed_ms);
                s.result = Some(r)
            }
            Ok(r) => {
                tracing::warn!("AI generation failed kind=backend");
                s.message = r.error.unwrap_or_default();
            }
            Err(e) => {
                tracing::warn!("AI generation failed kind=transport");
                s.message = e;
            }
        }
        true
    });
    refresh_geometry();
    render();
}
fn render() {
    let view = SESSION.with(|s| {
        let s = s.borrow();
        let s = s.as_ref()?;
        let instruction = format!("{}{}", s.instruction, s.input.full_text());
        let text = if let Some(result) = &s.result {
            wrap(&result.text, 24)
                .into_iter()
                .skip(s.page * 5)
                .take(5)
                .collect::<Vec<_>>()
                .join("\n")
        } else if s.rx.is_some() {
            let chars: Vec<_> = s.last_instruction.chars().collect();
            chars[chars.len().saturating_sub(120)..].iter().collect()
        } else {
            // Keep the input caret visible for long instructions.
            let chars: Vec<_> = instruction.chars().collect();
            chars[chars.len().saturating_sub(120)..].iter().collect()
        };
        Some(ai_window::View {
            geometry: s.geometry.clone()?,
            text,
            replace: s.result.as_ref().is_some_and(|r| !r.append),
            busy: s.rx.is_some(),
            editing: s.rx.is_none() && s.result.is_none(),
        })
    });
    // Native windows and COM may reenter TSF. Never retain a SESSION borrow here.
    if let Some(view) = view {
        ai_window::show(view);
    } else {
        ai_window::hide();
    }
}

unsafe fn geometry(
    ctx: &ITfContext,
    range: &ITfRange,
    original: &str,
    ec: u32,
) -> windows::core::Result<ai_window::Geometry> {
    let view = unsafe { ctx.GetActiveView()? };
    let viewport = unsafe { view.GetScreenExt()? };
    let end = unsafe { range.Clone()? };
    unsafe {
        end.Collapse(ec, TF_ANCHOR_END)?;
    }
    let mut caret = RECT::default();
    let mut clipped = BOOL(0);
    unsafe {
        view.GetTextExt(ec, &end, &mut caret, &mut clipped)?;
    }
    if clipped.as_bool() || caret.bottom <= caret.top {
        return Err(E_FAIL.into());
    }
    let mut lines = Vec::new();
    let units: Vec<i32> = original.chars().map(|c| c.len_utf16() as i32).collect();
    unsafe {
        line_rects(
            &view,
            range,
            ec,
            &units,
            caret.bottom - caret.top,
            &mut lines,
        )?;
    }
    if lines.is_empty() {
        return Err(E_FAIL.into());
    }
    Ok(ai_window::Geometry {
        caret,
        lines,
        viewport,
    })
}

// Split only multiline ranges, preserving UTF-16 scalar boundaries. A single-line
// paragraph costs one GetTextExt call instead of one COM call for every character.
unsafe fn line_rects(
    view: &ITfContextView,
    range: &ITfRange,
    ec: u32,
    units: &[i32],
    height: i32,
    lines: &mut Vec<RECT>,
) -> windows::core::Result<()> {
    let mut rect = RECT::default();
    let mut clipped = BOOL(0);
    unsafe {
        view.GetTextExt(ec, range, &mut rect, &mut clipped)?;
    }
    if clipped.as_bool() {
        return Err(E_FAIL.into());
    }
    if units.len() <= 1 || rect.bottom - rect.top <= height * 3 / 2 {
        if rect.right > rect.left && rect.bottom > rect.top {
            if let Some(last) = lines.last_mut()
                && last.top == rect.top
                && last.bottom == rect.bottom
            {
                last.left = last.left.min(rect.left);
                last.right = last.right.max(rect.right);
            } else {
                lines.push(rect);
            }
        }
        return Ok(());
    }
    let mid = units.len() / 2;
    let left = unsafe { range.Clone()? };
    let right = unsafe { range.Clone()? };
    let mut moved = 0;
    let trim: i32 = units[mid..].iter().sum();
    unsafe {
        left.ShiftEnd(ec, -trim, &mut moved, std::ptr::null())?;
    }
    if moved != -trim {
        return Err(E_FAIL.into());
    }
    let skip: i32 = units[..mid].iter().sum();
    unsafe {
        right.ShiftStart(ec, skip, &mut moved, std::ptr::null())?;
    }
    if moved != skip {
        return Err(E_FAIL.into());
    }
    unsafe {
        line_rects(view, &left, ec, &units[..mid], height, lines)?;
        line_rects(view, &right, ec, &units[mid..], height, lines)
    }
}
fn refresh_geometry() {
    let snapshot = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let s = s.as_mut()?;
        if s.geometry_checked.elapsed() < Duration::from_millis(240) {
            return None;
        }
        s.geometry_checked = std::time::Instant::now();
        let t = s.target.as_ref()?;
        Some((
            t.ctx.clone(),
            t.range.clone(),
            t.text.clone(),
            t.composition.clone(),
            t.tid,
        ))
    });
    let Some((ctx, range, original, composition, tid)) = snapshot else {
        return;
    };
    let output = Rc::new(RefCell::new(None));
    let captured = output.clone();
    let context = ctx.clone();
    let edit = EditSession::new(move |ec| unsafe {
        let unchanged = text(&range, ec)? == original
            && if let Some(comp) = &composition {
                state::composition_clone()
                    .ok()
                    .flatten()
                    .is_some_and(|c| c.as_raw() == comp.as_raw())
            } else {
                let selected = selection(&context, ec)?;
                selected.CompareStart(ec, &range, TF_ANCHOR_START)? == 0
                    && selected.CompareEnd(ec, &range, TF_ANCHOR_END)? == 0
            };
        *captured.borrow_mut() = Some((unchanged, geometry(&context, &range, &original, ec).ok()));
        Ok(())
    });
    let _ = unsafe { ctx.RequestEditSession(tid, &edit, TF_ES_SYNC | TF_ES_READ) };
    let captured = output.borrow_mut().take();
    if captured.as_ref().is_some_and(|(unchanged, _)| !unchanged) {
        cancel();
        return;
    }
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.geometry = captured.and_then(|(_, geometry)| geometry);
        }
    });
}
fn wrap(text: &str, width: usize) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                vec![String::new()]
            } else {
                chars.chunks(width).map(|c| c.iter().collect()).collect()
            }
        })
        .collect()
}
fn reject(s: &mut Session) {
    if let Some(result) = s.result.take() {
        s.previous = result.text;
    }
    s.cancel.store(true, Ordering::Relaxed);
    s.rx = None;
    s.message = "補正指示を入力してEnterで生成".into();
    s.page = 0;
}
pub(super) fn key(vk: u16, action: Option<UserAction>) -> windows::core::Result<()> {
    poll();
    if !active() {
        return Ok(());
    }
    if vk == 0x1b {
        cancel();
        return Ok(());
    }
    let mut accept = false;
    SESSION.with(|state| {
        let mut state = state.borrow_mut();
        let Some(s) = state.as_mut() else { return };
        match vk {
            0x0d => {
                s.input.flush();
                let instruction = format!("{}{}", s.instruction, s.input.full_text());
                match enter_intent(!instruction.is_empty(), s.result.is_some(), s.rx.is_some()) {
                    EnterIntent::Generate => {
                        if !instruction.is_empty() {
                            s.last_instruction = instruction;
                        }
                        s.instruction.clear();
                        s.input.reset();
                        start(s);
                    }
                    EnterIntent::Accept => accept = true,
                    EnterIntent::Wait => {}
                }
            }
            0x09 => {
                if s.rx.is_none() {
                    if let Some(r) = s.result.take() {
                        s.previous = r.text;
                    }
                    start(s)
                }
            }
            0x08 => {
                reject(s);
                if s.input.full_text().is_empty() {
                    s.instruction.pop();
                } else {
                    s.input.backspace();
                }
            }
            0x21 => s.page = s.page.saturating_sub(1),
            0x22 => {
                let max = s
                    .result
                    .as_ref()
                    .map(|r| wrap(&r.text, 24).len().saturating_sub(1) / 5)
                    .unwrap_or(0);
                s.page = (s.page + 1).min(max)
            }
            0x20 => {
                reject(s);
                s.input.flush();
                let hira = s.input.full_text();
                // Dictionary-only conversion: never reset the app's main preedit.
                let converted = state::engine_get()
                    .ok()
                    .and_then(|mut g| {
                        g.as_mut()
                            .map(|e| e.merge_candidates_for_reading(&hira, Vec::new(), 1))
                    })
                    .and_then(|v| v.into_iter().next())
                    .unwrap_or(hira);
                s.instruction.push_str(&converted);
                s.input.reset();
            }
            _ => {
                if let Some(UserAction::Input(c) | UserAction::InputRaw(c)) = action {
                    reject(s);
                    if s.instruction.len() + s.input.full_text().len() < 6000 {
                        s.input.push(c);
                    }
                }
            }
        }
    });
    if accept {
        commit()?
    } else {
        render()
    };
    Ok(())
}
fn commit() -> windows::core::Result<()> {
    let state = SESSION.with(|s| s.borrow_mut().take());
    let Some(mut session) = state else {
        return Ok(());
    };
    session.cancel.store(true, Ordering::Relaxed);
    candidate_window::ai_timer(false);
    candidate_window::hide();
    ai_window::hide();
    let Some(target) = session.target.take() else {
        return Ok(());
    };
    let Some(reply) = session.result.take() else {
        return Ok(());
    };
    let request = target.ctx.clone();
    let edit = EditSession::new(move |ec| unsafe {
        if target.mgr.GetFocus()?.GetTop()?.as_raw() != target.ctx.as_raw()
            || text(&target.range, ec)? != target.text
        {
            return Err(E_FAIL.into());
        }
        if target.ctx.GetStatus()?.dwDynamicFlags & TS_SD_READONLY != 0 {
            return Err(E_FAIL.into());
        }
        if let Some(comp) = &target.composition {
            if state::composition_clone()
                .ok()
                .flatten()
                .is_none_or(|c| c.as_raw() != comp.as_raw())
            {
                return Err(E_FAIL.into());
            }
        } else {
            let selected = selection(&target.ctx, ec)?;
            if selected.CompareStart(ec, &target.range, TF_ANCHOR_START)? != 0
                || selected.CompareEnd(ec, &target.range, TF_ANCHOR_END)? != 0
            {
                return Err(E_FAIL.into());
            }
        }
        let result = if reply.append {
            format!("{}{}", target.text, reply.text)
        } else {
            reply.text.clone()
        };
        target
            .range
            .SetText(ec, 0, &result.encode_utf16().collect::<Vec<_>>())?;
        let cursor = target.range.Clone()?;
        cursor.Collapse(ec, TF_ANCHOR_END)?;
        let mut sel = TF_SELECTION {
            range: std::mem::ManuallyDrop::new(Some(cursor)),
            style: TF_SELECTIONSTYLE {
                ase: TF_AE_NONE,
                fInterimChar: FALSE,
            },
        };
        let set = target.ctx.SetSelection(ec, std::slice::from_ref(&sel));
        std::mem::ManuallyDrop::drop(&mut sel.range);
        set?;
        if let Some(comp) = &target.composition {
            comp.EndComposition(ec)?;
            let _ = state::composition_set(None);
        }
        if let Ok(mut engine) = state::engine_get()
            && let Some(e) = engine.as_mut()
        {
            e.bg_reclaim();
            e.reset_preedit();
        }
        if let Ok(mut s) = state::session_get() {
            s.set_idle();
        }
        super::reconversion::clear();
        Ok(())
    });
    unsafe {
        request
            .RequestEditSession(target.tid, &edit, TF_ES_READWRITE)?
            .ok()?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapping_preserves_unicode() {
        assert_eq!(wrap("あいうえ\nabc", 2), vec!["あい", "うえ", "ab", "c"]);
    }
    #[test]
    fn missing_config_does_not_override_henkan() {
        assert!(!Config::default().enabled);
        assert_eq!(Config::default().key, "Henkan");
    }
    #[test]
    fn enter_never_accepts_a_result_after_typing_or_while_generating() {
        assert_eq!(enter_intent(false, true, false), EnterIntent::Accept);
        assert_eq!(enter_intent(true, true, false), EnterIntent::Generate);
        assert_eq!(enter_intent(true, false, true), EnterIntent::Generate);
        assert_eq!(enter_intent(false, false, true), EnterIntent::Wait);
        assert_eq!(enter_intent(false, false, false), EnterIntent::Generate);
    }
    #[test]
    fn rejecting_result_preserves_correction_context_and_cancels_old_request() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut s = Session {
            target: None,
            input: RomajiConverter::new(),
            instruction: String::new(),
            result: Some(Reply {
                text: "Previous English".into(),
                append: false,
                elapsed_ms: 1,
                error: None,
                language: "english".into(),
            }),
            previous: String::new(),
            last_instruction: "えいご".into(),
            append: false,
            language: "english".into(),
            rx: None,
            cancel: cancel.clone(),
            message: String::new(),
            geometry: None,
            geometry_checked: std::time::Instant::now(),
            page: 2,
        };
        reject(&mut s);
        assert!(s.result.is_none());
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(s.previous, "Previous English");
        assert_eq!(s.last_instruction, "えいご");
        assert_eq!(s.language, "english");
        assert!(!s.append);
        assert_eq!(s.page, 0);
    }
}
