#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(dirname -- "$SCRIPT_DIR")
PREFIX=${PREFIX:-"$HOME/.local"}
DESTDIR=${DESTDIR:-}
SKIP_BUILD=${SKIP_BUILD:-0}
if [ -z "${SYSTEMD_USER_UNIT_DIR:-}" ]; then
    if [ "$PREFIX" = "$HOME/.local" ]; then
        SYSTEMD_USER_UNIT_DIR="$HOME/.config/systemd/user"
    else
        SYSTEMD_USER_UNIT_DIR="$PREFIX/lib/systemd/user"
    fi
fi

if [ "$SKIP_BUILD" != "1" ]; then
    cargo build --locked --release --manifest-path "$PROJECT_ROOT/Cargo.toml"
    (
        cd "$PROJECT_ROOT/web"
        bun install --frozen-lockfile
        bun run build
    )
fi

install -Dm755 \
    "$PROJECT_ROOT/target/release/better-wallpaper-daemon" \
    "$DESTDIR$PREFIX/bin/better-wallpaper-daemon"
install -d "$DESTDIR$PREFIX/share/better-wallpaper/web"
cp -R "$PROJECT_ROOT/web/dist/." "$DESTDIR$PREFIX/share/better-wallpaper/web/"
install -d "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper"
cp -R "$PROJECT_ROOT/kde/org.better-wallpaper/." \
    "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/"
install -d "$DESTDIR$SYSTEMD_USER_UNIT_DIR"
sed "s|@PREFIX@|$PREFIX|g" \
    "$SCRIPT_DIR/systemd/better-wallpaper.service.in" \
    > "$DESTDIR$SYSTEMD_USER_UNIT_DIR/better-wallpaper.service"

printf '%s\n' "已安装到 $DESTDIR$PREFIX"
if [ -z "$DESTDIR" ]; then
    systemctl --user daemon-reload
    printf '%s\n' "运行 systemctl --user enable --now better-wallpaper.service 启动服务"
fi
