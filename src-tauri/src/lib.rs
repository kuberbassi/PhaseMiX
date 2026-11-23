pub mod audio_engine;
pub mod dsp;

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, RunEvent, State, WebviewWindowBuilder, Wry,
};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HWND};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
};
use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_REMOVE, WM_HOTKEY};

use audio_engine::{
    default_config_path, AudioRouterEngine, CalibrationResult, DevicesResponse, FastMetersEvent,
    MixerPatch, MixerState, SelectedIds, StatusPayload,
};

pub struct TrayUiState {
    pub tray: TrayIcon<Wry>,
    pub presets: Vec<(String, CheckMenuItem<Wry>)>,
    pub mute_item: CheckMenuItem<Wry>,
    pub last_sig: String,
}

pub struct AppState {
    pub engine: Arc<AudioRouterEngine>,
    pub emitter_stop: Arc<AtomicBool>,
    pub tray_ui: Mutex<Option<TrayUiState>>,
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

#[derive(Serialize)]
pub struct ProfileIoResponse {
    pub ok: bool,
    pub cancelled: bool,
    pub path: Option<String>,
    pub status: Option<StatusPayload>,
}

fn preset_short_label(id: &str) -> &'static str {
    match id {
        "desk_triangle" => "Desk Triangle",
        "cinema_center" => "Cinema 3.2",
        "wide_room" => "Wide Room",
        "night_safe" => "Night Safe",
        "asym_desk" => "Offset Desk",
        "pure_mirror" => "Pure Flat",
        _ => "Custom",
    }
}

fn sync_tray_ui(app: &tauri::AppHandle, force: bool) {
    let st = app.state::<AppState>();
    let (preset, vol, mute, delay_ms, running) = st.engine.tray_summary();
    let sig = format!("{preset}:{vol:.0}:{mute}:{delay_ms:.0}:{running}");
    let should_update = {
        let Ok(mut guard) = st.tray_ui.lock() else {
            return;
        };
        match guard.as_mut() {
            Some(ui) if force || ui.last_sig != sig => {
                ui.last_sig = sig;
                true
            }
            _ => false,
        }
    };
    if !should_update {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let st = handle.state::<AppState>();
        let Ok(guard) = st.tray_ui.lock() else {
            return;
        };
        let Some(ui) = guard.as_ref() else {
            return;
        };
        for (id, item) in &ui.presets {
            let _ = item.set_checked(id == &preset);
        }
        let _ = ui.mute_item.set_checked(mute);
        let state_txt = if !running {
            "Stopped".to_string()
        } else if mute {
            "MUTED".to_string()
        } else {
            format!("{:.0}%", vol)
        };
        let tip = format!(
            "PhaseMiX • {} • {} ({:.0} ms)",
            preset_short_label(&preset),
            state_txt,
            delay_ms
        );
        let _ = ui.tray.set_tooltip(Some(&tip));
    });
}

fn run_file_dialog(save: bool, default_name: &str) -> Option<PathBuf> {
    let filter: Vec<u16> = "PhaseMiX Profile (*.json)\0*.json\0All Files (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut file_buf = vec![0_u16; 4096];
    for (i, ch) in default_name.encode_utf16().take(4000).enumerate() {
        file_buf[i] = ch;
    }
    let title = if save {
        w!("Export PhaseMiX Profile")
    } else {
        w!("Import PhaseMiX Profile")
    };
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = HWND::default();
    ofn.lpstrFilter = PCWSTR(filter.as_ptr());
    ofn.lpstrFile = PWSTR(file_buf.as_mut_ptr());
    ofn.nMaxFile = file_buf.len() as u32;
    ofn.lpstrTitle = title;
    ofn.lpstrDefExt = w!("json");
    ofn.Flags = if save {
        OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT | OFN_NOCHANGEDIR
    } else {
        OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR
    };

    let ok = unsafe {
        if save {
            GetSaveFileNameW(&mut ofn).as_bool()
        } else {
            GetOpenFileNameW(&mut ofn).as_bool()
        }
    };
    if !ok {
        return None;
    }
    let end = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
    Some(PathBuf::from(String::from_utf16_lossy(&file_buf[..end])))
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
fn update_mixer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    patch: MixerPatch,
) -> UpdateMixerResponse {
    let mixer = state.engine.update_mixer(patch);
    sync_tray_ui(&app, false);
    UpdateMixerResponse { ok: true, mixer }
}

#[tauri::command(rename_all = "snake_case")]
async fn calibrate_sync_delay(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<CalibrationResult, String> {
    let eng = state.engine.clone();
    let res = tauri::async_runtime::spawn_blocking(move || eng.calibrate_sync_delay())
        .await
        .map_err(|e| format!("{e}"))?;
    if res.ok {
        let _ = app.emit("phasemix:mixer", res.status.mixer.clone());
        sync_tray_ui(&app, false);
    }
    Ok(res)
}

#[tauri::command(rename_all = "snake_case")]
async fn export_profile(state: State<'_, AppState>) -> Result<ProfileIoResponse, String> {
    let eng = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<ProfileIoResponse, String> {
        let Some(path) = run_file_dialog(true, "PhaseMiX-Profile.json") else {
            return Ok(ProfileIoResponse {
                ok: false,
                cancelled: true,
                path: None,
                status: None,
            });
        };
        let json = eng.export_profile_json()?;
        fs::write(&path, json).map_err(|e| format!("Failed to save profile: {e}"))?;
        Ok(ProfileIoResponse {
            ok: true,
            cancelled: false,
            path: Some(path.to_string_lossy().into_owned()),
            status: None,
        })
    })
    .await
    .map_err(|e| format!("{e}"))?
}

#[tauri::command(rename_all = "snake_case")]
async fn import_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ProfileIoResponse, String> {
    let eng = state.engine.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> Result<ProfileIoResponse, String> {
        let Some(path) = run_file_dialog(false, "") else {
            return Ok(ProfileIoResponse {
                ok: false,
                cancelled: true,
                path: None,
                status: None,
            });
        };
        let raw =
            fs::read_to_string(&path).map_err(|e| format!("Failed to read profile: {e}"))?;
        let status = eng.import_profile_json(&raw)?;
        Ok(ProfileIoResponse {
            ok: true,
            cancelled: false,
            path: Some(path.to_string_lossy().into_owned()),
            status: Some(status),
        })
    })
    .await
    .map_err(|e| format!("{e}"))??;
    if let Some(ref st) = res.status {
        let _ = app.emit("phasemix:mixer", st.mixer.clone());
        sync_tray_ui(&app, false);
    }
    Ok(res)
}

#[tauri::command(rename_all = "snake_case")]
async fn reset_factory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<StatusPayload, String> {
    let eng = state.engine.clone();
    let st = tauri::async_runtime::spawn_blocking(move || eng.reset_factory())
        .await
        .map_err(|e| format!("{e}"))?;
    let _ = app.emit("phasemix:mixer", st.mixer.clone());
    sync_tray_ui(&app, false);
    Ok(st)
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

fn signal_wake_event() {
    unsafe {
        if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\PhaseMiXShowEvent")) {
            let _ = SetEvent(ev);
            let _ = CloseHandle(ev);
        }
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = handle.get_webview_window("main") {
            let _ = win.unminimize();
            let _ = win.show();
            let _ = win.set_focus();
            return;
        }
        if let Some(win_cfg) = handle.config().app.windows.first().cloned() {
            if let Ok(builder) = WebviewWindowBuilder::from_config(&handle, &win_cfg) {
                if let Ok(win) = builder.build() {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        }
    });
}

pub fn run() {
    let engine = Arc::new(AudioRouterEngine::new(default_config_path()));
    let emitter_stop = Arc::new(AtomicBool::new(false));
    let smoke = std::env::args().any(|a| a == "--smoke-test");
    let start_in_tray = !smoke && std::env::args().any(|a| a == "--tray");

    let app_state = AppState {
        engine: engine.clone(),
        emitter_stop: emitter_stop.clone(),
        tray_ui: Mutex::new(None),
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
            calibrate_sync_delay,
            export_profile,
            import_profile,
            reset_factory,
        ])
        .setup(move |app| {
            // In --tray silent boot, defer WebView2 creation until user opens tray/app (~6 MB RAM)
            if !start_in_tray {
                if let Some(win_cfg) = app.config().app.windows.first().cloned() {
                    let mut builder = WebviewWindowBuilder::from_config(app.handle(), &win_cfg)?;
                    if smoke {
                        let smoke_wv_dir = std::env::temp_dir().join("phasemix_smoke_webview");
                        let _ = std::fs::create_dir_all(&smoke_wv_dir);
                        builder = builder.visible(false).data_directory(smoke_wv_dir);
                    }
                    builder.build()?;
                }
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

            // Build native Windows system tray menu with live checkmarks & Sound Presets submenu
            let initial_tray = state.engine.tray_summary();
            let cur_p = initial_tray.0.as_str();
            let open_item = MenuItem::with_id(app, "open", "Open PhaseMiX", true, None::<&str>)?;
            let p_desk = CheckMenuItem::with_id(
                app,
                "preset:desk_triangle",
                "Desk Triangle (Balanced)",
                true,
                cur_p == "desk_triangle",
                None::<&str>,
            )?;
            let p_cinema = CheckMenuItem::with_id(
                app,
                "preset:cinema_center",
                "Cinema 3.2 Dialogue",
                true,
                cur_p == "cinema_center",
                None::<&str>,
            )?;
            let p_music = CheckMenuItem::with_id(
                app,
                "preset:wide_room",
                "Wide Room Music",
                true,
                cur_p == "wide_room",
                None::<&str>,
            )?;
            let p_night = CheckMenuItem::with_id(
                app,
                "preset:night_safe",
                "Night Ear-Safe (-15 dB)",
                true,
                cur_p == "night_safe",
                None::<&str>,
            )?;
            let p_offset = CheckMenuItem::with_id(
                app,
                "preset:asym_desk",
                "Offset Desk",
                true,
                cur_p == "asym_desk",
                None::<&str>,
            )?;
            let p_flat = CheckMenuItem::with_id(
                app,
                "preset:pure_mirror",
                "Pure Flat",
                true,
                cur_p == "pure_mirror",
                None::<&str>,
            )?;
            let presets_sub = Submenu::with_items(
                app,
                "Sound Presets",
                true,
                &[&p_desk, &p_cinema, &p_music, &p_night, &p_offset, &p_flat],
            )?;
            let mute_item = CheckMenuItem::with_id(
                app,
                "mute",
                "Mute / Unmute All  (Ctrl+Alt+M)",
                true,
                initial_tray.2,
                None::<&str>,
            )?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit PhaseMiX", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&open_item, &presets_sub, &mute_item, &sep, &quit_item],
            )?;

            let mut tray_builder = TrayIconBuilder::with_id("phasemix-tray")
                .tooltip("PhaseMiX Audio Console")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app_h, event| {
                    let id = event.id.as_ref();
                    if id == "open" {
                        show_main_window(app_h);
                    } else if id == "mute" {
                        let st = app_h.state::<AppState>();
                        let cur_mute = st.engine.fast_meters_event().master_mute;
                        let updated = st.engine.update_mixer(MixerPatch {
                            master_mute: Some(!cur_mute),
                            ..Default::default()
                        });
                        let _ = app_h.emit("phasemix:mixer", updated);
                        sync_tray_ui(app_h, true);
                    } else if let Some(preset_id) = id.strip_prefix("preset:") {
                        let st = app_h.state::<AppState>();
                        if let Some(updated) = st.engine.apply_preset(preset_id) {
                            let _ = app_h.emit("phasemix:mixer", updated);
                        }
                        sync_tray_ui(app_h, true);
                    } else if id == "quit" {
                        let st = app_h.state::<AppState>();
                        st.emitter_stop.store(true, Ordering::Relaxed);
                        signal_wake_event();
                        st.engine.stop();
                        app_h.exit(0);
                    }
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
            let tray = tray_builder.build(app)?;
            if let Ok(mut guard) = state.tray_ui.lock() {
                *guard = Some(TrayUiState {
                    tray,
                    presets: vec![
                        ("desk_triangle".to_string(), p_desk),
                        ("cinema_center".to_string(), p_cinema),
                        ("wide_room".to_string(), p_music),
                        ("night_safe".to_string(), p_night),
                        ("asym_desk".to_string(), p_offset),
                        ("pure_mirror".to_string(), p_flat),
                    ],
                    mute_item,
                    last_sig: String::new(),
                });
            }
            sync_tray_ui(app.handle(), true);

            // Global hotkeys: Ctrl+Alt+M (Mute), Ctrl+Alt+P (Cycle Preset), Ctrl+Alt+N (Midnight Guard toggle)
            let hk_handle = app.handle().clone();
            let hk_stop = state.emitter_stop.clone();
            thread::Builder::new()
                .name("global-hotkeys".to_string())
                .spawn(move || unsafe {
                    let mut msg = MSG::default();
                    let _ = PeekMessageW(&mut msg, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE);
                    let mods = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
                    let _ = RegisterHotKey(None, 1, mods, 0x4D);
                    let _ = RegisterHotKey(None, 2, mods, 0x50);
                    let _ = RegisterHotKey(None, 3, mods, 0x4E);
                    while !hk_stop.load(Ordering::Relaxed) {
                        while PeekMessageW(&mut msg, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE)
                            .as_bool()
                        {
                            let st = hk_handle.state::<AppState>();
                            let updated = match msg.wParam.0 as i32 {
                                1 => {
                                    let cur = st.engine.fast_meters_event().master_mute;
                                    Some(st.engine.update_mixer(MixerPatch {
                                        master_mute: Some(!cur),
                                        ..Default::default()
                                    }))
                                }
                                2 => Some(st.engine.cycle_smart_preset()),
                                3 => Some(st.engine.toggle_night_guard()),
                                _ => None,
                            };
                            if let Some(m) = updated {
                                let _ = hk_handle.emit("phasemix:mixer", m);
                                sync_tray_ui(&hk_handle, false);
                            }
                        }
                        thread::sleep(Duration::from_millis(35));
                    }
                    let _ = UnregisterHotKey(None, 1);
                    let _ = UnregisterHotKey(None, 2);
                    let _ = UnregisterHotKey(None, 3);
                })?;

            // Listen for single-instance wake event (when launching shortcut while UI is unloaded in tray)
            let wake_handle = app.handle().clone();
            let wake_stop = state.emitter_stop.clone();
            thread::Builder::new()
                .name("wake-listener".to_string())
                .spawn(move || unsafe {
                    if let Ok(ev) =
                        CreateEventW(None, false, false, w!("Local\\PhaseMiXShowEvent"))
                    {
                        while !wake_stop.load(Ordering::Relaxed) {
                            let wait = WaitForSingleObject(ev, 500);
                            if wake_stop.load(Ordering::Relaxed) {
                                break;
                            }
                            if wait.0 == 0 {
                                show_main_window(&wake_handle);
                            }
                        }
                        let _ = CloseHandle(ev);
                    }
                })?;

            // Push live 30 Hz meter snapshots over WebView2 IPC + keep tray tooltip live
            let emit_handle = app.handle().clone();
            let emit_eng = state.engine.clone();
            let emit_stop = state.emitter_stop.clone();
            thread::Builder::new()
                .name("meters-emitter".to_string())
                .spawn(move || {
                    let mut sent_stopped = false;
                    while !emit_stop.load(Ordering::Relaxed) {
                        sync_tray_ui(&emit_handle, false);
                        let visible = emit_handle
                            .get_webview_window("main")
                            .and_then(|w| w.is_visible().ok())
                            .unwrap_or(false);
                        if !visible {
                            thread::sleep(Duration::from_millis(300));
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
        .build(tauri::generate_context!())
        .expect("error while building PhaseMiX")
        .run(|app_h, event| {
            // Closing [X] while running unloads WebView2 (~283 MB freed to ~6 MB tray core)
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                let st = app_h.state::<AppState>();
                if st.engine.is_running() {
                    api.prevent_exit();
                } else {
                    st.emitter_stop.store(true, Ordering::Relaxed);
                    signal_wake_event();
                    st.engine.stop();
                }
            }
        });
}
