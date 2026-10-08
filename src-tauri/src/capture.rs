//! 画面を撮る (Windows)。GDI で仮想デスクトップ全体を 1 枚の BGRA にする。
//!
//! 座標はすべて物理ピクセル。アプリは「モニタごとの DPI 対応」で動く (Tauri のマニフェスト) ので、
//! GetSystemMetrics などの値は、拡大率でごまかされない実際のピクセルになる。
//! Windows 以外では、型チェックが通るように、空の代役を置く。

use shotkey_core::{geom::Rect, pixels::Bgra};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows::Win32::Foundation::{BOOL, LPARAM, POINT, RECT, TRUE};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::*;
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    use windows::Win32::UI::WindowsAndMessaging::*;

    /// すべてのモニタの範囲 (仮想デスクトップ座標)
    pub fn monitors() -> Vec<Rect> {
        unsafe extern "system" fn cb(_m: HMONITOR, _dc: HDC, r: *mut RECT, data: LPARAM) -> BOOL {
            let list = &mut *(data.0 as *mut Vec<Rect>);
            let r = &*r;
            list.push(Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top));
            TRUE
        }
        let mut list: Vec<Rect> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut list as *mut Vec<Rect> as isize));
        }
        list
    }

    /// 仮想デスクトップ全体を撮る
    pub fn grab_desktop() -> Result<Bgra, String> {
        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if w <= 0 || h <= 0 {
                return Err("画面の大きさを取得できませんでした".into());
            }
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("GetDC に失敗しました".into());
            }
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp);
            // CAPTUREBLT: 半透明のウィンドウ (右クリックメニューの影など) も入れる
            let blt = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);

            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // 負 = 上の行から
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut data = vec![0u8; (w as usize) * (h as usize) * 4];
            let lines = GetDIBits(mem, bmp, 0, h as u32, Some(data.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);

            SelectObject(mem, old);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);

            if blt.is_err() {
                return Err("BitBlt に失敗しました (保護された画面の可能性があります)".into());
            }
            if lines == 0 {
                return Err("GetDIBits に失敗しました".into());
            }
            Ok(Bgra { origin: (x, y), w, h, data })
        }
    }

    /// 点 (x, y) があるモニタのハンドル (HMONITOR)。windows-capture に渡す。
    pub fn monitor_handle_at(x: i32, y: i32) -> Option<usize> {
        unsafe {
            let h = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL);
            if h.is_invalid() { None } else { Some(h.0 as usize) }
        }
    }

    pub fn cursor_pos() -> Option<(i32, i32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p).ok()?; }
        Some((p.x, p.y))
    }

    /// いま手前にあるウィンドウの範囲。影の分は含めない (見えている枠だけ)。
    pub fn foreground_window() -> Option<Rect> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() || hwnd == GetShellWindow() {
                return None;
            }
            let mut r = RECT::default();
            let ok = DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut RECT as *mut c_void,
                size_of::<RECT>() as u32,
            )
            .is_ok();
            if !ok {
                GetWindowRect(hwnd, &mut r).ok()?;
            }
            let rect = Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top);
            if rect.is_empty() { None } else { Some(rect) }
        }
    }

    // 撮影音。WAV を実行ファイルに埋め込んで、メモリから鳴らす (ファイルを探さない)。
    // SND_ASYNC: 鳴らし終わるのを待たない。SND_NODEFAULT: 失敗したときに、Windows の警告音を鳴らさない。
    static SHUTTER: &[u8] = include_bytes!("../assets/shutter.wav");

    pub fn play_shutter() {
        unsafe {
            let _ = PlaySoundW(PCWSTR(SHUTTER.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn monitors() -> Vec<Rect> { Vec::new() }
    pub fn grab_desktop() -> Result<Bgra, String> { Err("Windows 専用".into()) }
    pub fn monitor_handle_at(_x: i32, _y: i32) -> Option<usize> { None }
    pub fn cursor_pos() -> Option<(i32, i32)> { None }
    pub fn foreground_window() -> Option<Rect> { None }
    pub fn play_shutter() {}
}

pub use imp::*;
