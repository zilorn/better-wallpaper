//! Small built-in catalogue; daemon logs and project-authored strings are unchanged.
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Language {
    #[value(name = "en")]
    English,
    #[value(name = "zh-CN", alias = "zh", alias = "zh_CN")]
    Chinese,
}

static CHINESE: AtomicBool = AtomicBool::new(false);

pub fn init(explicit: Option<Language>) {
    let language = explicit.unwrap_or_else(|| {
        let values =
            ["LC_ALL", "LC_MESSAGES", "LANG"].map(|key| std::env::var(key).unwrap_or_default());
        locale_language(values.iter().map(String::as_str))
    });
    CHINESE.store(language == Language::Chinese, Ordering::Relaxed);
}

fn locale_language<'a>(values: impl IntoIterator<Item = &'a str>) -> Language {
    let locale = values
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or("en");
    let language = locale.split(['_', '-', '.', '@']).next().unwrap_or("en");
    if language.eq_ignore_ascii_case("zh") {
        Language::Chinese
    } else {
        Language::English
    }
}

pub fn is_chinese() -> bool {
    CHINESE.load(Ordering::Relaxed)
}

pub fn tr(source: &str) -> &str {
    translate(source, is_chinese())
}

fn translate(source: &str, chinese: bool) -> &str {
    if !chinese {
        return source;
    }
    match source {
        "Output name, e.g. DP-1" => "输出名称，例如 DP-1",
        "Enable this output" => "启用此显示器",
        "Remove output" => "移除显示器",
        "Connecting…" => "正在连接…",
        "Pause" => "暂停",
        "Resume" => "继续播放",
        "Closing this window keeps your wallpaper running." => "关闭此窗口不会停止壁纸播放。",
        "Reconnect" => "重新连接",
        "Save changes" => "保存更改",
        "Your wallpapers" => "我的壁纸",
        "Choose a wallpaper, then save to apply it to your desktop." => {
            "选择壁纸，保存后即可应用到桌面。"
        }
        "Search wallpapers" => "搜索壁纸",
        "All types" => "全部类型",
        "Video" => "视频",
        "Web" => "网页",
        "Scene" => "场景",
        "Refresh library" => "刷新壁纸库",
        "Previous" => "上一页",
        "Next" => "下一页",
        "Wallpapers" => "壁纸",
        "Wallpaper settings" => "壁纸设置",
        "Adjust playback and choose a local file or project." => {
            "调整播放设置，选择本地文件或壁纸项目。"
        }
        "Displays" => "显示器",
        "Use the output names reported by your desktop. An empty list selects outputs automatically." => {
            "请使用桌面环境中的输出名称。列表为空时自动选择显示器。"
        }
        "Add output" => "添加显示器",
        "Library folders" => "壁纸目录",
        "One folder per line. Steam Workshop libraries are discovered automatically." => {
            "每行填写一个目录。Steam 创意工坊壁纸库会自动发现。"
        }
        "Settings & diagnostics" => "设置与诊断",
        "Backend changes take effect after restarting the wallpaper service." => {
            "显示后端的更改将在重启壁纸服务后生效。"
        }
        "Service logs" => "服务日志",
        "Recent messages from the wallpaper service." => "查看壁纸服务最近输出的日志。",
        "Refresh automatically" => "自动刷新",
        "Filter log messages" => "筛选日志",
        "Connection details" => "连接详情",
        "Playback" => "播放设置",
        "Settings" => "设置",
        "Logs" => "日志",
        "Choose a wallpaper file…" => "选择壁纸文件…",
        "Browse…" => "浏览…",
        "Wallpaper path" => "壁纸路径",
        "Choose wallpaper" => "选择壁纸",
        "Choose" => "选择",
        "Choose file" => "选择文件",
        "Cancel" => "取消",
        "Wallpaper type" => "壁纸类型",
        "Fill mode" => "缩放方式",
        "Frame rate limit" => "帧率上限",
        "Loop playback" => "循环播放",
        "Mute sound" => "静音",
        "Web settings" => "网页设置",
        "Desktop audio spectrum" => "桌面音频频谱",
        "Media information (MPRIS)" => "媒体信息（MPRIS）",
        "Web properties are saved per project. File properties must stay inside the project folder. Desktop audio and media information are shared with the wallpaper only when enabled." => {
            "网页属性按项目保存。文件属性必须位于项目文件夹内。仅在启用后，才会向壁纸提供桌面音频和媒体信息。"
        }
        "Scene settings" => "场景设置",
        "Scene support depends on the playback backend. Basic pointer parallax is available on exposed niri wallpaper surfaces. Particle limits and scripts are not implemented." => {
            "场景支持情况取决于播放后端。niri 支持鼠标位于桌面背景时的基础视差；粒子数量限制和脚本执行尚未实现。"
        }
        "Quality" => "画质",
        "Mouse input" => "鼠标输入",
        "Basic parallax" => "基础视差",
        "Audio response" => "音频响应",
        "Reset wallpaper properties to defaults" => "恢复壁纸属性默认值",
        "Property overrides cleared. Save changes to apply the defaults." => {
            "已清除自定义属性。保存更改后将应用默认值。"
        }
        "Add folder…" => "添加目录…",
        "Add library folder" => "添加壁纸目录",
        "Add" => "添加",
        "Save changes to update the indexed folders. Removing a folder here does not delete its files." => {
            "保存更改后更新扫描目录。在此移除目录不会删除其中的文件。"
        }
        "Backend" => "显示后端",
        "Hardware decoding" => "硬件解码",
        "Maximum decode height (0 = original)" => "最大解码高度（0 为原始高度）",
        "Log level" => "日志级别",
        "Restore playback on startup" => "启动时恢复播放",
        "Each display needs a unique, nonempty output name." => {
            "显示器的输出名称不能为空，也不能重复。"
        }
        "Wallpaper properties" => "壁纸属性",
        "Connecting to the wallpaper service…" => "正在连接壁纸服务…",
        "Reload settings" => "重新加载设置",
        "Offline" => "未连接",
        "Retry connection" => "重试连接",
        "Discard unsaved changes?" => "放弃尚未保存的更改？",
        "Reloading replaces the settings you have edited with the saved configuration." => {
            "重新加载将用已保存的配置替换当前编辑内容。"
        }
        "Keep editing" => "继续编辑",
        "Reload" => "重新加载",
        "Saving changes…" => "正在保存更改…",
        "Settings saved. Restart the Better Wallpaper service to switch the running backend." => {
            "设置已保存。请重启 Better Wallpaper 服务以切换显示后端。"
        }
        "Settings saved. Wallpaper reload requested." => "设置已保存，已请求重新加载壁纸。",
        "Settings saved." => "设置已保存。",
        "Playback updated." => "播放状态已更新。",
        "Connection restored. Unsaved edits have been kept." => {
            "连接已恢复，尚未保存的更改已保留。"
        }
        "Scanning wallpaper folders…" => "正在扫描壁纸目录…",
        "The library could not be loaded. Use Refresh to try again." => {
            "无法加载壁纸库，请点击刷新重试。"
        }
        "\nShowing the first 1,000 entries (scan limit)." => {
            "\n仅显示前 1,000 个条目（已达到扫描上限）。"
        }
        "No wallpapers found. Add a folder in Library folders, save, then refresh." => {
            "未找到壁纸。请在“壁纸目录”中添加目录，保存后刷新。"
        }
        "No wallpapers match your search." => "没有符合搜索条件的壁纸。",
        "Use current wallpaper" => "使用当前壁纸",
        "Use wallpaper" => "使用此壁纸",
        "Preview" => "预览",
        "Metadata analysis does not guarantee playback on the current backend.\n" => {
            "元数据分析不保证当前后端能够成功播放。\n"
        }
        "Supported features" => "已支持的功能",
        "Unsupported features" => "暂不支持的功能",
        "Warnings" => "警告",
        "Scene compatibility" => "场景兼容性",
        "Wallpaper preview" => "壁纸预览",
        "Background request failed" => "后台请求失败",
        "Stopped" => "已停止",
        "Idle" => "空闲",
        "Paused" => "已暂停",
        "Playing" => "播放中",
        "Invalid management address" => "管理地址无效",
        "The management address must be an HTTP origin on 127.0.0.1" => {
            "管理地址必须是 127.0.0.1 上的 HTTP 服务地址"
        }
        "HOME is not set" => "未设置 HOME 环境变量",
        "Management discovery file is too large" => "服务地址发现文件过大",
        "Cannot read management discovery file" => "无法读取服务地址发现文件",
        "Cannot start Better Wallpaper user service" => "无法启动 Better Wallpaper 用户服务",
        "Service did not become ready within 10 seconds" => "服务未能在 10 秒内就绪",
        "Cannot reach the wallpaper service" => "无法连接壁纸服务",
        "Service response is too large" => "服务响应过大",
        "Invalid service response" => "服务响应无效",
        "This service does not provide the Better Wallpaper v1 API" => {
            "此服务不提供 Better Wallpaper v1 API"
        }
        "Unsupported configuration document" => "不支持此配置文档",
        "Service did not confirm the save" => "服务未确认保存成功",
        "Thumbnail is too large" => "缩略图过大",
        "Automatic" => "自动",
        "Cover" => "裁切铺满",
        "Contain" => "完整显示",
        "Stretch" => "拉伸",
        "High" => "高",
        "Medium" => "中",
        "Low" => "低",
        "Software" => "软件解码",
        "Headless" => "无显示输出",
        "Info" => "常规信息",
        "Debug" => "调试信息",
        _ => source,
    }
}

// Literal format strings keep compile-time checks for both language variants.
macro_rules! localized {
    ($english:literal, $chinese:literal $(, $argument:expr)* $(,)?) => {
        if crate::i18n::is_chinese() { format!($chinese $(, $argument)*) }
        else { format!($english $(, $argument)*) }
    };
}
pub(crate) use localized;

pub fn option_label(value: &str) -> &str {
    tr(match value {
        "auto" => "Automatic",
        "video" => "Video",
        "web" => "Web",
        "scene" => "Scene",
        "cover" => "Cover",
        "contain" => "Contain",
        "stretch" => "Stretch",
        "high" => "High",
        "medium" => "Medium",
        "low" => "Low",
        "software" => "Software",
        "headless" => "Headless",
        "kde" => "KDE Plasma",
        "info" => "Info",
        "debug" => "Debug",
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_precedence_and_variants() {
        assert_eq!(locale_language(["", "", "zh_CN.UTF-8"]), Language::Chinese);
        assert_eq!(
            locale_language(["zh-CN", "en_US", "en_US"]),
            Language::Chinese
        );
        assert_eq!(locale_language(["C", "zh_CN", "zh_CN"]), Language::English);
        assert_eq!(
            locale_language(["", "zh_Hans_CN", "en_US"]),
            Language::Chinese
        );
        assert_eq!(locale_language(["", "", "en_US.UTF-8"]), Language::English);
        assert_eq!(locale_language(["", "", ""]), Language::English);
    }
    #[test]
    fn translations_preserve_protocol_and_authored_text() {
        assert_eq!(translate("Save changes", true), "保存更改");
        assert_eq!(translate("Save changes", false), "Save changes");
        assert_eq!(translate("/api/v1/config", true), "/api/v1/config");
        assert_eq!(translate("作者的壁纸名称", true), "作者的壁纸名称");
        assert_eq!(translate("DP-1", true), "DP-1");
    }
}
