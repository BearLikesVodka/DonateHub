# PyInstaller spec: onedir bundle of the desktop app (packaging/build-appimage.sh wraps it into an AppImage).
from pathlib import Path

ROOT = Path(SPECPATH).parent
SRC = ROOT / "src"

# Qt modules the app never imports; dropping them keeps the AppImage smaller.
QT_EXCLUDES = [
    f"PySide6.{m}" for m in (
        "Qt3DAnimation", "Qt3DCore", "Qt3DExtras", "Qt3DInput", "Qt3DLogic", "Qt3DRender", "QtBluetooth",
        "QtCharts", "QtDataVisualization", "QtDesigner", "QtGraphs", "QtGraphsWidgets", "QtHelp", "QtHttpServer",
        "QtLocation", "QtMultimedia", "QtMultimediaWidgets", "QtNfc", "QtPdf", "QtPdfWidgets", "QtQuick3D",
        "QtQuickTest", "QtRemoteObjects", "QtScxml", "QtSensors", "QtSerialBus", "QtSerialPort",
        "QtSpatialAudio", "QtSql", "QtStateMachine", "QtTest", "QtTextToSpeech", "QtUiTools",
        "QtWebEngineQuick", "QtWebSockets", "QtWebView", "QtCanvasPainter", "QtLabsStyleKit",
    )
]

a = Analysis(
    [str(ROOT / "packaging" / "entry.py")],
    pathex=[str(SRC)],
    datas=[(str(SRC / "donatehub" / "static"), "donatehub/static")],
    hiddenimports=[
        "donatehub.desktop",
        "uvicorn.logging",
        "uvicorn.loops.auto",
        "uvicorn.loops.asyncio",
        "uvicorn.protocols.http.auto",
        "uvicorn.protocols.http.h11_impl",
        "uvicorn.protocols.http.httptools_impl",
        "uvicorn.protocols.websockets.auto",
        "uvicorn.protocols.websockets.websockets_impl",
        "uvicorn.lifespan.on",
    ],
    excludes=QT_EXCLUDES + ["tkinter", "pytest"],
    noarchive=False,
)
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, [], exclude_binaries=True, name="donatehub", console=False, strip=False, upx=False)
coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name="donatehub")
