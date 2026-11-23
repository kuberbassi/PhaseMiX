#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, OpenEventW, SetEvent, EVENT_MODIFY_STATE};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
};

fn ensure_single_instance() -> bool {
    if std::env::args().any(|a| a == "--smoke-test") {
        return true;
    }
    unsafe {
        let handle = CreateMutexW(None, false, w!("Local\\PhaseMiXTauriSingleInstance"));
        if handle.is_ok() && GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\PhaseMiXShowEvent")) {
                let _ = SetEvent(ev);
                let _ = CloseHandle(ev);
            }
            if let Ok(hwnd) = FindWindowW(None, w!("PhaseMiX")) {
                if !hwnd.is_invalid() {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                    let _ = SetForegroundWindow(hwnd);
                }
            }
            return false;
        }
        // Intentionally leak mutex handle for process lifetime so lock persists until exit
        std::mem::forget(handle);
        true
    }
}

fn main() {
    if !ensure_single_instance() {
        return;
    }
    phasemix_lib::run();
}
