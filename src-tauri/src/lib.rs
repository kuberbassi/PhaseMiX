pub mod audio_engine;
pub mod dsp;

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running PhaseMiX");
}
