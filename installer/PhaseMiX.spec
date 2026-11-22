# -*- mode: python ; coding: utf-8 -*-
import os
from PyInstaller.utils.hooks import collect_all

ROOT = os.path.abspath(os.path.join(SPECPATH, ".."))

wv_datas, wv_bins, wv_hidden = collect_all("webview")

a = Analysis(
    [os.path.join(ROOT, "desktop.py")],
    pathex=[ROOT],
    binaries=wv_bins,
    datas=[
        (os.path.join(ROOT, "frontend", "dist"), os.path.join("frontend", "dist")),
        (os.path.join(SPECPATH, "phasemix.ico"), "assets"),
        *wv_datas,
    ],
    hiddenimports=[
        *wv_hidden,
        "pyaudiowpatch",
        "pycaw.pycaw",
        "comtypes",
        "scipy.signal",
        "waitress",
        "pystray",
        "clr",
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        "tkinter",
        "matplotlib",
        "IPython",
        "pandas",
        "pytest",
        "sounddevice",
        "torch",
        "torchaudio",
        "torchvision",
        "numba",
        "llvmlite",
        "sympy",
        "sklearn",
        "cv2",
        "transformers",
        "tensorflow",
        "onnxruntime",
        "PyQt5",
        "PyQt6",
        "PySide2",
        "PySide6",
        "wx",
        "gi",
    ],
    noarchive=False,
    optimize=1,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="PhaseMiX",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
    icon=os.path.join(SPECPATH, "phasemix.ico"),
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name="PhaseMiX",
)
