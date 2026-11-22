#![windows_subsystem = "windows"]

use std::env;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, utype: u32) -> i32;
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn show_error(msg: &str) {
    let text = to_wide(msg);
    let title = to_wide("PhaseMiX");
    unsafe {
        MessageBoxW(0, text.as_ptr(), title.as_ptr(), 0x10);
    }
}

fn main() -> ExitCode {
    let exe_path = match env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            show_error(&format!("Failed to locate PhaseMiX directory: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let root = exe_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let args: Vec<String> = env::args().skip(1).collect();
    let smoke = args.iter().any(|a| a == "--smoke-test");

    let py_name = if smoke { "python.exe" } else { "pythonw.exe" };
    let py_exe: PathBuf = root.join("runtime").join(py_name);
    let entry: PathBuf = root.join("app").join("desktop.py");

    if !py_exe.is_file() || !entry.is_file() {
        show_error("PhaseMiX runtime files are incomplete. Please reinstall PhaseMiX.");
        return ExitCode::FAILURE;
    }

    let mut cmd = Command::new(&py_exe);
    cmd.current_dir(root)
        .env("PHASEMIX_PACKAGED", "1")
        .arg("-I")
        .arg(&entry)
        .args(&args);

    if !smoke {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    match cmd.status() {
        Ok(st) if st.success() => ExitCode::SUCCESS,
        Ok(st) => ExitCode::from(st.code().unwrap_or(1) as u8),
        Err(e) => {
            show_error(&format!("Failed to start PhaseMiX runtime: {e}"));
            ExitCode::FAILURE
        }
    }
}
