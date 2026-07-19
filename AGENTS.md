# AGENTS.md

## 项目详情

Better Wallpaper：Linux 视频壁纸程序，支持niri，Plasma
项目使用以下技术栈：

- rust+bun+solidjs

## 要求

- 写日志以便跟踪问题，使用英语日志（无需写web路由访问日志）。
- 安装脚本在`packaging/install.sh`。
- 重要：绝对禁止使用子agent。
- 运行日志看`journalctl --user -u better-wallpaper.service`，注意筛选日志，防止日志过长（调式可看到DEBUG及以上的日志/用户为INFO）。
- 检验：`cargo clippy`通过。

## 目标

制作壁纸软件，支持视频壁纸。
