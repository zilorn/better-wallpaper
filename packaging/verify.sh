#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(dirname -- "$SCRIPT_DIR")
STAGE=$(mktemp -d "${TMPDIR:-/tmp}/better-wallpaper-package.XXXXXX")

cleanup() {
    rm -rf "$STAGE"
}
trap cleanup EXIT HUP INT TERM

log() {
    printf '%s\n' "[packaging-verify] $*"
}

log "构建 release daemon 和桌面客户端"
cargo build --locked --release --manifest-path "$PROJECT_ROOT/Cargo.toml"

log "构建网页壁纸渲染器"
cmake -S "$PROJECT_ROOT/native/web-wallpaper" -B "$PROJECT_ROOT/target/web-wallpaper" -DCMAKE_BUILD_TYPE=Release
cmake --build "$PROJECT_ROOT/target/web-wallpaper" --parallel 2

log "构建 Web 静态资源"
(
    cd "$PROJECT_ROOT/web"
    bun install --frozen-lockfile
    bun run build
)
log "安装到临时 staging: $STAGE"
# Simulate an upgrade from a package containing the unused native module.
LEGACY_PLASMA_MODULE="$STAGE/usr/share/plasma/wallpapers/org.better-wallpaper/contents/ui/BetterWallpaper"
mkdir -p "$LEGACY_PLASMA_MODULE"
touch "$LEGACY_PLASMA_MODULE/qmldir" "$LEGACY_PLASMA_MODULE/libbetterwallpaperplugin.so"
DESTDIR="$STAGE" PREFIX=/usr SKIP_BUILD=1 "$SCRIPT_DIR/install.sh"

DAEMON="$STAGE/usr/bin/better-wallpaper-daemon"
DESKTOP="$STAGE/usr/bin/better-wallpaper-desktop"
ENTRY="$STAGE/usr/share/applications/org.betterwallpaper.desktop.desktop"
UNIT="$STAGE/usr/lib/systemd/user/better-wallpaper.service"
WEB="$STAGE/usr/share/better-wallpaper/web/index.html"
PLASMA="$STAGE/usr/share/plasma/wallpapers/org.better-wallpaper"
ROOT_INSTALLER="$PROJECT_ROOT/install.sh"

test -x "$ROOT_INSTALLER"
test -x "$DAEMON"
test -x "$DESKTOP"
test -x "$STAGE/usr/bin/better-wallpaper-web"
test -f "$ENTRY"
test -f "$STAGE/usr/share/icons/hicolor/64x64/apps/better-wallpaper.png"
grep -q '^Exec="/usr/bin/better-wallpaper-desktop"$' "$ENTRY"
if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$ENTRY"
fi
test -f "$UNIT"
test -f "$WEB"
test -f "$PLASMA/metadata.json"
test -f "$PLASMA/contents/ui/main.qml"
test ! -e "$PLASMA/contents/ui/BetterWallpaper"
test -z "$(find "$STAGE" -name '*.new.*' -print -quit)"
grep -q '^ExecStart=/usr/bin/better-wallpaper-daemon$' "$UNIT"
grep -q '^ExecStartPre=/usr/bin/test -f /usr/share/better-wallpaper/web/index.html$' "$UNIT"
grep -q '^Environment=BETTER_WALLPAPER_WEB_ROOT=/usr/share/better-wallpaper/web$' "$UNIT"
if grep -q 'libbetterwallpaperplugin' "$UNIT"; then
    log "systemd 单元仍依赖已移除的 Plasma 帧插件"
    exit 1
fi

log "验证 systemd 单元、动态依赖和 Plasma QML"
if ! SYSTEMD_OUTPUT=$(systemd-analyze verify "$UNIT" 2>&1); then
    SYSTEMD_ERRORS=$(printf '%s\n' "$SYSTEMD_OUTPUT" | grep -v \
        -e 'Failed to turn off SO_PASSRIGHTS on user lookup socket' \
        -e 'Failed to enable SO_PASSCRED on handoff timestamp socket' || true)
    if [ -n "$SYSTEMD_ERRORS" ]; then
        printf '%s\n' "$SYSTEMD_OUTPUT"
        exit 1
    fi
    log "受限环境无法访问 systemd 用户查询 socket；单元静态检查未发现其他错误"
fi
if ldd "$DAEMON" | grep -q 'not found' || ldd "$DESKTOP" | grep -q 'not found' || ldd "$STAGE/usr/bin/better-wallpaper-web" | grep -q 'not found'; then
    log "发现缺失的动态链接库"
    ldd "$DAEMON"
    ldd "$DESKTOP"
    exit 1
fi
qmllint "$PLASMA/contents/ui/main.qml" "$PROJECT_ROOT/native/web-wallpaper/WebWallpaper.qml"

log "验证 staging 卸载"
DESTDIR="$STAGE" PREFIX=/usr "$SCRIPT_DIR/uninstall.sh"
test ! -e "$DAEMON"
test ! -e "$DESKTOP"
test ! -e "$STAGE/usr/bin/better-wallpaper-web"
test ! -e "$ENTRY"
test ! -e "$STAGE/usr/share/icons/hicolor/64x64/apps/better-wallpaper.png"
test ! -e "$STAGE/usr/share/better-wallpaper"
test ! -e "$PLASMA"
test ! -e "$UNIT"

log "发行版产物验收通过"
