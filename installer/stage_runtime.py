"""Stage a clean, self-contained PhaseMiX portable Python 3.14 runtime without PyInstaller."""

from __future__ import annotations

import compileall
import importlib.metadata as md
import os
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST_DIR = ROOT / "dist" / "PhaseMiX"
RUNTIME_DIR = DIST_DIR / "runtime"
APP_DIR = DIST_DIR / "app"

ROOT_PACKAGES = [
    "Flask",
    "Flask-Cors",
    "numpy",
    "scipy",
    "PyAudioWPatch",
    "pycaw",
    "comtypes",
    "pywebview",
    "pystray",
    "pillow",
    "waitress",
]

SKIP_STDLIB_DIRS = {
    "site-packages",
    "tkinter",
    "idlelib",
    "test",
    "ensurepip",
    "turtledemo",
    "venv",
    "__pycache__",
}

SKIP_PKG_TOP_DIRS = {"__pycache__", "pip", "setuptools", "wheel", "pkg_resources"}


def resolve_distributions(roots: list[str]) -> list[md.Distribution]:
    seen: set[str] = set()
    stack = list(roots)
    dists: list[md.Distribution] = []
    while stack:
        req = stack.pop()
        name = re.split(r"[<>=!~;\[\s]", req)[0].strip().lower().replace("_", "-")
        if not name or name in seen:
            continue
        seen.add(name)
        try:
            d = md.distribution(name)
        except md.PackageNotFoundError:
            continue
        dists.append(d)
        for dep in d.requires or []:
            if "extra ==" in dep:
                continue
            if "sys_platform" in dep and "win32" not in dep:
                continue
            stack.append(dep)
    return dists


def ignore_junk(_dir: str, names: list[str]) -> set[str]:
    return {
        n
        for n in names
        if n == "__pycache__"
        or n.endswith((".pyc", ".pyo", ".pdb", ".lib", ".a"))
        or (n.startswith(("tcl", "tk", "_tkinter")) and n.endswith((".dll", ".pyd")))
    }


def stage() -> None:
    launcher_exe = ROOT / "src-launcher" / "target" / "release" / "PhaseMiX.exe"
    if not launcher_exe.is_file():
        raise FileNotFoundError(f"Missing Rust launcher at {launcher_exe}")

    if DIST_DIR.exists():
        shutil.rmtree(DIST_DIR)
    RUNTIME_DIR.mkdir(parents=True, exist_ok=True)
    APP_DIR.mkdir(parents=True, exist_ok=True)

    shutil.copy2(launcher_exe, DIST_DIR / "PhaseMiX.exe")

    # 1. Stage PhaseMiX Python sources, built React frontend, and icon
    for py_name in ("desktop.py", "app.py", "audio_engine.py", "dsp.py"):
        shutil.copy2(ROOT / py_name, APP_DIR / py_name)
    shutil.copytree(ROOT / "frontend" / "dist", APP_DIR / "frontend" / "dist")
    (APP_DIR / "assets").mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "installer" / "phasemix.ico", APP_DIR / "assets" / "phasemix.ico")

    # 2. Stage signed Python core executables, DLLs, and stdlib from sys.base_prefix
    base = Path(sys.base_prefix)
    for item in base.iterdir():
        if item.is_file() and (
            item.name.lower() in {"python.exe", "pythonw.exe"}
            or item.name.lower().endswith(".dll")
        ):
            shutil.copy2(item, RUNTIME_DIR / item.name)

    shutil.copytree(base / "DLLs", RUNTIME_DIR / "DLLs", ignore=ignore_junk)

    lib_dst = RUNTIME_DIR / "Lib"
    lib_dst.mkdir(parents=True, exist_ok=True)
    for item in (base / "Lib").iterdir():
        if item.name in SKIP_STDLIB_DIRS:
            continue
        if item.is_dir():
            shutil.copytree(item, lib_dst / item.name, ignore=ignore_junk)
        elif item.is_file():
            shutil.copy2(item, lib_dst / item.name)

    # 3. Copy only resolved transitive package files into runtime/Lib/site-packages
    sp_dst = lib_dst / "site-packages"
    sp_dst.mkdir(parents=True, exist_ok=True)

    copied_roots: set[Path] = set()
    dists = resolve_distributions(ROOT_PACKAGES)
    for dist in dists:
        sp_root = Path(dist.locate_file("")).resolve()
        top_items: set[str] = set()
        for f in dist.files or []:
            parts = Path(str(f)).parts
            if not parts:
                continue
            top = parts[0]
            if top == ".." or top in SKIP_PKG_TOP_DIRS:
                # Some wheels place sibling `.libs` via relative path `../scipy.libs/...`
                if top == ".." and len(parts) > 1 and parts[1].endswith(".libs"):
                    top_items.add(parts[1])
                continue
            top_items.add(top)

        # Also catch `<dist>.libs` directories placed by delvewheel in site-packages
        norm_name = dist.metadata["Name"].lower().replace("-", "_")
        candidate_libs = sp_root / f"{norm_name}.libs"
        if candidate_libs.is_dir():
            top_items.add(candidate_libs.name)

        for top in sorted(top_items):
            src = sp_root / top
            dst = sp_dst / top
            if src in copied_roots or not src.exists():
                continue
            copied_roots.add(src)
            if src.is_dir():
                if not dst.exists():
                    shutil.copytree(src, dst, ignore=ignore_junk)
            else:
                shutil.copy2(src, dst)

    # Write isolated path config so `python -I` finds DLLs, stdlib, site-packages, and ..\app cleanly
    pth_path = RUNTIME_DIR / f"python{sys.version_info.major}{sys.version_info.minor}._pth"
    pth_path.write_text("DLLs\nLib\nLib\\site-packages\n..\\app\nimport site\n", encoding="utf-8")

    compileall.compile_dir(str(APP_DIR), quiet=1, optimize=1)
    print(f"STAGED_RUNTIME packages={len(dists)} target={DIST_DIR}")


if __name__ == "__main__":
    stage()
