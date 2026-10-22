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
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::core::w;
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, WAIT_ABANDONED,
    WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetPropW, GetWindowTextW, IsWindow, IsWindowVisible,
    PeekMessageW, SetForegroundWindow, SetPropW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, GWL_EXSTYLE, MSG, PM_REMOVE, SC_MINIMIZE, SWP_FRAMECHANGED, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOW, WM_SYSCOMMAND, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
};

use crate::config::Language;
use crate::i18n::tr;
use crate::icon;

const PROPERTY: windows::core::PCWSTR = w!("Clocks.Clock");
const MUTEX_NAME: windows::core::PCWSTR = w!("Local\\ClocksTrayOwner");
const TRAY_GUID: u128 = 0xC10C_5017_4A2E_4B19_8C31_6D0A_7E4F_91B2;

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
        let _ = SetPropW(hwnd, PROPERTY, Some(HANDLE(1 as *mut c_void)));
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
        if owner {
            let clocks = enumerate_clocks();
            let lang = language_of(&language);
            let next = menu_signature(lang, &clocks);
            if next != signature {
                signature = next;
                if let Some(icon) = &icon {
                    let _ = icon.set_tooltip(Some(tr(lang, "tray_tooltip")));
                    let _ = icon.set_menu(Some(Box::new(build_menu(lang, &clocks))));
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
    if owner {
        unsafe {
            let _ = ReleaseMutex(mutex);
        }
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

fn build_menu(language: Language, clocks: &[ClockWindow]) -> Menu {
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
    menu
}

fn menu_signature(language: Language, clocks: &[ClockWindow]) -> String {
    let mut signature = format!("{language:?}");
    for clock in clocks {
        signature.push_str(&format!(
            "|{}:{}:{}",
            clock.hwnd, clock.visible, clock.title
        ));
    }
    signature
}

pub fn launch_new_clock() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe).spawn();
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
