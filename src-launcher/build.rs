use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc_exe() -> Option<PathBuf> {
    let kits_root = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = fs::read_dir(kits_root) {
        let mut candidates: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join("x64").join("rc.exe"))
            .filter(|p| p.is_file())
            .collect();
        candidates.sort();
        if let Some(latest) = candidates.pop() {
            return Some(latest);
        }
    }
    None
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico_path = manifest_dir.join("..").join("installer").join("phasemix.ico");
    println!("cargo:rerun-if-changed={}", ico_path.display());

    if let (Some(rc_exe), true) = (find_rc_exe(), ico_path.exists()) {
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let rc_file = out_dir.join("phasemix.rc");
        let res_file = out_dir.join("phasemix.res");
        let ico_escaped = ico_path.to_string_lossy().replace('\\', "\\\\");

        let rc_content = format!(
            r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION 3,2,0,0
PRODUCTVERSION 3,2,0,0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "kuberbassi"
      VALUE "FileDescription", "PhaseMiX Audio Console"
      VALUE "FileVersion", "3.2.0"
      VALUE "InternalName", "PhaseMiX"
      VALUE "OriginalFilename", "PhaseMiX.exe"
      VALUE "ProductName", "PhaseMiX"
      VALUE "ProductVersion", "3.2.0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#,
            ico = ico_escaped
        );

        fs::write(&rc_file, rc_content).expect("write rc file");
        let status = Command::new(rc_exe)
            .args(["/nologo", "/fo"])
            .arg(&res_file)
            .arg(&rc_file)
            .status()
            .expect("run rc.exe");
        if status.success() && res_file.exists() {
            println!("cargo:rustc-link-arg-bins={}", res_file.display());
        }
    }
}
