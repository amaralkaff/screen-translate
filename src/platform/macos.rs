use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{msg_send, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertStyle, NSApplication, NSBackingStoreType, NSColor, NSEvent,
    NSEventMask, NSFont, NSPanel, NSScreen, NSTextField, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSPoint, NSRect, NSSize, NSString,
};

use crate::clipboard::SelectionPos;
use super::MouseEvent;

// ---------------------------------------------------------------------------
// CoreGraphics / CoreFoundation FFI
// ---------------------------------------------------------------------------

type CGEventTapProxy = *mut c_void;
type CGEventRef = *mut c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFStringRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CGEventMask = u64;
type CGEventType = u32;
type CGEventFlags = u64;
type CGKeyCode = u16;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

const K_CG_HID_EVENT_TAP: u32 = 0; // kCGHIDEventTap
const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0; // kCGHeadInsertEventTap
const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;

const K_CG_EVENT_LEFT_MOUSE_DOWN: CGEventType = 1;
const K_CG_EVENT_LEFT_MOUSE_UP: CGEventType = 2;
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFFFFFE;

const K_CG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 1 << 20;

const KEYCODE_C: CGKeyCode = 8;

type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;

    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);

    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;

    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        virtual_key: CGKeyCode,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);
    fn CGEventPost(tap: u32, event: CGEventRef);

    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: i64,
    ) -> CFRunLoopSourceRef;
    fn CFMachPortInvalidate(port: CFMachPortRef);

    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRemoveSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after: bool) -> i32;

    fn CFRelease(cf: *mut c_void);

    static kCFRunLoopDefaultMode: CFStringRef;

    fn CGWindowLevelForKey(key: i32) -> i32;
}

const K_CG_FLOATING_WINDOW_LEVEL_KEY: i32 = 5;

// ---------------------------------------------------------------------------
// Atomic state (same pattern as Windows)
// ---------------------------------------------------------------------------

static MOUSE_UP_FLAG: AtomicBool = AtomicBool::new(false);
static MOUSE_CLICK_FLAG: AtomicBool = AtomicBool::new(false);
static MOUSE_DOWN_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_DOWN_Y: AtomicI32 = AtomicI32::new(0);
static MOUSE_UP_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_UP_Y: AtomicI32 = AtomicI32::new(0);

// Store tap ref for re-enabling on timeout (AtomicPtr is Send+Sync)
static TAP_REF: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// ---------------------------------------------------------------------------
// Popup state (main thread only)
// ---------------------------------------------------------------------------

const PHASE_NONE: u8 = 0;
const PHASE_FADE_IN: u8 = 1;
const PHASE_VISIBLE: u8 = 2;
const PHASE_FADE_OUT: u8 = 3;

const MAX_ALPHA: f64 = 0.92;
const FADE_IN_MS: f64 = 180.0;
const FADE_OUT_MS: f64 = 220.0;
const FADE_OUT_DESELECT_MS: f64 = 120.0;
const SLIDE_PX: f64 = 10.0;
const PADDING: f64 = 16.0;
const MAX_WIDTH: f64 = 640.0;
const MIN_WIDTH: f64 = 200.0;
const GAP_ABOVE: f64 = 8.0;
const CORNER_RADIUS: f64 = 22.0;
const FONT_SIZE: f64 = 14.0;
const MARGIN: f64 = 4.0;

static mut POPUP_PANEL: Option<Retained<NSPanel>> = None;
static mut PHASE: u8 = PHASE_NONE;
static mut ANIM_START: Option<Instant> = None;
static mut TARGET_Y: f64 = 0.0;
static mut DESELECT_CLOSE: bool = false;
static mut AUTO_HIDE_DEADLINE: Option<Instant> = None;
static mut POSITIONED_ABOVE: bool = true;

// ---------------------------------------------------------------------------
// HookHandle (RAII)
// ---------------------------------------------------------------------------

pub struct HookHandle {
    tap: CFMachPortRef,
    source: CFRunLoopSourceRef,
    run_loop: CFRunLoopRef,
}

impl Drop for HookHandle {
    fn drop(&mut self) {
        unsafe {
            CFRunLoopRemoveSource(self.run_loop, self.source, kCFRunLoopDefaultMode);
            CFMachPortInvalidate(self.tap);
            CFRelease(self.source);
            CFRelease(self.tap);
        }
    }
}

// ---------------------------------------------------------------------------
// init_platform
// ---------------------------------------------------------------------------

pub fn init_platform() {
    // Initialize NSApplication BEFORE creating tray icon.
    // Without this, macOS doesn't recognize the process as a GUI app
    // and `open -a` will fail to launch it (process exits immediately).
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        // LSUIElement=true in Info.plist sets Accessory policy automatically,
        // but we must call setActivationPolicy explicitly when running outside
        // a bundle (e.g. cargo run) or when the bundle isn't fully initialised yet.
        app.setActivationPolicy(
            objc2_app_kit::NSApplicationActivationPolicy::Accessory,
        );
        app.finishLaunching();
    }

    unsafe {
        if !CGPreflightListenEventAccess() {
            tracing::info!("Requesting Input Monitoring permission...");
            CGRequestListenEventAccess();
        }
    }
}

// ---------------------------------------------------------------------------
// install_mouse_hook
// ---------------------------------------------------------------------------

pub fn install_mouse_hook() -> anyhow::Result<HookHandle> {
    unsafe {
        if !CGPreflightListenEventAccess() {
            anyhow::bail!(
                "Input Monitoring permission denied.\n\n\
                 Go to System Settings > Privacy & Security > Input Monitoring\n\
                 and enable this app, then relaunch."
            );
        }

        let events: CGEventMask =
            (1 << K_CG_EVENT_LEFT_MOUSE_DOWN) | (1 << K_CG_EVENT_LEFT_MOUSE_UP);

        let tap = CGEventTapCreate(
            K_CG_HID_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            events,
            mouse_tap_callback,
            std::ptr::null_mut(),
        );
        if tap.is_null() {
            anyhow::bail!("Failed to create CGEventTap — check Input Monitoring permission");
        }

        let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        if source.is_null() {
            CFRelease(tap);
            anyhow::bail!("Failed to create run loop source for event tap");
        }

        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
        CGEventTapEnable(tap, true);

        TAP_REF.store(tap, Ordering::Relaxed);

        tracing::info!("Mouse hook installed — ready!");
        Ok(HookHandle { tap, source, run_loop })
    }
}

// ---------------------------------------------------------------------------
// Mouse tap callback
// ---------------------------------------------------------------------------

unsafe extern "C" fn mouse_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    match event_type {
        K_CG_EVENT_LEFT_MOUSE_DOWN => {
            let loc = CGEventGetLocation(event);
            MOUSE_DOWN_X.store(loc.x as i32, Ordering::Relaxed);
            MOUSE_DOWN_Y.store(loc.y as i32, Ordering::Relaxed);
            MOUSE_CLICK_FLAG.store(true, Ordering::Relaxed);
        }
        K_CG_EVENT_LEFT_MOUSE_UP => {
            let loc = CGEventGetLocation(event);
            MOUSE_UP_X.store(loc.x as i32, Ordering::Relaxed);
            MOUSE_UP_Y.store(loc.y as i32, Ordering::Relaxed);
            MOUSE_UP_FLAG.store(true, Ordering::Relaxed);
        }
        K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT => {
            tracing::warn!("Event tap disabled by timeout, re-enabling");
            let tap = TAP_REF.load(Ordering::Relaxed);
            if !tap.is_null() {
                CGEventTapEnable(tap, true);
            }
        }
        _ => {}
    }
    event
}

// ---------------------------------------------------------------------------
// poll_mouse_event
// ---------------------------------------------------------------------------

pub fn poll_mouse_event() -> Option<MouseEvent> {
    // Pump NSApplication events so tray icon menus work
    if let Some(mtm) = MainThreadMarker::new() {
        pump_app_events(mtm);
    }

    // Drain the run loop so CGEventTap callbacks fire
    unsafe {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, false);
    }

    // Drive popup animation
    animate_popup();

    // Check auto-hide deadline
    unsafe {
        if PHASE == PHASE_VISIBLE {
            if let Some(deadline) = AUTO_HIDE_DEADLINE {
                if Instant::now() >= deadline {
                    begin_fade_out(false);
                }
            }
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

// ---------------------------------------------------------------------------
// get_double_click_time_ms
// ---------------------------------------------------------------------------

pub fn get_double_click_time_ms() -> u64 {
    let interval = NSEvent::doubleClickInterval();
    (interval * 1000.0) as u64
}

// ---------------------------------------------------------------------------
// send_copy_command (Cmd+C)
// ---------------------------------------------------------------------------

pub fn send_copy_command() {
    unsafe {
        // Key down
        let event_down = CGEventCreateKeyboardEvent(std::ptr::null(), KEYCODE_C, true);
        if !event_down.is_null() {
            CGEventSetFlags(event_down, K_CG_EVENT_FLAG_MASK_COMMAND);
            CGEventPost(K_CG_HID_EVENT_TAP, event_down);
            CFRelease(event_down);
        }

        // Key up
        let event_up = CGEventCreateKeyboardEvent(std::ptr::null(), KEYCODE_C, false);
        if !event_up.is_null() {
            CGEventSetFlags(event_up, K_CG_EVENT_FLAG_MASK_COMMAND);
            CGEventPost(K_CG_HID_EVENT_TAP, event_up);
            CFRelease(event_up);
        }
    }
}

// ---------------------------------------------------------------------------
// show_error (NSAlert)
// ---------------------------------------------------------------------------

pub fn show_error(title: &str, msg: &str) {
    if let Some(mtm) = MainThreadMarker::new() {
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Critical);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(msg));
        alert.runModal();
    } else {
        eprintln!("[{}] {}", title, msg);
    }
}

pub fn show_info(title: &str, msg: &str) {
    if let Some(mtm) = MainThreadMarker::new() {
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(msg));
        alert.runModal();
    } else {
        eprintln!("[{}] {}", title, msg);
    }
}

// ---------------------------------------------------------------------------
// show_popup (NSPanel + Liquid Glass / NSVisualEffectView)
// ---------------------------------------------------------------------------

pub fn show_popup(
    _original: &str,
    translated: &str,
    _duration_secs: u64,
    pos: SelectionPos,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("show_popup called off main thread");
        return;
    };

    unsafe {
        destroy_popup();

        // Get primary screen height for Quartz → AppKit coordinate conversion
        let screens = NSScreen::screens(mtm);
        if screens.count() == 0 {
            return;
        }
        let primary: Retained<NSScreen> = screens.objectAtIndex(0);
        let primary_frame = primary.frame();
        let screen_h = primary_frame.size.height;

        // Find the screen containing the selection center (in Quartz coords)
        let sel_top_q = pos.down_y.min(pos.up_y) as f64;
        let sel_bottom_q = pos.down_y.max(pos.up_y) as f64;
        let sel_center_x = (pos.down_x + pos.up_x) as f64 / 2.0;
        let sel_center_y_q = (sel_top_q + sel_bottom_q) / 2.0;

        // Convert selection to AppKit coords
        let sel_top_ak = screen_h - sel_bottom_q;
        let sel_bottom_ak = screen_h - sel_top_q;

        // Find the screen that contains the selection center
        let sel_center_ak = NSPoint::new(sel_center_x, screen_h - sel_center_y_q);
        let mut target_visible = primary.visibleFrame();
        let screen_count = screens.count();
        for i in 0..screen_count {
            let screen: Retained<NSScreen> = screens.objectAtIndex(i);
            let frame = screen.frame();
            if sel_center_ak.x >= frame.origin.x
                && sel_center_ak.x < frame.origin.x + frame.size.width
                && sel_center_ak.y >= frame.origin.y
                && sel_center_ak.y < frame.origin.y + frame.size.height
            {
                target_visible = screen.visibleFrame();
                break;
            }
        }

        // Create the text label to measure its size
        let text_ns = NSString::from_str(translated);
        let label = NSTextField::wrappingLabelWithString(&text_ns, mtm);
        let font = NSFont::systemFontOfSize(FONT_SIZE);
        label.setFont(Some(&font));
        label.setTextColor(Some(&NSColor::labelColor()));

        // Constrain width and measure
        let content_w = (MAX_WIDTH - PADDING * 2.0).max(MIN_WIDTH - PADDING * 2.0);
        label.setPreferredMaxLayoutWidth(content_w);
        let fitting = label.fittingSize();
        let text_w = fitting.width.min(content_w);
        let text_h = fitting.height;

        let panel_w = (text_w + PADDING * 2.0).clamp(MIN_WIDTH, MAX_WIDTH);
        let panel_h = text_h + PADDING * 2.0;

        // Position: prefer above selection, fallback below
        let mut x = sel_center_x - panel_w / 2.0;
        let above = sel_top_ak - panel_h - GAP_ABOVE >= target_visible.origin.y + MARGIN;
        POSITIONED_ABOVE = above;
        let mut y = if above {
            sel_top_ak - panel_h - GAP_ABOVE
        } else {
            sel_bottom_ak + GAP_ABOVE
        };

        // Clamp to visible frame
        let vis_right = target_visible.origin.x + target_visible.size.width;
        let vis_top = target_visible.origin.y + target_visible.size.height;
        if x + panel_w > vis_right - MARGIN {
            x = vis_right - panel_w - MARGIN;
        }
        if x < target_visible.origin.x + MARGIN {
            x = target_visible.origin.x + MARGIN;
        }
        if y + panel_h > vis_top - MARGIN {
            y = vis_top - panel_h - MARGIN;
        }
        if y < target_visible.origin.y + MARGIN {
            y = target_visible.origin.y + MARGIN;
        }

        TARGET_Y = y;

        // Start position for slide animation
        let start_y = if above { y - SLIDE_PX } else { y + SLIDE_PX };

        let content_rect = NSRect::new(
            NSPoint::new(x, start_y),
            NSSize::new(panel_w, panel_h),
        );

        // Create NSPanel (borderless, non-activating, floating)
        let style = NSWindowStyleMask::Borderless
            | NSWindowStyleMask::NonactivatingPanel;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            content_rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        );

        let floating_level = CGWindowLevelForKey(K_CG_FLOATING_WINDOW_LEVEL_KEY);
        panel.setLevel(floating_level as isize);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setHidesOnDeactivate(false);
        panel.setAlphaValue(0.0); // start invisible for fade-in

        // Create the background view (Liquid Glass or NSVisualEffectView fallback)
        let bg_view = create_background_view(panel_w, panel_h, mtm);

        // Position the label inside the background view
        label.setFrame(NSRect::new(
            NSPoint::new(PADDING, PADDING),
            NSSize::new(text_w, text_h),
        ));
        bg_view.addSubview(&label);

        panel.setContentView(Some(&bg_view));
        panel.orderFrontRegardless();

        // Set up animation state
        PHASE = PHASE_FADE_IN;
        ANIM_START = Some(Instant::now());

        // Auto-hide deadline: reading time based on char count
        let total_chars = translated.chars().count();
        let reading_secs = (total_chars as f64 / 15.0).clamp(2.0, 20.0);
        let total_ms = FADE_IN_MS + reading_secs * 1000.0 + 3000.0;
        AUTO_HIDE_DEADLINE =
            Some(Instant::now() + std::time::Duration::from_millis(total_ms as u64));

        POPUP_PANEL = Some(panel);
    }
}

// ---------------------------------------------------------------------------
// Background view: Liquid Glass (macOS 26+) or NSVisualEffectView fallback
// ---------------------------------------------------------------------------

unsafe fn create_background_view(
    width: f64,
    height: f64,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));

    // Try NSGlassEffectView (macOS 26 Tahoe+)
    {
        let name = c"NSGlassEffectView";
        if let Some(glass_cls) = AnyClass::get(name) {
            let view: *mut NSView = msg_send![glass_cls, alloc];
            let view: *mut NSView = msg_send![view, initWithFrame: frame];
            if !view.is_null() {
                let _: () = msg_send![view, setCornerRadius: CORNER_RADIUS];
                return Retained::from_raw(view).unwrap();
            }
        }
    }

    // Fallback: NSVisualEffectView with HUD material
    let effect_view =
        NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
    effect_view.setMaterial(NSVisualEffectMaterial::HUDWindow);
    effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect_view.setState(NSVisualEffectState::Active);
    effect_view.setWantsLayer(true);

    // Round corners and subtle border via CALayer
    if let Some(layer) = effect_view.layer() {
        let _: () = msg_send![&layer, setCornerRadius: CORNER_RADIUS];
        let _: () = msg_send![&layer, setMasksToBounds: true];
        let _: () = msg_send![&layer, setBorderWidth: 0.5f64];

        // White border at 0.2 alpha
        let border_color = NSColor::colorWithWhite_alpha(1.0, 0.2);
        let cg_color: *mut c_void = msg_send![&border_color, CGColor];
        if !cg_color.is_null() {
            let _: () = msg_send![&layer, setBorderColor: cg_color];
        }
    }

    // Upcast NSVisualEffectView to NSView
    Retained::into_super(effect_view)
}

// ---------------------------------------------------------------------------
// Animation
// ---------------------------------------------------------------------------

fn animate_popup() {
    unsafe {
        let panel = match (*std::ptr::addr_of!(POPUP_PANEL)).as_ref() {
            Some(p) => p,
            None => return,
        };

        let elapsed = match ANIM_START {
            Some(s) => s.elapsed().as_secs_f64() * 1000.0,
            None => return,
        };

        match PHASE {
            PHASE_FADE_IN => {
                let t = (elapsed / FADE_IN_MS).min(1.0);
                let ease = ease_out_cubic(t);

                panel.setAlphaValue(ease * MAX_ALPHA);

                // Slide toward target
                let offset = (1.0 - ease) * SLIDE_PX;
                let slide_y = if POSITIONED_ABOVE {
                    TARGET_Y - offset
                } else {
                    TARGET_Y + offset
                };
                let mut frame = panel.frame();
                frame.origin.y = slide_y;
                panel.setFrame_display(frame, true);

                if t >= 1.0 {
                    PHASE = PHASE_VISIBLE;
                    panel.setAlphaValue(MAX_ALPHA);
                    frame.origin.y = TARGET_Y;
                    panel.setFrame_display(frame, false);
                }
            }
            PHASE_FADE_OUT => {
                let duration = if DESELECT_CLOSE {
                    FADE_OUT_DESELECT_MS
                } else {
                    FADE_OUT_MS
                };
                let t = (elapsed / duration).min(1.0);
                let ease = ease_in_cubic(t);

                panel.setAlphaValue((1.0 - ease) * MAX_ALPHA);

                // Slide slightly upward while fading
                let offset = ease * (SLIDE_PX / 2.0);
                let mut frame = panel.frame();
                frame.origin.y = TARGET_Y + offset;
                panel.setFrame_display(frame, false);

                if t >= 1.0 {
                    destroy_popup();
                }
            }
            _ => {}
        }
    }
}

fn ease_out_cubic(t: f64) -> f64 {
    let u = 1.0 - t;
    1.0 - u * u * u
}

fn ease_in_cubic(t: f64) -> f64 {
    t * t * t
}

// ---------------------------------------------------------------------------
// on_click_away / destroy_popup
// ---------------------------------------------------------------------------

pub fn on_click_away() {
    unsafe {
        if (*std::ptr::addr_of!(POPUP_PANEL)).is_none() || PHASE == PHASE_FADE_OUT {
            return;
        }
        begin_fade_out(true);
    }
}

unsafe fn begin_fade_out(is_deselect: bool) {
    if PHASE == PHASE_FADE_OUT || PHASE == PHASE_NONE {
        return;
    }
    PHASE = PHASE_FADE_OUT;
    DESELECT_CLOSE = is_deselect;
    ANIM_START = Some(Instant::now());
    AUTO_HIDE_DEADLINE = None;
}

fn destroy_popup() {
    unsafe {
        if let Some(panel) = (*std::ptr::addr_of_mut!(POPUP_PANEL)).take() {
            panel.orderOut(None);
        }
        PHASE = PHASE_NONE;
        ANIM_START = None;
        DESELECT_CLOSE = false;
        AUTO_HIDE_DEADLINE = None;
    }
}

// ---------------------------------------------------------------------------
// NSApplication event pump (required for tray icon menus)
// ---------------------------------------------------------------------------

fn pump_app_events(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let mode = NSString::from_str("kCFRunLoopDefaultMode");
    loop {
        let event = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            None,
            &mode,
            true,
        );
        let Some(event) = event else { break };
        app.sendEvent(&event);
    }
}
