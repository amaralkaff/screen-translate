use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::clipboard::SelectionPos;
use super::MouseEvent;

static MOUSE_UP_FLAG: AtomicBool = AtomicBool::new(false);
static MOUSE_CLICK_FLAG: AtomicBool = AtomicBool::new(false);
static MOUSE_DOWN_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_DOWN_Y: AtomicI32 = AtomicI32::new(0);
static MOUSE_UP_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_UP_Y: AtomicI32 = AtomicI32::new(0);
static POPUP_RECT_LEFT: AtomicI32 = AtomicI32::new(0);
static POPUP_RECT_TOP: AtomicI32 = AtomicI32::new(0);
static POPUP_RECT_RIGHT: AtomicI32 = AtomicI32::new(0);
static POPUP_RECT_BOTTOM: AtomicI32 = AtomicI32::new(0);

#[repr(C)]
#[allow(clippy::upper_case_acronyms)]
struct MSLLHOOKSTRUCT {
    pt: POINT,
    mouse_data: u32,
    _flags: u32,
    _time: u32,
    _extra_info: usize,
}

// base sizes at 96 DPI, scaled by dpi_scale
const BASE_PADDING: i32 = 16;
const BASE_MAX_WIDTH: i32 = 640;
const BASE_FONT_TRANSLATED: i32 = -16;
const BASE_CORNER_RADIUS: i32 = 22;
const BASE_GAP_ABOVE: i32 = 8;
const BASE_SLIDE_PX: i32 = 10;
const BASE_MIN_WIDTH: i32 = 200;
const BASE_MAX_HEIGHT: i32 = 400;
const BASE_SCROLL_LINE: i32 = 40;

const BG_COLOR: u32 = 0x002A2A2A;
const BORDER_SHADOW: u32 = 0x00181818;
const BORDER_HIGHLIGHT: u32 = 0x00606060;
const BORDER_HIGHLIGHT_INNER: u32 = 0x00404040;
const TRANSLATED_COLOR: u32 = 0x00F0F0F0;
const SCROLLBAR_COLOR: u32 = 0x00808080;
const SCROLLBAR_WIDTH: i32 = 4;

const MAX_ALPHA: u8 = 230;
const FADE_IN_MS: f64 = 180.0;
const FADE_OUT_MS: f64 = 220.0;
const FADE_OUT_DESELECT_MS: f64 = 120.0;
const ANIM_TIMER: usize = 100;
const ANIM_FRAME_MS: u32 = 16;
const HIDE_TIMER: usize = 101;

const PHASE_NONE: u8 = 0;
const PHASE_FADE_IN: u8 = 1;
const PHASE_VISIBLE: u8 = 2;
const PHASE_FADE_OUT: u8 = 3;

const WM_POPUP_SCROLL: u32 = WM_USER + 1;

// popup state (main thread only)
static CLASS_NAME: OnceLock<Vec<u16>> = OnceLock::new();
static mut POPUP_HWND: HWND = ptr::null_mut();
static mut TRANSLATED_TEXT: Option<String> = None;
static mut PHASE: u8 = PHASE_NONE;
static mut ANIM_START: Option<Instant> = None;
static mut TARGET_X: i32 = 0;
static mut TARGET_Y: i32 = 0;
static mut CLOSE_SCHEDULED: bool = false;
static mut DPI_SCALE: f64 = 1.0;
static mut DESELECT_CLOSE: bool = false;
static mut SCROLL_OFFSET: i32 = 0;
static mut CONTENT_HEIGHT: i32 = 0;

fn s(v: i32) -> i32 {
    unsafe { (v as f64 * DPI_SCALE).round() as i32 }
}

fn update_popup_rect_cache() {
    unsafe {
        if !POPUP_HWND.is_null() {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(POPUP_HWND, &mut r);
            POPUP_RECT_LEFT.store(r.left, Ordering::Relaxed);
            POPUP_RECT_TOP.store(r.top, Ordering::Relaxed);
            POPUP_RECT_RIGHT.store(r.right, Ordering::Relaxed);
            POPUP_RECT_BOTTOM.store(r.bottom, Ordering::Relaxed);
        }
    }
}

pub struct HookHandle {
    hook: HHOOK,
}

impl Drop for HookHandle {
    fn drop(&mut self) {
        unsafe { UnhookWindowsHookEx(self.hook); }
    }
}

pub fn init_platform() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }

    let class_name = to_wide("ClipTransPopup");
    CLASS_NAME.get_or_init(|| class_name.clone());

    unsafe {
        let hdc = GetDC(ptr::null_mut());
        let dpi = GetDeviceCaps(hdc, LOGPIXELSX as i32);
        ReleaseDC(ptr::null_mut(), hdc);
        DPI_SCALE = dpi as f64 / 96.0;
        tracing::info!("DPI: {} (scale: {:.0}%)", dpi, DPI_SCALE * 100.0);

        let hi = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hi,
            hIcon: ptr::null_mut(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: ptr::null_mut(),
        };
        RegisterClassExW(&wc);
    }
}

pub fn install_mouse_hook() -> anyhow::Result<HookHandle> {
    let hook = unsafe {
        SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), ptr::null_mut(), 0)
    };
    if hook.is_null() {
        anyhow::bail!("Failed to install mouse hook");
    }
    tracing::info!("Mouse hook installed — ready!");
    Ok(HookHandle { hook })
}

pub fn poll_mouse_event() -> Option<MouseEvent> {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT {
                return Some(MouseEvent::Quit);
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    if MOUSE_UP_FLAG.swap(false, Ordering::Relaxed) {
        return Some(MouseEvent::SelectionDone {
            down_x: MOUSE_DOWN_X.load(Ordering::Relaxed),
            down_y: MOUSE_DOWN_Y.load(Ordering::Relaxed),
            up_x: MOUSE_UP_X.load(Ordering::Relaxed),
            up_y: MOUSE_UP_Y.load(Ordering::Relaxed),
        });
    }

    if MOUSE_CLICK_FLAG.swap(false, Ordering::Relaxed) {
        return Some(MouseEvent::Click);
    }

    None
}

pub fn get_double_click_time_ms() -> u64 {
    (unsafe { GetDoubleClickTime() }) as u64
}

pub fn send_copy_command() {
    let mut inputs: [INPUT; 4] = unsafe { std::mem::zeroed() };

    inputs[0].r#type = INPUT_KEYBOARD;
    inputs[0].Anonymous.ki.wVk = VK_CONTROL;

    inputs[1].r#type = INPUT_KEYBOARD;
    inputs[1].Anonymous.ki.wVk = VK_C;

    inputs[2].r#type = INPUT_KEYBOARD;
    inputs[2].Anonymous.ki.wVk = VK_C;
    inputs[2].Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;

    inputs[3].r#type = INPUT_KEYBOARD;
    inputs[3].Anonymous.ki.wVk = VK_CONTROL;
    inputs[3].Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;

    unsafe {
        SendInput(4, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn show_popup(
    _original: &str,
    translated: &str,
    _duration_secs: u64,
    pos: SelectionPos,
) {
    unsafe {
        destroy_popup();

        TRANSLATED_TEXT = Some(translated.into());

        let hi = GetModuleHandleW(ptr::null());
        let cls = CLASS_NAME.get().unwrap();

        let padding = s(BASE_PADDING);
        let max_w = s(BASE_MAX_WIDTH);
        let min_w = s(BASE_MIN_WIDTH);
        let gap_above = s(BASE_GAP_ABOVE);
        let corner_r = s(BASE_CORNER_RADIUS);
        let slide_px = s(BASE_SLIDE_PX);

        let hdc = GetDC(ptr::null_mut());
        let cw = max_w - padding * 2;

        let h_trans = measure_text(hdc, translated, s(BASE_FONT_TRANSLATED), true, cw);
        ReleaseDC(ptr::null_mut(), hdc);

        let w = (cw + padding * 2).max(min_w);
        let full_h = padding + h_trans + padding;

        let sel_top = pos.down_y.min(pos.up_y);
        let sel_bottom = pos.down_y.max(pos.up_y);
        let sel_center_x = (pos.down_x + pos.up_x) / 2;

        // Use the monitor where the selection center is located
        let center_pt = POINT { x: sel_center_x, y: (sel_top + sel_bottom) / 2 };
        let hmon = MonitorFromPoint(center_pt, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(hmon, &mut mi);
        let work = mi.rcWork;
        let mon_left = work.left;
        let mon_top = work.top;
        let mon_right = work.right;
        let mon_bottom = work.bottom;

        let max_h = s(BASE_MAX_HEIGHT).min((mon_bottom - mon_top) * 3 / 5);
        let h = full_h.min(max_h);
        CONTENT_HEIGHT = full_h;
        SCROLL_OFFSET = 0;

        let mut x = sel_center_x - w / 2;
        let above = sel_top - h - gap_above >= mon_top + 4;
        let mut y = if above {
            sel_top - h - gap_above
        } else {
            sel_bottom + gap_above
        };

        if x + w > mon_right - 4 { x = mon_right - w - 4; }
        if x < mon_left + 4 { x = mon_left + 4; }
        if y + h > mon_bottom - 4 { y = mon_bottom - h - 4; }
        if y < mon_top + 4 { y = mon_top + 4; }

        TARGET_X = x;
        TARGET_Y = y;
        let start_y = if above { y + slide_px } else { y - slide_px };

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            cls.as_ptr(),
            ptr::null(),
            WS_POPUP,
            x, start_y, w, h,
            ptr::null_mut(),
            ptr::null_mut(),
            hi,
            ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }

        let rgn = CreateRoundRectRgn(0, 0, w, h, corner_r * 2, corner_r * 2);
        SetWindowRgn(hwnd, rgn, 0);

        SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);

        POPUP_HWND = hwnd;
        PHASE = PHASE_FADE_IN;
        ANIM_START = Some(Instant::now());

        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        update_popup_rect_cache();

        SetTimer(hwnd, ANIM_TIMER, ANIM_FRAME_MS, None);

        // auto-hide: ~15 chars/sec reading speed, 2-20s range
        let total_chars = translated.chars().count();
        let reading_secs = (total_chars as f64 / 15.0).clamp(2.0, 20.0);
        let reading_ms = (reading_secs * 1000.0) as u32;
        let auto_hide_ms = FADE_IN_MS as u32 + reading_ms + 3000;
        SetTimer(hwnd, HIDE_TIMER, auto_hide_ms, None);
    }
}

pub fn on_click_away() {
    unsafe {
        if POPUP_HWND.is_null() || PHASE == PHASE_FADE_OUT || CLOSE_SCHEDULED {
            return;
        }
        begin_fade_out(POPUP_HWND, true);
    }
}

unsafe extern "system" fn mouse_hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let info = &*(lparam as *const MSLLHOOKSTRUCT);
        match wparam as u32 {
            WM_LBUTTONDOWN => {
                MOUSE_DOWN_X.store(info.pt.x, Ordering::Relaxed);
                MOUSE_DOWN_Y.store(info.pt.y, Ordering::Relaxed);
                MOUSE_CLICK_FLAG.store(true, Ordering::Relaxed);
            }
            WM_LBUTTONUP => {
                MOUSE_UP_X.store(info.pt.x, Ordering::Relaxed);
                MOUSE_UP_Y.store(info.pt.y, Ordering::Relaxed);
                MOUSE_UP_FLAG.store(true, Ordering::Relaxed);
            }
            WM_MOUSEWHEEL => {
                if !POPUP_HWND.is_null() && CONTENT_HEIGHT > 0 {
                    let left = POPUP_RECT_LEFT.load(Ordering::Relaxed);
                    let top = POPUP_RECT_TOP.load(Ordering::Relaxed);
                    let right = POPUP_RECT_RIGHT.load(Ordering::Relaxed);
                    let bottom = POPUP_RECT_BOTTOM.load(Ordering::Relaxed);
                    if info.pt.x >= left && info.pt.x < right
                        && info.pt.y >= top && info.pt.y < bottom
                    {
                        let delta = (info.mouse_data >> 16) as i16 as isize;
                        PostMessageW(POPUP_HWND, WM_POPUP_SCROLL, delta as usize, 0);
                        return 1; // consume so background doesn't scroll
                    }
                }
            }
            _ => {}
        }
    }
    CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
}

unsafe fn anim_tick(hwnd: HWND) {
    let elapsed = match ANIM_START {
        Some(s) => s.elapsed().as_secs_f64() * 1000.0,
        None => return,
    };

    let slide_px = s(BASE_SLIDE_PX);

    match PHASE {
        PHASE_FADE_IN => {
            let t = (elapsed / FADE_IN_MS).min(1.0);
            let ease = ease_out_cubic(t);

            let alpha = (ease * MAX_ALPHA as f64) as u8;
            SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);

            let offset = ((1.0 - ease) * slide_px as f64) as i32;
            SetWindowPos(
                hwnd, ptr::null_mut(),
                TARGET_X, TARGET_Y + offset, 0, 0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
            );
            InvalidateRect(hwnd, ptr::null(), 0);

            if t >= 1.0 {
                PHASE = PHASE_VISIBLE;
                SetLayeredWindowAttributes(hwnd, 0, MAX_ALPHA, LWA_ALPHA);
                SetWindowPos(
                    hwnd, ptr::null_mut(),
                    TARGET_X, TARGET_Y, 0, 0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        PHASE_FADE_OUT => {
            let fade_duration = if DESELECT_CLOSE { FADE_OUT_DESELECT_MS } else { FADE_OUT_MS };
            let t = (elapsed / fade_duration).min(1.0);
            let ease = ease_in_cubic(t);

            let alpha = ((1.0 - ease) * MAX_ALPHA as f64) as u8;
            SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);

            let offset = (ease * (slide_px / 2) as f64) as i32;
            SetWindowPos(
                hwnd, ptr::null_mut(),
                TARGET_X, TARGET_Y - offset, 0, 0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
            );

            if t >= 1.0 {
                PHASE = PHASE_NONE;
                KillTimer(hwnd, ANIM_TIMER);
                destroy_popup();
            }
        }
        PHASE_VISIBLE => {}
        _ => {
            KillTimer(hwnd, ANIM_TIMER);
        }
    }
}

unsafe fn begin_fade_out(hwnd: HWND, is_deselect: bool) {
    if PHASE == PHASE_FADE_OUT || PHASE == PHASE_NONE {
        return;
    }
    PHASE = PHASE_FADE_OUT;
    DESELECT_CLOSE = is_deselect;
    ANIM_START = Some(Instant::now());
    KillTimer(hwnd, HIDE_TIMER);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_TIMER => {
            match wp {
                ANIM_TIMER => anim_tick(hwnd),
                HIDE_TIMER => begin_fade_out(hwnd, false),
                _ => {}
            }
            0
        }
        WM_LBUTTONDOWN => {
            begin_fade_out(hwnd, true);
            0
        }
        WM_POPUP_SCROLL => {
            let delta = wp as i16 as i32;
            let scroll_step = s(BASE_SCROLL_LINE);
            let pixels = -(delta as f64 / 120.0 * scroll_step as f64) as i32;
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let max_scroll = (CONTENT_HEIGHT - rc.bottom).max(0);
            SCROLL_OFFSET = (SCROLL_OFFSET + pixels).clamp(0, max_scroll);
            InvalidateRect(hwnd, ptr::null(), 0);
            if PHASE == PHASE_FADE_OUT {
                PHASE = PHASE_VISIBLE;
                SetLayeredWindowAttributes(hwnd, 0, MAX_ALPHA, LWA_ALPHA);
                SetWindowPos(
                    hwnd, ptr::null_mut(),
                    TARGET_X, TARGET_Y, 0, 0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            KillTimer(hwnd, HIDE_TIMER);
            SetTimer(hwnd, HIDE_TIMER, 8000, None);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    GetClientRect(hwnd, &mut rc);
    let w = rc.right;
    let h = rc.bottom;

    let mem_dc = CreateCompatibleDC(hdc);
    let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
    let old_bmp = SelectObject(mem_dc, mem_bmp);

    let padding = s(BASE_PADDING);

    let bg = CreateSolidBrush(BG_COLOR);
    let fill_rc = RECT { left: 0, top: 0, right: w, bottom: h };
    FillRect(mem_dc, &fill_rc, bg);
    DeleteObject(bg);

    let corner_r = s(BASE_CORNER_RADIUS);
    let null_brush = GetStockObject(NULL_BRUSH);
    let saved_brush = SelectObject(mem_dc, null_brush);

    let pen1 = CreatePen(PS_SOLID, 1, BORDER_SHADOW);
    let saved_pen = SelectObject(mem_dc, pen1);
    RoundRect(mem_dc, 0, 0, w, h, corner_r * 2, corner_r * 2);

    let pen2 = CreatePen(PS_SOLID, 1, BORDER_HIGHLIGHT);
    SelectObject(mem_dc, pen2);
    DeleteObject(pen1);
    RoundRect(mem_dc, 1, 1, w - 1, h - 1, (corner_r - 1) * 2, (corner_r - 1) * 2);

    let pen3 = CreatePen(PS_SOLID, 1, BORDER_HIGHLIGHT_INNER);
    SelectObject(mem_dc, pen3);
    DeleteObject(pen2);
    RoundRect(mem_dc, 2, 2, w - 2, h - 2, (corner_r - 2) * 2, (corner_r - 2) * 2);

    SelectObject(mem_dc, saved_pen);
    DeleteObject(pen3);
    SelectObject(mem_dc, saved_brush);

    SetBkMode(mem_dc, TRANSPARENT as i32);

    let has_scroll = CONTENT_HEIGHT > h;

    #[allow(clippy::deref_addrof)]
    let trans_ref = &*(&raw const TRANSLATED_TEXT);
    if let Some(trans) = trans_ref {
        let text_left = padding + 2;
        let cw = w - text_left * 2;

        let saved = SaveDC(mem_dc);
        IntersectClipRect(mem_dc, text_left, padding, text_left + cw, h - padding);

        let f = create_font(s(BASE_FONT_TRANSLATED), true);
        let old_f = SelectObject(mem_dc, f);
        SetTextColor(mem_dc, TRANSLATED_COLOR);
        let text_top = padding - SCROLL_OFFSET;
        let mut r = RECT { left: text_left, top: text_top, right: text_left + cw, bottom: text_top + CONTENT_HEIGHT };
        DrawTextW(mem_dc, to_wide(trans).as_ptr(), -1, &mut r, DT_WORDBREAK | DT_NOPREFIX);
        SelectObject(mem_dc, old_f);
        DeleteObject(f);

        RestoreDC(mem_dc, saved);
    }

    if has_scroll {
        let track_top = padding;
        let track_h = h - padding * 2;
        let visible_ratio = track_h as f64 / CONTENT_HEIGHT as f64;
        let thumb_h = (visible_ratio * track_h as f64).max(20.0) as i32;
        let max_scroll = CONTENT_HEIGHT - h;
        let scroll_ratio = if max_scroll > 0 { SCROLL_OFFSET as f64 / max_scroll as f64 } else { 0.0 };
        let thumb_y = track_top + (scroll_ratio * (track_h - thumb_h) as f64) as i32;

        let bar_w = s(SCROLLBAR_WIDTH);
        let bar_x = w - bar_w - s(3);

        let brush = CreateSolidBrush(SCROLLBAR_COLOR);
        let old_brush = SelectObject(mem_dc, brush);
        let old_pen = SelectObject(mem_dc, GetStockObject(NULL_PEN));
        RoundRect(mem_dc, bar_x, thumb_y, bar_x + bar_w, thumb_y + thumb_h, bar_w, bar_w);
        SelectObject(mem_dc, old_pen);
        SelectObject(mem_dc, old_brush);
        DeleteObject(brush);
    }

    BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);

    SelectObject(mem_dc, old_bmp);
    DeleteObject(mem_bmp);
    DeleteDC(mem_dc);

    EndPaint(hwnd, &ps);
}

fn destroy_popup() {
    unsafe {
        if !POPUP_HWND.is_null() {
            KillTimer(POPUP_HWND, ANIM_TIMER);
            KillTimer(POPUP_HWND, HIDE_TIMER);
            DestroyWindow(POPUP_HWND);
            POPUP_HWND = ptr::null_mut();
            TRANSLATED_TEXT = None;
            PHASE = PHASE_NONE;
            ANIM_START = None;
            CLOSE_SCHEDULED = false;
            DESELECT_CLOSE = false;
            SCROLL_OFFSET = 0;
            CONTENT_HEIGHT = 0;
        }
    }
}

unsafe fn measure_text(hdc: HDC, text: &str, font_size: i32, bold: bool, max_w: i32) -> i32 {
    let font = create_font(font_size, bold);
    let old = SelectObject(hdc, font);
    let wide = to_wide(text);
    let mut rc = RECT { left: 0, top: 0, right: max_w, bottom: 0 };
    DrawTextW(hdc, wide.as_ptr(), -1, &mut rc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
    SelectObject(hdc, old);
    DeleteObject(font);
    rc.bottom
}

unsafe fn create_font(size: i32, bold: bool) -> HFONT {
    CreateFontW(
        size, 0, 0, 0,
        if bold { FW_SEMIBOLD as i32 } else { FW_NORMAL as i32 },
        0, 0, 0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        DEFAULT_PITCH as u32,
        to_wide("Segoe UI").as_ptr(),
    )
}

fn ease_out_cubic(t: f64) -> f64 {
    let u = 1.0 - t;
    1.0 - u * u * u
}

fn ease_in_cubic(t: f64) -> f64 {
    t * t * t
}

pub fn show_error(title: &str, msg: &str) {
    let wide_title = to_wide(title);
    let wide_msg = to_wide(msg);
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide_msg.as_ptr(),
            wide_title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_TOPMOST,
        );
    }
}

pub fn show_info(title: &str, msg: &str) {
    let wide_title = to_wide(title);
    let wide_msg = to_wide(msg);
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide_msg.as_ptr(),
            wide_title.as_ptr(),
            MB_OK | MB_ICONINFORMATION | MB_TOPMOST,
        );
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
