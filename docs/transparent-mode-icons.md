# Transparent taskbar mode icons

The TSF language bar renders `A`, `あ`, and the other mode labels directly to an
HICON; these icons do not come from PNG resources.

Previously, the renderer used an opaque background and a fixed 16×16 bitmap.
It now uses the primary taskbar window's DPI and `GetSystemMetricsForDpi` to
choose the small-icon resolution (24 pixels at 150%, 32 pixels at 200%). If the
taskbar window is unavailable, it falls back to system DPI. Separate taskbars
on monitors with different DPI do not yet receive independently sized icons.

Text is rendered to a monochrome coverage mask at four times the target size,
then downsampled into premultiplied BGRA. Empty pixels are fully transparent;
edge pixels retain partial coverage. Light system themes use black text and
dark themes use white text. A matching monochrome AND mask supports legacy
icon drawing. GDI objects are released after creating the icon, which is owned
by the TSF caller.

Validation: the release TSF suite passes 103 tests (two desktop tests ignored).
The new regression test covers four labels, both themes, and 16/20/24/32/48/64
pixel sizes, checking transparent corners, visible glyphs, partial edge alpha,
and premultiplied color. It also creates and destroys an actual HICON. Rendered
16/24/32 pixel previews were visually inspected against light and dark surfaces.
The live taskbar appearance still requires an application to load the new DLL.

Windows API references:

- [ITfLangBarItemButton::GetIcon](https://learn.microsoft.com/en-us/windows/win32/api/ctfutb/nf-ctfutb-itflangbaritembutton-geticon)
- [CreateIconIndirect](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createiconindirect)
