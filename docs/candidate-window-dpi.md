# Candidate window DPI scaling

`[appearance] candidate_font_height` is a logical pixel height at 96 DPI
(100% Windows display scaling). Its range remains 10–72 and its default remains
17. The old renderer passed this setting directly to GDI, so candidates were too
small in DPI-aware applications on high-DPI displays.

The candidate popup now obtains its effective DPI with `GetDpiForWindow`, after
moving it to the caret's monitor when necessary. Font height is rounded from
`configured_height * dpi / 96`. The existing layout calculation scales row heights,
padding and width limits from that height; GDI measurement and drawing use the
same layout snapshot. Work-area fitting still applies after DPI scaling.

For the default setting, the requested font heights are 17 px at 100%, 26 px at
150%, and 34 px at 200%. A manually enlarged setting used to compensate for the
old behavior may now need to be reduced in Settings.

The popup inherits the application's DPI awareness. No process-wide or thread
awareness is changed in production. `GetDpiForWindow` returns 96 for an unaware
window, system DPI for a system-aware window, and monitor DPI for a per-monitor
window. This lets Windows retain responsibility for compatibility bitmap scaling
without applying the monitor factor twice. See Microsoft's
[GetDpiForWindow documentation](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforwindow).

Repositioning recalculates the layout. External `WM_DPICHANGED` notifications
apply Windows' suggested rectangle and schedule a layout refresh, preserving
candidate data and selection. Synchronous DPI notifications during our own layout
update are guarded against reentry. See
[WM_DPICHANGED](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged).

## Validation

```powershell
cargo test -p rakukan-tsf --release --lib --locked --offline
cargo test -p rakukan-tsf --release --lib --locked --offline candidate_dpi_desktop_regression -- --ignored --nocapture
cargo clippy -p rakukan-tsf --release --lib --locked --offline --no-deps -- -D warnings
cargo build -p rakukan-tsf --release --locked --offline
```

Unit coverage checks 100%, 125%, 150% and 200% scaling, row clearance and
work-area fitting. The explicit desktop test creates hidden popups under unaware,
system-aware and per-monitor-v2 contexts, visits each connected monitor and checks
actual GDI font metrics. On the development PC both monitors reported 192 DPI for
aware windows and 96 DPI for unaware windows; all six combinations passed.

Physical 150%/200% mixed-monitor operation and changing Windows scaling while a
candidate list is visible still need interactive verification. The mode indicator
and language-bar icons are separate renderers and are outside this setting's scope.
