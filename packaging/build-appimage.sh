#!/usr/bin/env bash
# Build dist/DonateHub-x86_64.AppImage: PyInstaller onedir -> AppDir -> appimagetool.
set -euo pipefail
cd "$(dirname "$0")/.."

TOOLS=build/tools
APPDIR=build/AppDir
APPIMAGETOOL="$TOOLS/appimagetool-x86_64.AppImage"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"

uv sync --group build
uv run pyinstaller --noconfirm --clean --distpath build/pyinstaller --workpath build/pyinstaller-work packaging/donatehub.spec

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/lib" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps"
cp -a build/pyinstaller/donatehub "$APPDIR/usr/lib/donatehub"
install -m755 packaging/AppRun "$APPDIR/AppRun"
install -m644 packaging/donatehub.desktop "$APPDIR/donatehub.desktop"
install -m644 packaging/donatehub.desktop "$APPDIR/usr/share/applications/donatehub.desktop"
install -m644 src/donatehub/static/icon.svg "$APPDIR/donatehub.svg"
install -m644 src/donatehub/static/icon.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/donatehub.svg"
ln -sf donatehub.svg "$APPDIR/.DirIcon"

if [ ! -x "$APPIMAGETOOL" ]; then
    mkdir -p "$TOOLS"
    curl -fL --retry 3 -o "$APPIMAGETOOL" "$APPIMAGETOOL_URL"
    chmod +x "$APPIMAGETOOL"
fi

mkdir -p dist
# --appimage-extract-and-run: works even where FUSE is unavailable (CI, containers).
ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run "$APPDIR" dist/DonateHub-x86_64.AppImage
ls -lh dist/DonateHub-x86_64.AppImage
