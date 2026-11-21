#!/usr/bin/env bash
# Build dist/DonationGoal-x86_64.AppImage: PyInstaller onedir -> AppDir -> appimagetool.
set -euo pipefail
cd "$(dirname "$0")/.."

TOOLS=build/tools
APPDIR=build/AppDir
APPIMAGETOOL="$TOOLS/appimagetool-x86_64.AppImage"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"

uv sync --group build
uv run pyinstaller --noconfirm --clean --distpath build/pyinstaller --workpath build/pyinstaller-work packaging/donation-goal.spec

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/lib" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps"
cp -a build/pyinstaller/donation-goal "$APPDIR/usr/lib/donation-goal"
install -m755 packaging/AppRun "$APPDIR/AppRun"
install -m644 packaging/donation-goal.desktop "$APPDIR/donation-goal.desktop"
install -m644 packaging/donation-goal.desktop "$APPDIR/usr/share/applications/donation-goal.desktop"
install -m644 src/donation_goal/static/icon.svg "$APPDIR/donation-goal.svg"
install -m644 src/donation_goal/static/icon.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/donation-goal.svg"
ln -sf donation-goal.svg "$APPDIR/.DirIcon"

if [ ! -x "$APPIMAGETOOL" ]; then
    mkdir -p "$TOOLS"
    curl -fL --retry 3 -o "$APPIMAGETOOL" "$APPIMAGETOOL_URL"
    chmod +x "$APPIMAGETOOL"
fi

mkdir -p dist
# --appimage-extract-and-run: works even where FUSE is unavailable (CI, containers).
ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run "$APPDIR" dist/DonationGoal-x86_64.AppImage
ls -lh dist/DonationGoal-x86_64.AppImage
