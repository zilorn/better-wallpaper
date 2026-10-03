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

DAEMON_SOURCE="$PROJECT_ROOT/target/release/better-wallpaper-daemon"
SCENE_INSPECT_SOURCE="$PROJECT_ROOT/target/release/scene-inspect"
SCENE_VALIDATE_SOURCE="$PROJECT_ROOT/target/release/scene-validate"
WEB_SOURCE="$PROJECT_ROOT/web/dist/index.html"
PLASMA_SOURCE="$PROJECT_ROOT/kde/org.better-wallpaper/contents/ui/main.qml"

test -x "$DAEMON_SOURCE" || {
    printf '%s\n' "错误：daemon 构建产物不存在：$DAEMON_SOURCE" >&2
    exit 1
}
test -x "$SCENE_INSPECT_SOURCE" || {
    printf '%s\n' "错误：scene-inspect 构建产物不存在：$SCENE_INSPECT_SOURCE" >&2
    exit 1
}
test -x "$SCENE_VALIDATE_SOURCE" || {
    printf '%s\n' "错误：scene-validate 构建产物不存在：$SCENE_VALIDATE_SOURCE" >&2
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
atomic_install "$SCENE_INSPECT_SOURCE" "$DESTDIR$PREFIX/bin/scene-inspect" 755
atomic_install "$SCENE_VALIDATE_SOURCE" "$DESTDIR$PREFIX/bin/scene-validate" 755
install -d "$DESTDIR$PREFIX/share/better-wallpaper/web"
cp -R "$PROJECT_ROOT/web/dist/." "$DESTDIR$PREFIX/share/better-wallpaper/web/"
install -d "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper"
# Remove only the obsolete native module's installed files when upgrading.
LEGACY_PLASMA_MODULE="$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/contents/ui/BetterWallpaper"
if [ -d "$LEGACY_PLASMA_MODULE" ]; then
    rm -f "$LEGACY_PLASMA_MODULE/qmldir" "$LEGACY_PLASMA_MODULE/libbetterwallpaperplugin.so"
    rmdir "$LEGACY_PLASMA_MODULE" 2>/dev/null || true
fi
cp -R "$PROJECT_ROOT/kde/org.better-wallpaper/." \
    "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/"
install -d "$DESTDIR$SYSTEMD_USER_UNIT_DIR"
sed "s|@PREFIX@|$PREFIX|g" \
    "$SCRIPT_DIR/systemd/better-wallpaper.service.in" \
    > "$DESTDIR$SYSTEMD_USER_UNIT_DIR/better-wallpaper.service"

test -x "$DESTDIR$PREFIX/bin/better-wallpaper-daemon"
test -x "$DESTDIR$PREFIX/bin/scene-inspect"
test -x "$DESTDIR$PREFIX/bin/scene-validate"
test -f "$DESTDIR$PREFIX/share/better-wallpaper/web/index.html"
test -f "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/metadata.json"
test -f "$DESTDIR$PREFIX/share/plasma/wallpapers/org.better-wallpaper/contents/ui/main.qml"

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
