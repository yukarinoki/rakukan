//! システムトレイ用：入力モード共有（DLL→トレイ常駐プロセス）
//!
//! TSF DLL は複数プロセスにロードされうるため、
//! - `Local\\rakukan.mode` の共有メモリ（AtomicU64、v2）へ現在モードを書き込み
//! - `Local\\rakukan.mode.changed` のイベントを SetEvent
//!   という *最小IPC* でトレイアプリに通知する。
//!
//! 値フォーマット：
//! - bit32..63 : 発信元プロセス ID（前面アプリの状態だけを適用する）
//! - bit8    : open (1=IME オン, 0=IME オフ)

use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
            PAGE_READWRITE, UnmapViewOfFile,
        },
        Threading::{CreateEventW, GetCurrentProcessId, SetEvent},
    },
};

use crate::engine::ime_mode::ImeMode;

const MAP_NAME: &str = "Local\\rakukan.mode.v2";
const EVT_NAME: &str = "Local\\rakukan.mode.v2.changed";

#[derive(Copy, Clone)]
struct TrayIpc {
    map: HANDLE,
    evt: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

unsafe impl Send for TrayIpc {}
unsafe impl Sync for TrayIpc {}

static IPC: OnceLock<TrayIpc> = OnceLock::new();

fn to_wide_z(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

fn encode(mode: ImeMode) -> u64 {
    (u64::from(unsafe { GetCurrentProcessId() }) << 32) | ((mode.is_on() as u64) << 8)
}

/// 共有メモリとイベントを初期化する。
/// 失敗しても IME 本体は動作継続するため、Result は握りつぶしやすい。
pub fn init() -> windows::core::Result<()> {
    if IPC.get().is_some() {
        return Ok(());
    }

    unsafe {
        let map_name = to_wide_z(MAP_NAME);
        let evt_name = to_wide_z(EVT_NAME);

        let map = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            0,
            8,
            windows::core::PCWSTR(map_name.as_ptr()),
        )?;

        let view = MapViewOfFile(map, FILE_MAP_WRITE, 0, 0, 8);
        if view.Value.is_null() {
            let _ = CloseHandle(map);
            return Err(windows::core::Error::from_win32());
        }

        let evt = CreateEventW(
            None,
            false, // auto-reset
            false,
            windows::core::PCWSTR(evt_name.as_ptr()),
        )?;

        let ipc = TrayIpc { map, evt, view };
        let _ = IPC.set(ipc);
    }
    Ok(())
}

/// 現在の IME オン/オフを共有し、トレイへ通知する。
pub fn publish(mode: ImeMode) {
    // Background applications must not overwrite the foreground mode snapshot.
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        if pid != GetCurrentProcessId() {
            return;
        }
    }
    let _ = init();
    let Some(ipc) = IPC.get().copied() else {
        return;
    };
    unsafe {
        (&*(ipc.view.Value as *const AtomicU64)).store(encode(mode), Ordering::SeqCst);
        let _ = SetEvent(ipc.evt);
    }
}

/// Stop applying rakukan's width when its TSF service is deactivated.
pub fn clear() {
    if let Some(ipc) = IPC.get() {
        unsafe {
            let value = &*(ipc.view.Value as *const AtomicU64);
            let current = value.load(Ordering::SeqCst);
            if (current >> 32) as u32 == GetCurrentProcessId() {
                let _ = value.compare_exchange(current, 0, Ordering::SeqCst, Ordering::SeqCst);
                let _ = SetEvent(ipc.evt);
            }
        }
    }
}

/// 明示的に解放したい場合（通常は不要）。
#[allow(dead_code)]
pub fn shutdown() {
    if let Some(ipc) = IPC.get().copied() {
        unsafe {
            let _ = UnmapViewOfFile(ipc.view);
            let _ = CloseHandle(ipc.evt);
            let _ = CloseHandle(ipc.map);
        }
    }
}
