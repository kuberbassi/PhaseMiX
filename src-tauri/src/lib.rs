pub mod audio_engine;
pub mod dsp;

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State, WindowEvent,
};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

use audio_engine::{
    default_config_path, AudioRouterEngine, DevicesResponse, FastMetersEvent, MixerPatch,
    MixerState, SelectedIds, StatusPayload,
};

pub struct AppState {
    pub engine: Arc<AudioRouterEngine>,
    pub emitter_stop: Arc<AtomicBool>,
}

#[derive(Serialize)]
pub struct UpdateMixerResponse {
    pub ok: bool,
    pub mixer: MixerState,
}

#[derive(Serialize)]
pub struct OkStatusResponse {
    pub ok: bool,
    #[serde(flatten)]
    pub status: StatusPayload,
}

#[derive(Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[tauri::command(rename_all = "snake_case")]
async fn get_devices(state: State<'_, AppState>) -> Result<DevicesResponse, String> {
    let eng = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || eng.refresh_devices())
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command(rename_all = "snake_case")]
async fn set_devices(
    state: State<'_, AppState>,
    selected: SelectedIds,
) -> Result<StatusPayload, String> {
    let eng = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || eng.set_devices(selected))
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command(rename_all = "snake_case")]
async fn get_state(state: State<'_, AppState>) -> Result<StatusPayload, String> {
    let eng = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || eng.status_payload())
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command(rename_all = "snake_case")]
fn get_meters(state: State<'_, AppState>) -> FastMetersEvent {
    state.engine.fast_meters_event()
}

#[tauri::command(rename_all = "snake_case")]
fn update_mixer(state: State<'_, AppState>, patch: MixerPatch) -> UpdateMixerResponse {
    let mixer = state.engine.update_mixer(patch);
    UpdateMixerResponse { ok: true, mixer }
}

#[tauri::command(rename_all = "snake_case")]
async fn start_engine(state: State<'_, AppState>) -> Result<StatusPayload, String> {
    let eng = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        eng.start();
        eng.status_payload()
    })
    .await
    .map_err(|e| format!("{e}"))
}

#[tauri::command(rename_all = "snake_case")]
async fn stop_engine(state: State<'_, AppState>) -> Result<StatusPayload, String> {
    let eng = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        eng.stop();
        eng.status_payload()
    })
    .await
    .map_err(|e| format!("{e}"))
}

#[tauri::command(rename_all = "snake_case")]
async fn boost_capture(state: State<'_, AppState>) -> Result<OkStatusResponse, String> {
    let eng = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (ok, status) = eng.boost_capture();
        OkStatusResponse { ok, status }
    })
    .await
    .map_err(|e| format!("{e}"))
}

#[tauri::command(rename_all = "snake_case")]
async fn set_win_vol(
    state: State<'_, AppState>,
    role: String,
    volume_pct: f64,
) -> Result<OkStatusResponse, String> {
    let eng = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (ok, status) = eng.set_win_vol(&role, volume_pct);
        OkStatusResponse { ok, status }
    })
    .await
    .map_err(|e| format!("{e}"))
}

#[tauri::command(rename_all = "snake_case")]
fn open_sound_settings() -> OkResponse {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let launched = Command::new("cmd")
        .args(["/c", "start", "", "ms-settings:sound"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .is_ok();
    if !launched {
        let _ = Command::new("control")
            .arg("mmsys.cpl")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    OkResponse { ok: true }
}

const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("PhaseMiX");

fn autostart_opt_out_marker() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata).join("PhaseMiX").join(".no_autostart")
}

#[tauri::command(rename_all = "snake_case")]
fn get_autostart() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, Some(0), KEY_READ, &mut hkey)
            != ERROR_SUCCESS
        {
            return false;
        }
        let mut size: u32 = 0;
        let status = RegQueryValueExW(hkey, RUN_VALUE, None, None, None, Some(&mut size));
        let _ = RegCloseKey(hkey);
        status == ERROR_SUCCESS && size > 2
    }
}

#[tauri::command(rename_all = "snake_case")]
fn set_autostart(enabled: bool) -> Result<bool, String> {
    let marker = autostart_opt_out_marker();
    if let Some(parent) = marker.parent() {
        let _ = fs::create_dir_all(parent);
    }

    unsafe {
        let mut hkey = HKEY::default();
        let mut disp = Default::default();
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            Some(&mut disp),
        );
        if rc != ERROR_SUCCESS {
            return Err(format!("Failed to open startup registry key: {:?}", rc));
        }

        if enabled {
            let exe = std::env::current_exe().map_err(|e| format!("{e}"))?;
            let cmd = format!("\"{}\" --tray", exe.display());
            let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes =
                std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            let res = RegSetValueExW(hkey, RUN_VALUE, Some(0), REG_SZ, Some(bytes));
            let _ = RegCloseKey(hkey);
            if res != ERROR_SUCCESS {
                return Err(format!("Failed to write startup value: {:?}", res));
            }
            let _ = fs::remove_file(&marker);
            Ok(true)
        } else {
            let res = RegDeleteValueW(hkey, RUN_VALUE);
            let _ = RegCloseKey(hkey);
            if res != ERROR_SUCCESS && res != ERROR_FILE_NOT_FOUND {
                return Err(format!("Failed to remove startup value: {:?}", res));
            }
            let _ = fs::write(&marker, b"disabled");
            Ok(false)
        }
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

pub fn run() {
    let engine = Arc::new(AudioRouterEngine::new(default_config_path()));
    let emitter_stop = Arc::new(AtomicBool::new(false));
    let smoke = std::env::args().any(|a| a == "--smoke-test");
    let start_in_tray = !smoke && std::env::args().any(|a| a == "--tray");

    let app_state = AppState {
        engine: engine.clone(),
        emitter_stop: emitter_stop.clone(),
    };

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_devices,
            set_devices,
            get_state,
            get_meters,
            update_mixer,
            start_engine,
            stop_engine,
            boost_capture,
            set_win_vol,
            open_sound_settings,
            get_autostart,
            set_autostart,
        ])
        .setup(move |app| {
            if let Some(win_cfg) = app.config().app.windows.first().cloned() {
                let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &win_cfg)?;
                if smoke {
                    let smoke_wv_dir = std::env::temp_dir().join("phasemix_smoke_webview");
                    let _ = std::fs::create_dir_all(&smoke_wv_dir);
                    builder = builder.visible(false).data_directory(smoke_wv_dir);
                } else if start_in_tray {
                    builder = builder.visible(false);
                }
                builder.build()?;
            }

            let state = app.state::<AppState>();
            state.engine.start();

            if smoke {
                let handle = app.handle().clone();
                let eng = state.engine.clone();
                let stop_sig = state.emitter_stop.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(700));
                    let st = eng.status_payload();
                    println!(
                        "TAURI_SMOKE_PASS running={} preset={}",
                        st.running, st.mixer.position_preset
                    );
                    stop_sig.store(true, Ordering::Relaxed);
                    eng.stop();
                    handle.exit(0);
                });
                return Ok(());
            }

            // Build native Windows system tray menu
            let open_item = MenuItem::with_id(app, "open", "Open PhaseMiX", true, None::<&str>)?;
            let mute_item =
                MenuItem::with_id(app, "mute", "Mute / Unmute All", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit PhaseMiX", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_item, &mute_item, &sep, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::with_id("phasemix-tray")
                .tooltip("PhaseMiX Audio Console")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app_h, event| match event.id.as_ref() {
                    "open" => show_main_window(app_h),
                    "mute" => {
                        let st = app_h.state::<AppState>();
                        let cur_mute = st.engine.fast_meters_event().master_mute;
                        st.engine.update_mixer(MixerPatch {
                            master_mute: Some(!cur_mute),
                            ..Default::default()
                        });
                    }
                    "quit" => {
                        let st = app_h.state::<AppState>();
                        st.emitter_stop.store(true, Ordering::Relaxed);
                        st.engine.stop();
                        app_h.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });

            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }
            let _ = tray_builder.build(app)?;

            // Push live 30 Hz meter snapshots over WebView2 IPC only while window is visible
            let emit_handle = app.handle().clone();
            let emit_eng = state.engine.clone();
            let emit_stop = state.emitter_stop.clone();
            thread::Builder::new()
                .name("meters-emitter".to_string())
                .spawn(move || {
                    let mut sent_stopped = false;
                    while !emit_stop.load(Ordering::Relaxed) {
                        let visible = emit_handle
                            .get_webview_window("main")
                            .and_then(|w| w.is_visible().ok())
                            .unwrap_or(false);
                        if !visible {
                            thread::sleep(Duration::from_millis(250));
                            continue;
                        }
                        let is_running = emit_eng.is_running();
                        if is_running || !sent_stopped {
                            let snap = emit_eng.fast_meters_event();
                            let _ = emit_handle.emit("phasemix:meters", snap);
                            sent_stopped = !is_running;
                        }
                        thread::sleep(Duration::from_millis(if is_running { 33 } else { 200 }));
                    }
                })?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let st = window.state::<AppState>();
                if st.engine.is_running() {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    st.emitter_stop.store(true, Ordering::Relaxed);
                    st.engine.stop();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running PhaseMiX");
}
