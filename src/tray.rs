//! One tray icon shared by every running clock.
//!
//! winit owns the native window class, so clocks are marked with the window
//! property `Clocks.Clock` instead. The process that holds
//! `Local\ClocksTrayOwner` draws the icon and finds the others with
//! `EnumWindows`. When that process exits it releases the mutex; another
//! living clock takes the icon over.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::core::w;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT,
    WAIT_ABANDONED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HDC, HMONITOR,
    MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, ReleaseMutex, SetEvent, WaitForSingleObject,
    EVENT_ALL_ACCESS,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetCursorPos, GetPropW, GetWindowTextW, IsWindow,
    IsWindowVisible, PeekMessageW, PostMessageW, SetForegroundWindow, SetPropW, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TranslateMessage, GWL_EXSTYLE, MONITORINFOF_PRIMARY, MSG, PM_REMOVE,
    SC_MINIMIZE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_RESTORE,
    SW_SHOW, WM_CLOSE, WM_SYSCOMMAND, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use crate::config::{ClockConfig, Language};
use crate::i18n::tr;
use crate::icon;
use crate::place::{self, Bounds};
use crate::store::FileSessionStore;
use crate::zone;

const PROPERTY: windows::core::PCWSTR = w!("Clocks.Clock");
const MUTEX_NAME: windows::core::PCWSTR = w!("Local\\ClocksTrayOwner");
const SESSION_EXIT_EVENT: windows::core::PCWSTR = w!("Local\\ClocksSessionExit");
const TRAY_GUID: u128 = 0xC10C_5017_4A2E_4B19_8C31_6D0A_7E4F_91B2;
static SESSION_EXIT_LOCAL: AtomicU8 = AtomicU8::new(0);

pub struct TrayGuard {
    shutdown: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for TrayGuard {
    fn drop(&mut self) {
        self.shutdown.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn hwnd_from_creation(cc: &eframe::CreationContext<'_>) -> Option<HWND> {
    let handle = cc.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(window) => Some(HWND(window.hwnd.get() as *mut c_void)),
        _ => None,
    }
}

pub fn tag_clock_window(hwnd: HWND) {
    unsafe {
        let _ = SetPropW(
            hwnd,
            PROPERTY,
            Some(HANDLE(std::ptr::dangling_mut::<c_void>())),
        );
        let _ = SetWindowSubclass(hwnd, Some(subclass_proc), 1, 0);
    }
}

pub fn spawn(language: Arc<AtomicU8>) -> TrayGuard {
    let (shutdown, receiver) = mpsc::channel();
    let thread = thread::spawn(move || tray_loop(language, receiver));
    TrayGuard {
        shutdown: Some(shutdown),
        thread: Some(thread),
    }
}

struct ClockWindow {
    hwnd: isize,
    title: String,
    visible: bool,
}

fn tray_loop(language: Arc<AtomicU8>, shutdown: Receiver<()>) {
    let Ok(mutex) = (unsafe { CreateMutexW(None, true, MUTEX_NAME) }) else {
        return;
    };
    let mut owner = unsafe { GetLastError() } != ERROR_ALREADY_EXISTS;
    let mut icon: Option<TrayIcon> = None;
    let mut signature = String::new();

    loop {
        if owner && icon.is_none() {
            icon = build_icon(language_of(&language));
            signature.clear();
        }
        pump_messages();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            handle_menu(&event.id.0);
        }
        // Leave immediately after Close all: rebuilding the menu or touching the
        // session file while WM_CLOSE is in flight can deadlock the last process.
        if session_exit_requested() {
            break;
        }
        if owner {
            let clocks = enumerate_clocks();
            let closed = FileSessionStore::user()
                .closed_list()
                .unwrap_or_default();
            let lang = language_of(&language);
            let next = menu_signature(lang, &clocks, &closed);
            if next != signature {
                signature = next;
                if let Some(icon) = &icon {
                    let _ = icon.set_tooltip(Some(tr(lang, "tray_tooltip")));
                    icon.set_menu(Some(Box::new(build_menu(lang, &clocks, &closed))));
                }
            }
        } else if became_owner(mutex) {
            owner = true;
            signature.clear();
        }

        match shutdown.recv_timeout(Duration::from_millis(400)) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => break,
        }
    }

    drop(icon);
    unsafe {
        if owner {
            let _ = ReleaseMutex(mutex);
        }
        let _ = CloseHandle(mutex);
    }
}

fn became_owner(mutex: HANDLE) -> bool {
    let status = unsafe { WaitForSingleObject(mutex, 0) };
    status == WAIT_OBJECT_0 || status == WAIT_ABANDONED
}

fn language_of(language: &AtomicU8) -> Language {
    Language::from_code(language.load(Ordering::Relaxed))
}

fn build_icon(language: Language) -> Option<TrayIcon> {
    let rgba = icon::clock_icon_rgba(32);
    let icon = Icon::from_rgba(rgba, 32, 32).ok()?;
    TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip(tr(language, "tray_tooltip"))
        .with_menu(Box::new(Menu::new()))
        .with_menu_on_left_click(true)
        .with_menu_on_right_click(true)
        .with_guid(TRAY_GUID)
        .build()
        .ok()
}

fn build_menu(language: Language, clocks: &[ClockWindow], closed: &[ClockConfig]) -> Menu {
    let menu = Menu::new();
    for clock in clocks {
        let mark = if clock.visible { "●" } else { "○" };
        let title = if clock.title.is_empty() {
            tr(language, "fallback_title")
        } else {
            clock.title.clone()
        };
        let item = MenuItem::with_id(
            format!("toggle:{}", clock.hwnd),
            format!("{mark} {title}"),
            true,
            None,
        );
        let _ = menu.append(&item);
    }
    if !clocks.is_empty() {
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    let _ = menu.append(&MenuItem::with_id(
        "new-clock",
        tr(language, "new_clock"),
        true,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(
        "show-all",
        tr(language, "show_all"),
        true,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(
        "hide-all",
        tr(language, "hide_all"),
        true,
        None,
    ));
    let closed_menu = Submenu::new(tr(language, "closed_clocks"), true);
    if closed.is_empty() {
        let empty = MenuItem::with_id(
            "closed-empty",
            tr(language, "closed_empty"),
            false,
            None,
        );
        let _ = closed_menu.append(&empty);
    } else {
        for clock in closed {
            let title = zone::window_title(&clock.zone);
            let label = if title.is_empty() {
                tr(language, "fallback_title")
            } else {
                title
            };
            let item = MenuItem::with_id(
                format!("restore-closed:{}", clock.id),
                label,
                true,
                None,
            );
            let _ = closed_menu.append(&item);
        }
    }
    let _ = menu.append(&closed_menu);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(
        "close-all",
        tr(language, "close_all"),
        true,
        None,
    ));
    menu
}

fn menu_signature(language: Language, clocks: &[ClockWindow], closed: &[ClockConfig]) -> String {
    let mut signature = format!("{language:?}");
    for clock in clocks {
        signature.push_str(&format!(
            "|{}:{}:{}",
            clock.hwnd, clock.visible, clock.title
        ));
    }
    for clock in closed {
        signature.push_str(&format!("|c:{}", clock.id));
    }
    signature
}

pub fn launch_new_clock() {
    launch_process(std::iter::empty::<&str>());
}

/// Start a clock that copies `json`, a serialized [`crate::config::ClockConfig`].
pub fn launch_inherited(json: &str) {
    launch_process(["--clock", json]);
}

fn launch_process<I, S>(args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe).args(args).spawn();
}

/// Work area of the monitor under the cursor, in physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Center a window of `inner_width` by `inner_height` points on the monitor
/// under the cursor. The cursor is where a tray-menu click just happened.
pub fn center_on_cursor_monitor(inner_width: f32, inner_height: f32) -> Option<(f32, f32)> {
    if inner_width <= 0.0 || inner_height <= 0.0 {
        return None;
    }
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let work = monitor_work_px(monitor)?;
    let scale = monitor_scale(monitor)?;
    let bounds = Bounds {
        x: work.left as f32 / scale,
        y: work.top as f32 / scale,
        width: (work.right - work.left) as f32 / scale,
        height: (work.bottom - work.top) as f32 / scale,
    };
    Some(place::center(bounds, (inner_width, inner_height)))
}

/// Work area of the monitor that contains this clock, in physical pixels.
pub fn window_work_px(hwnd_bits: isize) -> Option<PxRect> {
    let hwnd = HWND(hwnd_bits as *mut c_void);
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    monitor_work_px(monitor)
}

fn monitor_work_px(monitor: HMONITOR) -> Option<PxRect> {
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    unsafe {
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
    }
    Some(PxRect {
        left: info.rcWork.left,
        top: info.rcWork.top,
        right: info.rcWork.right,
        bottom: info.rcWork.bottom,
    })
}

fn monitor_scale(monitor: HMONITOR) -> Option<f32> {
    if monitor.is_invalid() {
        return None;
    }
    let mut dpi_x = 0_u32;
    let mut dpi_y = 0_u32;
    unsafe {
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).ok()?;
    }
    (dpi_x > 0).then_some(dpi_x as f32 / 96.0)
}

fn handle_menu(id: &str) {
    if id == "new-clock" {
        launch_new_clock();
    } else if id == "show-all" {
        for clock in enumerate_clocks() {
            set_visible(clock.hwnd, true);
        }
    } else if id == "hide-all" {
        for clock in enumerate_clocks() {
            set_visible(clock.hwnd, false);
        }
    } else if id == "close-all" {
        close_all_and_exit();
    } else if let Some(value) = id.strip_prefix("restore-closed:") {
        restore_closed_clock(value);
    } else if let Some(value) = id.strip_prefix("toggle:") {
        if let Ok(hwnd) = value.parse::<isize>() {
            let visible = enumerate_clocks()
                .into_iter()
                .find(|clock| clock.hwnd == hwnd)
                .is_some_and(|clock| clock.visible);
            set_visible(hwnd, !visible);
        }
    }
}

/// Mark a session exit and ask every clock, including hidden ones, to close.
///
/// Open clocks stay in the session file. Each process sees
/// [`session_exit_requested`] and skips the closed-clock history.
pub fn close_all_and_exit() {
    begin_session_exit();
    for clock in enumerate_clocks() {
        let hwnd = HWND(clock.hwnd as *mut c_void);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

fn begin_session_exit() {
    SESSION_EXIT_LOCAL.store(1, Ordering::SeqCst);
    let Ok(handle) = (unsafe { CreateEventW(None, true, false, SESSION_EXIT_EVENT) }) else {
        return;
    };
    let _ = unsafe { SetEvent(handle) };
    // Do not CloseHandle: HANDLE is Copy and the named event must stay open
    // so sibling processes can observe the signaled state.
    let _ = handle;
}

/// True after [`close_all_and_exit`] until the last process exits.
pub fn session_exit_requested() -> bool {
    if SESSION_EXIT_LOCAL.load(Ordering::SeqCst) != 0 {
        return true;
    }
    let Ok(handle) = (unsafe { OpenEventW(EVENT_ALL_ACCESS, false, SESSION_EXIT_EVENT) }) else {
        return false;
    };
    let signaled = unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0;
    let _ = unsafe { CloseHandle(handle) };
    signaled
}

fn restore_closed_clock(id: &str) {
    let store = FileSessionStore::user();
    let Ok(Some(mut config)) = store.restore_closed(id) else {
        return;
    };
    fix_closed_restore_position(&mut config);
    if let Ok(json) = serde_json::to_string(&config) {
        launch_inherited(&json);
    }
}

/// True when at least one tagged clock window already exists.
pub fn any_clocks_running() -> bool {
    !enumerate_clocks().is_empty()
}

/// Work areas of every monitor, in logical points (physical / DPI scale).
pub fn enum_work_areas() -> Vec<Bounds> {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_enum_proc),
            LPARAM(&mut monitors as *mut Vec<HMONITOR> as isize),
        );
    }
    monitors
        .into_iter()
        .filter_map(|monitor| work_bounds_points(monitor))
        .collect()
}

/// Primary monitor work area in logical points, or the first monitor found.
pub fn primary_work_area() -> Option<Bounds> {
    let mut monitors: Vec<(HMONITOR, bool)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_enum_primary_proc),
            LPARAM(&mut monitors as *mut Vec<(HMONITOR, bool)> as isize),
        );
    }
    let primary = monitors
        .iter()
        .find(|(_, is_primary)| *is_primary)
        .map(|(m, _)| *m)
        .or_else(|| monitors.first().map(|(m, _)| *m))?;
    work_bounds_points(primary)
}

fn work_bounds_points(monitor: HMONITOR) -> Option<Bounds> {
    let work = monitor_work_px(monitor)?;
    let scale = monitor_scale(monitor)?;
    Some(Bounds {
        x: work.left as f32 / scale,
        y: work.top as f32 / scale,
        width: (work.right - work.left) as f32 / scale,
        height: (work.bottom - work.top) as f32 / scale,
    })
}

unsafe extern "system" fn monitor_enum_proc(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    let monitors = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    true.into()
}

unsafe extern "system" fn monitor_enum_primary_proc(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    let monitors = unsafe { &mut *(lparam.0 as *mut Vec<(HMONITOR, bool)>) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    let primary = unsafe { GetMonitorInfoW(monitor, &mut info).as_bool() }
        && (info.dwFlags & MONITORINFOF_PRIMARY) != 0;
    monitors.push((monitor, primary));
    true.into()
}

fn fix_closed_restore_position(config: &mut ClockConfig) {
    let works = enum_work_areas();
    let Some(primary) = primary_work_area() else {
        return;
    };
    let size = outer_size(config);
    let origin = (config.window.x, config.window.y);
    if place::is_placeable(origin, size, &works) {
        return;
    }
    let (x, y) = place::cascade_from_top_right(primary, size, 0);
    config.window.x = x;
    config.window.y = y;
}

fn outer_size(config: &ClockConfig) -> (f32, f32) {
    (
        config.window.width.max(50.0),
        (config.window.height + 40.0).max(50.0),
    )
}

/// Move off-screen session clocks into a top-left cascade on the primary work area.
pub fn fix_session_positions(configs: &mut [ClockConfig]) {
    let works = enum_work_areas();
    let Some(primary) = primary_work_area() else {
        return;
    };
    let mut cascade = 0_u32;
    for config in configs {
        let size = outer_size(config);
        let origin = (config.window.x, config.window.y);
        if place::is_placeable(origin, size, &works) {
            continue;
        }
        let (x, y) = place::cascade_from_top_left(primary, size, cascade);
        config.window.x = x;
        config.window.y = y;
        cascade = cascade.saturating_add(1);
    }
}

fn enumerate_clocks() -> Vec<ClockWindow> {
    let mut clocks: Vec<ClockWindow> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut clocks as *mut Vec<ClockWindow> as isize),
        );
    }
    clocks.sort_by(|left, right| {
        left.title
            .cmp(&right.title)
            .then(left.hwnd.cmp(&right.hwnd))
    });
    clocks
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let clocks = unsafe { &mut *(lparam.0 as *mut Vec<ClockWindow>) };
    let property = unsafe { GetPropW(hwnd, PROPERTY) };
    if property.is_invalid() {
        return true.into();
    }
    let title = window_title(hwnd);
    clocks.push(ClockWindow {
        hwnd: hwnd_bits(hwnd),
        title,
        visible: unsafe { IsWindowVisible(hwnd).as_bool() },
    });
    true.into()
}

fn window_title(hwnd: HWND) -> String {
    let mut buffer = [0_u16; 512];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if copied <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buffer[..copied as usize])
    }
}

fn set_visible(bits: isize, show: bool) {
    let hwnd = HWND(bits as *mut c_void);
    unsafe {
        if IsWindow(Some(hwnd)).as_bool() {
            apply_visibility(hwnd, show);
        }
    }
}

unsafe fn apply_visibility(hwnd: HWND, show: bool) {
    let mut style = unsafe { ex_style(hwnd) };
    let tool = WS_EX_TOOLWINDOW.0 as isize;
    let app = WS_EX_APPWINDOW.0 as isize;
    if show {
        style = (style & !tool) | app;
    } else {
        style = (style | tool) & !app;
    }
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
        if show {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        } else {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

unsafe fn ex_style(hwnd: HWND) -> isize {
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWL_EXSTYLE) }
}

fn hwnd_bits(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

fn pump_messages() {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_SYSCOMMAND && (wparam.0 & 0xFFF0) == SC_MINIMIZE as usize {
        unsafe { apply_visibility(hwnd, false) };
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
