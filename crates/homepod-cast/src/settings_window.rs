//! A small native Win32 settings window: volume, streaming mode and the global
//! toggle hotkey.
//!
//! It uses the OS built-in controls (`msctls_trackbar32`, `msctls_hotkey32`,
//! `BUTTON`, `STATIC`) rather than a GUI toolkit, so it adds no heavy
//! dependencies and coexists with the tray's own message loop. The window runs
//! on its own thread (Win32 windows are thread-affine) and reports changes back
//! through two callbacks: volume is applied live as the slider moves; the hotkey
//! and streaming mode are reported when the user clicks Save.
//!
//! Three things carry the modern look, and all three have to be there: the
//! application manifest pulls in Common Controls 6 (otherwise every control is
//! drawn in the Windows 95 style), the controls are given Segoe UI instead of
//! the default bitmap font, and the layout is scaled from the system DPI so it
//! stays crisp on high resolution displays.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::UpdateWindow;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, GetWindowLongPtrW, LoadCursorW, LoadIconW, PostQuitMessage, RegisterClassW,
    SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage, IDC_ARROW, MSG,
    WNDCLASSW,
};

use crate::cast::{StreamingMode, MODE_MAX_MS, MODE_MIN_MS};

// --- Win32 constants (kept local to avoid extra windows-sys features) ---
const GWLP_USERDATA: i32 = -21;
const WM_DESTROY: u32 = 0x0002;
const WM_SETFONT: u32 = 0x0030;
const WM_CLOSE: u32 = 0x0010;
const WM_COMMAND: u32 = 0x0111;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_HSCROLL: u32 = 0x0114;

const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_GROUP: u32 = 0x0002_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const SW_SHOW: i32 = 5;
const CW_USEDEFAULT: i32 = i32::MIN; // 0x80000000

// Static / button styles.
const SS_RIGHT: u32 = 0x0000_0002;
const BS_DEFPUSHBUTTON: u32 = 0x0000_0001;
const BS_GROUPBOX: u32 = 0x0000_0007;
const BS_AUTORADIOBUTTON: u32 = 0x0000_0009;

// Button messages.
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: isize = 1;

// Trackbar messages / styles.
const WM_USER: u32 = 0x0400;
const TBM_GETPOS: u32 = WM_USER;
const TBM_SETPOS: u32 = WM_USER + 5;
const TBM_SETRANGE: u32 = WM_USER + 6;
const TBM_SETTICFREQ: u32 = WM_USER + 20;
const TBS_AUTOTICKS: u32 = 0x0001;
const TBS_HORZ: u32 = 0x0000;

// Hotkey control messages / modifier flags (HOTKEYF_*).
const HKM_SETHOTKEY: u32 = WM_USER + 1;
const HKM_GETHOTKEY: u32 = WM_USER + 2;
const HOTKEYF_SHIFT: u32 = 0x01;
const HOTKEYF_CONTROL: u32 = 0x02;
const HOTKEYF_ALT: u32 = 0x04;

// RegisterHotKey modifier flags (MOD_*).
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_NOREPEAT: u32 = 0x4000;

// GDI background mode.
const TRANSPARENT_BK: i32 = 1;

// Colours (COLORREF is 0x00BBGGRR).
const COLOR_WINDOW_BG: u32 = 0x00FF_FFFF; // white
const COLOR_HINT_TEXT: u32 = 0x0070_6B68; // muted grey

const ID_SAVE: usize = 100;
const ID_CLOSE: usize = 101;
const ID_MODE_REALTIME: usize = 110;
const ID_MODE_NORMAL: usize = 111;
const ID_MODE_BUFFERED: usize = 112;
const ID_MODE_CUSTOM: usize = 113;

/// Granularity of the custom latency slider, in milliseconds. The trackbar
/// works in these steps so it has a sane number of positions and tick marks.
const CUSTOM_STEP_MS: u32 = 50;

// Common Controls init (avoids needing the Win32_UI_Controls feature).
#[repr(C)]
struct INITCOMMONCONTROLSEX {
    dw_size: u32,
    dw_icc: u32,
}
const ICC_BAR_CLASSES: u32 = 0x0004;
const ICC_HOTKEY_CLASS: u32 = 0x0040;
const ICC_STANDARD_CLASSES: u32 = 0x4000;
#[link(name = "comctl32")]
extern "system" {
    fn InitCommonControlsEx(picce: *const INITCOMMONCONTROLSEX) -> i32;
}

// Declared here rather than taken from windows-sys so the exact signatures are
// pinned down in one place, the way the comctl32 import above already is.
#[link(name = "gdi32")]
extern "system" {
    #[allow(clippy::too_many_arguments)]
    fn CreateFontW(
        cheight: i32,
        cwidth: i32,
        cescapement: i32,
        corientation: i32,
        cweight: i32,
        bitalic: u32,
        bunderline: u32,
        bstrikeout: u32,
        icharset: u32,
        ioutprecision: u32,
        iclipprecision: u32,
        iquality: u32,
        ipitchandfamily: u32,
        pszfacename: *const u16,
    ) -> *mut c_void;
    fn CreateSolidBrush(color: u32) -> *mut c_void;
    fn DeleteObject(obj: *mut c_void) -> i32;
    fn SetBkMode(hdc: *mut c_void, mode: i32) -> i32;
    fn SetTextColor(hdc: *mut c_void, color: u32) -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn GetDpiForSystem() -> u32;
}

/// Only one settings window at a time.
static OPEN: AtomicBool = AtomicBool::new(false);

/// The window class keeps this brush as its background and the class is never
/// unregistered, so the brush has to outlive every window: it is created once
/// and deliberately never deleted.
static BACKGROUND: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe fn background_brush() -> *mut c_void {
    let existing = BACKGROUND.load(Ordering::Acquire);
    if existing != 0 {
        return existing as *mut c_void;
    }
    let brush = CreateSolidBrush(COLOR_WINDOW_BG);
    match BACKGROUND.compare_exchange(0, brush as usize, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => brush,
        Err(winner) => {
            // Another thread got there first; drop ours rather than leaking it.
            DeleteObject(brush);
            winner as *mut c_void
        }
    }
}

/// State shared with the window procedure (stored behind `GWLP_USERDATA`).
struct WindowState {
    volume_bar: HWND,
    pct_label: HWND,
    custom_bar: HWND,
    custom_label: HWND,
    mode_radios: [HWND; 4],
    hotkey_ctrl: HWND,
    /// Labels drawn in muted grey rather than the normal text colour.
    hints: Vec<HWND>,
    /// Brush returned for control background painting; owned by [`BACKGROUND`].
    background: *mut c_void,
    on_volume: Box<dyn Fn(f32)>,
    on_save: Box<dyn Fn(u32, u32, StreamingMode)>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Translate `RegisterHotKey` MOD_* flags into the hotkey control's HOTKEYF_*.
fn mod_to_hotkeyf(mods: u32) -> u32 {
    let mut f = 0;
    if mods & MOD_ALT != 0 {
        f |= HOTKEYF_ALT;
    }
    if mods & MOD_CONTROL != 0 {
        f |= HOTKEYF_CONTROL;
    }
    if mods & MOD_SHIFT != 0 {
        f |= HOTKEYF_SHIFT;
    }
    f
}

/// Translate the hotkey control's HOTKEYF_* flags back into MOD_* (always with
/// MOD_NOREPEAT so holding the keys doesn't retrigger).
fn hotkeyf_to_mod(f: u32) -> u32 {
    let mut m = MOD_NOREPEAT;
    if f & HOTKEYF_ALT != 0 {
        m |= MOD_ALT;
    }
    if f & HOTKEYF_CONTROL != 0 {
        m |= MOD_CONTROL;
    }
    if f & HOTKEYF_SHIFT != 0 {
        m |= MOD_SHIFT;
    }
    m
}

/// Format a buffer size for the slider readout.
///
/// Plain milliseconds throughout: the slider's own unit, with no locale
/// dependent decimal separator to get wrong.
fn format_latency(ms: u32) -> String {
    format!("{ms} ms")
}

/// Open the settings window (no-op if one is already open).
///
/// `on_volume(0.0..=1.0)` fires live as the slider moves; `on_save(mods, vk,
/// mode)` fires when Save is clicked.
pub fn open(
    initial_volume: f32,
    initial_mods: u32,
    initial_vk: u32,
    initial_mode: StreamingMode,
    on_volume: impl Fn(f32) + Send + 'static,
    on_save: impl Fn(u32, u32, StreamingMode) + Send + 'static,
) {
    if OPEN.swap(true, Ordering::SeqCst) {
        return; // already open
    }
    std::thread::spawn(move || {
        unsafe {
            run_window(
                initial_volume,
                initial_mods,
                initial_vk,
                initial_mode,
                Box::new(on_volume),
                Box::new(on_save),
            );
        }
        OPEN.store(false, Ordering::SeqCst);
    });
}

unsafe fn run_window(
    initial_volume: f32,
    initial_mods: u32,
    initial_vk: u32,
    initial_mode: StreamingMode,
    on_volume: Box<dyn Fn(f32)>,
    on_save: Box<dyn Fn(u32, u32, StreamingMode)>,
) {
    let hinstance = GetModuleHandleW(std::ptr::null());

    let icc = INITCOMMONCONTROLSEX {
        dw_size: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dw_icc: ICC_BAR_CLASSES | ICC_HOTKEY_CLASS | ICC_STANDARD_CLASSES,
    };
    InitCommonControlsEx(&icc);

    // The process is declared system-DPI aware in the manifest, so the layout
    // below (written in 96-dpi units) has to be scaled by hand.
    let dpi = GetDpiForSystem().max(96);
    let sc = move |v: i32| (v as i64 * dpi as i64 / 96) as i32;

    let background = background_brush();
    let face = wide("Segoe UI");
    let font = CreateFontW(
        -((9 * dpi as i32) / 72), // 9pt
        0,
        0,
        0,
        400, // FW_NORMAL
        0,
        0,
        0,
        1, // DEFAULT_CHARSET
        0, // OUT_DEFAULT_PRECIS
        0, // CLIP_DEFAULT_PRECIS
        5, // CLEARTYPE_QUALITY
        0, // DEFAULT_PITCH | FF_DONTCARE
        face.as_ptr(),
    );

    let class_name = wide("VulpiCastSettings");
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: LoadIconW(hinstance, 1 as *const u16),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hbrBackground: background,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    // Re-registering an existing class is harmless; ignore the result.
    RegisterClassW(&wc);

    // Client area in 96-dpi units; grown to a window size including the frame.
    const CLIENT_W: i32 = 400;
    const CLIENT_H: i32 = 460;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: sc(CLIENT_W),
        bottom: sc(CLIENT_H),
    };
    AdjustWindowRect(&mut rect, style, 0);

    let title = wide("VulpiCast — Settings");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        style,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        rect.right - rect.left,
        rect.bottom - rect.top,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        DeleteObject(font);
        return;
    }

    let child = |class: &[u16], text: &[u16], style: u32, x, y, w, h, id: usize| -> HWND {
        let control = CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            sc(x),
            sc(y),
            sc(w),
            sc(h),
            hwnd,
            id as *mut c_void,
            hinstance,
            std::ptr::null(),
        );
        // Without this every control falls back to the default bitmap font.
        SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
        control
    };

    let static_cls = wide("STATIC");
    let button_cls = wide("BUTTON");
    let trackbar_cls = wide("msctls_trackbar32");
    let hotkey_cls = wide("msctls_hotkey32");
    let empty = wide("");

    // --- Volume ---
    child(
        &button_cls,
        &wide("Volume"),
        BS_GROUPBOX,
        12,
        10,
        376,
        72,
        0,
    );
    let volume_bar = child(
        &trackbar_cls,
        &empty,
        WS_TABSTOP | TBS_AUTOTICKS | TBS_HORZ,
        24,
        34,
        296,
        30,
        0,
    );
    let pct_label = child(&static_cls, &empty, SS_RIGHT, 324, 40, 52, 20, 0);

    // --- Streaming mode ---
    child(
        &button_cls,
        &wide("Streaming mode"),
        BS_GROUPBOX,
        12,
        92,
        376,
        196,
        0,
    );
    let mode_hint = child(
        &static_cls,
        &wide("Trade connection stability against latency."),
        0,
        26,
        112,
        344,
        18,
        0,
    );
    let radio_realtime = child(
        &button_cls,
        &wide("Real-time — lowest latency (about 200 ms buffer)"),
        WS_TABSTOP | WS_GROUP | BS_AUTORADIOBUTTON,
        26,
        136,
        344,
        22,
        ID_MODE_REALTIME,
    );
    let radio_normal = child(
        &button_cls,
        &wide("Normal — recommended (about 1 s buffer)"),
        WS_TABSTOP | BS_AUTORADIOBUTTON,
        26,
        160,
        344,
        22,
        ID_MODE_NORMAL,
    );
    let radio_buffered = child(
        &button_cls,
        &wide("Buffered — most stable connection (about 2.5 s buffer)"),
        WS_TABSTOP | BS_AUTORADIOBUTTON,
        26,
        184,
        344,
        22,
        ID_MODE_BUFFERED,
    );
    let radio_custom = child(
        &button_cls,
        &wide("Custom buffer size:"),
        WS_TABSTOP | BS_AUTORADIOBUTTON,
        26,
        208,
        344,
        22,
        ID_MODE_CUSTOM,
    );
    let custom_bar = child(
        &trackbar_cls,
        &empty,
        WS_TABSTOP | WS_GROUP | TBS_AUTOTICKS | TBS_HORZ,
        42,
        234,
        280,
        30,
        0,
    );
    let custom_label = child(&static_cls, &empty, SS_RIGHT, 324, 240, 52, 20, 0);

    // --- Hotkey ---
    child(
        &button_cls,
        &wide("Keyboard shortcut"),
        BS_GROUPBOX,
        12,
        300,
        376,
        66,
        0,
    );
    child(
        &static_cls,
        &wide("Toggle streaming:"),
        0,
        26,
        324,
        140,
        20,
        0,
    );
    let hotkey_ctrl = child(
        &hotkey_cls,
        &empty,
        WS_TABSTOP | WS_BORDER,
        170,
        321,
        200,
        24,
        0,
    );

    // --- Footer ---
    let footer_hint = child(
        &static_cls,
        &wide(
            "Changing the buffer restarts a running stream. The target \
             device adds a fixed delay of its own.",
        ),
        0,
        14,
        376,
        372,
        34,
        0,
    );
    child(
        &button_cls,
        &wide("Save"),
        WS_TABSTOP | BS_DEFPUSHBUTTON,
        188,
        418,
        96,
        30,
        ID_SAVE,
    );
    child(
        &button_cls,
        &wide("Close"),
        WS_TABSTOP,
        292,
        418,
        96,
        30,
        ID_CLOSE,
    );

    // Volume slider: 0–100 with a tick every 10%.
    SendMessageW(volume_bar, TBM_SETRANGE, 1, make_lparam(0, 100));
    SendMessageW(volume_bar, TBM_SETTICFREQ, 10, 0);
    let pos = (initial_volume.clamp(0.0, 1.0) * 100.0).round() as i32;
    SendMessageW(volume_bar, TBM_SETPOS, 1, pos as LPARAM);
    set_text(pct_label, &format!("{pos} %"));

    // Custom latency slider works in CUSTOM_STEP_MS steps so it has a usable
    // number of positions; a tick every 500ms.
    let steps_min = (MODE_MIN_MS / CUSTOM_STEP_MS).max(1) as i32;
    let steps_max = (MODE_MAX_MS / CUSTOM_STEP_MS) as i32;
    SendMessageW(
        custom_bar,
        TBM_SETRANGE,
        1,
        make_lparam(steps_min, steps_max),
    );
    SendMessageW(
        custom_bar,
        TBM_SETTICFREQ,
        (500 / CUSTOM_STEP_MS) as WPARAM,
        0,
    );
    let custom_ms = match initial_mode {
        StreamingMode::Custom(ms) => ms,
        // Give the slider a sensible starting point even when a preset is active.
        other => other.latency_ms(),
    };
    let custom_steps = (custom_ms / CUSTOM_STEP_MS).clamp(steps_min as u32, steps_max as u32);
    SendMessageW(custom_bar, TBM_SETPOS, 1, custom_steps as LPARAM);
    set_text(custom_label, &format_latency(custom_steps * CUSTOM_STEP_MS));

    let mode_radios = [radio_realtime, radio_normal, radio_buffered, radio_custom];
    let selected = match initial_mode {
        StreamingMode::RealTime => 0,
        StreamingMode::Normal => 1,
        StreamingMode::Buffered => 2,
        StreamingMode::Custom(_) => 3,
    };
    SendMessageW(mode_radios[selected], BM_SETCHECK, BST_CHECKED as WPARAM, 0);
    EnableWindow(custom_bar, i32::from(selected == 3));

    // Initialise the hotkey control: LOWORD = vk, next byte = HOTKEYF flags.
    let hk_word = (initial_vk & 0xff) | (mod_to_hotkeyf(initial_mods) << 8);
    SendMessageW(hotkey_ctrl, HKM_SETHOTKEY, hk_word as WPARAM, 0);

    let state = Box::new(WindowState {
        volume_bar,
        pct_label,
        custom_bar,
        custom_label,
        mode_radios,
        hotkey_ctrl,
        hints: vec![mode_hint, footer_hint],
        background,
        on_volume,
        on_save,
    });
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    // Every control that referenced this font is gone by now.
    DeleteObject(font);
}

/// Pack two 16-bit values into an LPARAM (low, high).
fn make_lparam(low: i32, high: i32) -> LPARAM {
    (((high as u32) << 16) | (low as u32 & 0xffff)) as LPARAM
}

unsafe fn set_text(control: HWND, text: &str) {
    let text = wide(text);
    SetWindowTextW(control, text.as_ptr());
}

/// Which streaming mode the radio buttons currently describe.
unsafe fn selected_mode(state: &WindowState) -> StreamingMode {
    let checked = |i: usize| SendMessageW(state.mode_radios[i], BM_GETCHECK, 0, 0) == BST_CHECKED;
    if checked(0) {
        StreamingMode::RealTime
    } else if checked(2) {
        StreamingMode::Buffered
    } else if checked(3) {
        let steps = SendMessageW(state.custom_bar, TBM_GETPOS, 0, 0) as u32;
        StreamingMode::Custom((steps * CUSTOM_STEP_MS).clamp(MODE_MIN_MS, MODE_MAX_MS))
    } else {
        StreamingMode::Normal
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;

    match msg {
        // Paint labels, group boxes, radios and sliders on the window's own
        // white background instead of the default grey.
        WM_CTLCOLORSTATIC if !state.is_null() => {
            let s = &*state;
            let hdc = wparam as *mut c_void;
            SetBkMode(hdc, TRANSPARENT_BK);
            let control = lparam as HWND;
            if s.hints.contains(&control) {
                SetTextColor(hdc, COLOR_HINT_TEXT);
            }
            s.background as LRESULT
        }
        WM_HSCROLL if !state.is_null() => {
            let s = &*state;
            let control = lparam as HWND;
            if control == s.volume_bar {
                let pos = SendMessageW(s.volume_bar, TBM_GETPOS, 0, 0) as i32;
                (s.on_volume)((pos as f32 / 100.0).clamp(0.0, 1.0));
                set_text(s.pct_label, &format!("{pos} %"));
            } else if control == s.custom_bar {
                let steps = SendMessageW(s.custom_bar, TBM_GETPOS, 0, 0) as u32;
                set_text(s.custom_label, &format_latency(steps * CUSTOM_STEP_MS));
            }
            0
        }
        WM_COMMAND if !state.is_null() => {
            let s = &*state;
            let id = wparam & 0xffff;
            match id {
                ID_SAVE => {
                    let hk = SendMessageW(s.hotkey_ctrl, HKM_GETHOTKEY, 0, 0) as u32;
                    let vk = hk & 0xff;
                    let flags = (hk >> 8) & 0xff;
                    // vk == 0 means "no key captured": keep the current hotkey by
                    // reporting it as disabled rather than registering nothing.
                    (s.on_save)(hotkeyf_to_mod(flags), vk, selected_mode(s));
                    DestroyWindow(hwnd);
                }
                ID_CLOSE => {
                    DestroyWindow(hwnd);
                }
                ID_MODE_REALTIME | ID_MODE_NORMAL | ID_MODE_BUFFERED | ID_MODE_CUSTOM => {
                    // The custom slider is only meaningful for its own radio.
                    EnableWindow(s.custom_bar, i32::from(id == ID_MODE_CUSTOM));
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            if !state.is_null() {
                drop(Box::from_raw(state));
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
