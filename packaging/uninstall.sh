#!/bin/sh
set -eu

PREFIX=${PREFIX:-"$HOME/.local"}
DESTDIR=${DESTDIR:-}
if [ -z "${SYSTEMD_USER_UNIT_DIR:-}" ]; then
    if [ "$PREFIX" = "$HOME/.local" ]; then
        SYSTEMD_USER_UNIT_DIR="$HOME/.config/systemd/user"
    else
        SYSTEMD_USER_UNIT_DIR="$PREFIX/lib/systemd/user"
    fi
fi

if [ -z "$DESTDIR" ] && command -v systemctl >/dev/null 2>&1; then
    systemctl --user disable --now better-wallpaper.service 2>/dev/null || true
fi

rm -f "$DESTDIR$PREFIX/bin/better-wallpaper-daemon"
rm -f "$DESTDIR$PREFIX/bin/better-wallpaper-desktop"
rm -f "$DESTDIR$PREFIX/share/applications/org.betterwallpaper.desktop.desktop"
rm -f "$DESTDIR$PREFIX/share/icons/hicolor/64x64/apps/better-wallpaper.png"
rm -rf "$DESTDIR$PREFIX/share/better-wallpaper"
rm -rf "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper"
rm -f "$DESTDIR$SYSTEMD_USER_UNIT_DIR/better-wallpaper.service"

if [ -z "$DESTDIR" ] && command -v systemctl >/dev/null 2>&1; then
    systemctl --user daemon-reload
fi

printf '%s\n' "已卸载 Better Wallpaper；用户配置未删除"
