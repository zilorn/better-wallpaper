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
    cmake -S "$PROJECT_ROOT/kde/frame-plugin" -B "$PROJECT_ROOT/target/plasma-plugin" \
        -DCMAKE_BUILD_TYPE=Release
    cmake --build "$PROJECT_ROOT/target/plasma-plugin" --parallel
    (
        cd "$PROJECT_ROOT/web"
        bun install --frozen-lockfile
        bun run build
    )
fi

DAEMON_SOURCE="$PROJECT_ROOT/target/release/better-wallpaper-daemon"
WEB_SOURCE="$PROJECT_ROOT/web/dist/index.html"
PLASMA_SOURCE="$PROJECT_ROOT/kde/org.better-wallpaper/contents/ui/main.qml"
PLASMA_PLUGIN_SOURCE="$PROJECT_ROOT/target/plasma-plugin/libbetterwallpaperplugin.so"

test -x "$DAEMON_SOURCE" || {
    printf '%s\n' "错误：daemon 构建产物不存在：$DAEMON_SOURCE" >&2
    exit 1
}
test -f "$WEB_SOURCE" || {
    printf '%s\n' "错误：Web 构建产物不存在：$WEB_SOURCE" >&2
    printf '%s\n' "请移除 SKIP_BUILD=1，或先在 web 目录运行 bun run build" >&2
    exit 1
}
test -f "$PLASMA_SOURCE" || {
    printf '%s\n' "错误：Plasma 插件入口不存在：$PLASMA_SOURCE" >&2
    exit 1
}
test -f "$PLASMA_PLUGIN_SOURCE" || {
    printf '%s\n' "错误：Plasma 帧插件构建产物不存在：$PLASMA_PLUGIN_SOURCE" >&2
    exit 1
}

SERVICE_WAS_ACTIVE=0
PLASMA_WAS_ACTIVE=0
if [ -z "$DESTDIR" ] && command -v systemctl >/dev/null 2>&1; then
    if systemctl --user is-active --quiet better-wallpaper.service 2>/dev/null; then
        SERVICE_WAS_ACTIVE=1
    fi
    if systemctl --user is-active --quiet plasma-plasmashell.service 2>/dev/null; then
        PLASMA_WAS_ACTIVE=1
    fi
fi

atomic_install() {
    source_path=$1
    target_path=$2
    target_mode=$3
    temporary_path="$target_path.new.$$"
    install -d "$(dirname "$target_path")"
    if ! install -m "$target_mode" "$source_path" "$temporary_path"; then
        rm -f "$temporary_path"
        return 1
    fi
    if ! mv -f "$temporary_path" "$target_path"; then
        rm -f "$temporary_path"
        return 1
    fi
}

atomic_install \
    "$DAEMON_SOURCE" \
    "$DESTDIR$PREFIX/bin/better-wallpaper-daemon" \
    755
install -d "$DESTDIR$PREFIX/share/better-wallpaper/web"
cp -R "$PROJECT_ROOT/web/dist/." "$DESTDIR$PREFIX/share/better-wallpaper/web/"
install -d "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper"
cp -R "$PROJECT_ROOT/kde/org.better-wallpaper/." \
    "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/"
atomic_install "$PLASMA_PLUGIN_SOURCE" \
    "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/contents/ui/BetterWallpaper/libbetterwallpaperplugin.so" \
    755
install -d "$DESTDIR$SYSTEMD_USER_UNIT_DIR"
sed "s|@PREFIX@|$PREFIX|g" \
    "$SCRIPT_DIR/systemd/better-wallpaper.service.in" \
    > "$DESTDIR$SYSTEMD_USER_UNIT_DIR/better-wallpaper.service"

test -x "$DESTDIR$PREFIX/bin/better-wallpaper-daemon"
test -f "$DESTDIR$PREFIX/share/better-wallpaper/web/index.html"
test -f "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/metadata.json"
test -f "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/contents/ui/main.qml"
test -x "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/contents/ui/BetterWallpaper/libbetterwallpaperplugin.so"

printf '%s\n' "已安装到 $DESTDIR$PREFIX"
if [ -z "$DESTDIR" ]; then
    if command -v kbuildsycoca6 >/dev/null 2>&1; then
        kbuildsycoca6 --noincremental >/dev/null
        printf '%s\n' "已刷新 Plasma 6 插件缓存"
    fi
    systemctl --user daemon-reload
    if [ "$SERVICE_WAS_ACTIVE" = "1" ]; then
        systemctl --user restart better-wallpaper.service
        printf '%s\n' "已重启 Better Wallpaper 服务并加载新 daemon"
    fi
    printf '%s\n' "Web UI: $PREFIX/share/better-wallpaper/web/index.html"
    printf '%s\n' "Plasma 插件: $PREFIX/share/plasma/wallpapers/org.better-wallpaper"
    if [ "$PLASMA_WAS_ACTIVE" = "1" ]; then
        printf '%s\n' "Plasma 插件已更新；确认安装无误后执行：systemctl --user restart plasma-plasmashell.service"
    fi
    if [ "$SERVICE_WAS_ACTIVE" = "0" ]; then
        printf '%s\n' "运行 systemctl --user enable --now better-wallpaper.service 启动服务"
    fi
fi
