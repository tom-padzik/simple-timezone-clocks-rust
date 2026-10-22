#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod color;
mod config;
mod face;
mod i18n;
mod icon;
mod search;
mod store;
mod tray;
mod zone;

fn main() {
    if let Err(error) = app::run() {
        show_error(&error.to_string());
    }
}

fn show_error(message: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let text = HSTRING::from(message);
    let title = HSTRING::from("Clocks");
    unsafe {
        let _ = MessageBoxW(None, &text, &title, MB_OK | MB_ICONERROR);
    }
}
