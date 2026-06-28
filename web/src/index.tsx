import { A, Navigate, Route, Router, useLocation } from "@solidjs/router";
import {
  For,
  ParentProps,
  Show,
  createContext,
  createEffect,
  createMemo,
  onCleanup,
  createResource,
  createSignal,
  useContext,
} from "solid-js";
import { render } from "solid-js/web";
import "./style.css";

type PlaybackStatus = { running: boolean; paused: boolean; cancelled: boolean };
type Status = {
  api_version: number;
  desktop: string;
  evidence: string;
  candidates: string[];
  backend: string;
  playback: PlaybackStatus;
  plasma_instances: { output: string; last_seen_ms: number }[];
};
type Output = { name: string; enabled: boolean };
type Config = {
  version: number;
  general: { backend: string; restore_on_start: boolean; log_level: string };
  wallpaper: {
    path: string | null;
    loop_playback: boolean;
    muted: boolean;
    fill_mode: string;
    fps_limit: number;
  };
  decode: { hardware: string };
  outputs: Output[];
};
type LibraryEntry = { name: string; path: string; size_bytes: number; modified_unix_seconds: number | null };
type Library = { entries: LibraryEntry[]; roots: string[]; truncated: boolean };

async function requestJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init);
  const result = await response.json().catch(() => ({}));
  if (!response.ok) {
    const message = "error" in result ? String(result.error) : `请求失败 (${response.status})`;
    throw new Error(message);
  }
  return result as T;
}

function createAppState() {
  const [status, { mutate: setStatus, refetch: refetchStatus }] = createResource(() => requestJson<Status>("/api/v1/status"));
  const [config, { mutate: setConfig }] = createResource(() => requestJson<Config>("/api/v1/config"));
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal("");

  const connected = createMemo(() => Boolean(status()) && !status.error);
  let reconnectTimer: number | undefined;
  let reconnectAttempt = 0;
  let socket: WebSocket | undefined;
  const connectStatusStream = () => {
    const protocol = window.location.protocol === "https:" ? "wss" : "ws";
    socket = new WebSocket(`${protocol}://${window.location.host}/api/v1/ws`);
    socket.addEventListener("open", () => {
      reconnectAttempt = 0;
      console.info("[Better Wallpaper] WebSocket 状态流已连接");
    });
    socket.addEventListener("message", (event) => {
      try {
        const message = JSON.parse(String(event.data)) as { type?: string; data?: Status };
        if (message.type === "status" && message.data) setStatus(message.data);
      } catch (error) {
        console.error("[Better Wallpaper] WebSocket 状态消息无效", error);
      }
    });
    socket.addEventListener("close", () => {
      const delay = Math.min(30000, 1000 * 2 ** reconnectAttempt++);
      console.warn(`[Better Wallpaper] WebSocket 状态流断开，${delay}ms 后重连`);
      reconnectTimer = window.setTimeout(connectStatusStream, delay);
    });
    socket.addEventListener("error", () => socket?.close());
  };
  connectStatusStream();
  onCleanup(() => {
    if (reconnectTimer !== undefined) window.clearTimeout(reconnectTimer);
    socket?.close();
  });
  const run = async (label: string, action: () => Promise<void>) => {
    setBusy(true);
    setNotice("");
    console.info(`[Better Wallpaper] ${label}开始`);
    try {
      await action();
      console.info(`[Better Wallpaper] ${label}完成`);
    } catch (error) {
      console.error(`[Better Wallpaper] ${label}失败`, error);
      setNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  const updateConfig = (update: (current: Config) => Config) => {
    const current = config();
    if (current) setConfig(update(current));
  };
  const saveConfig = () => run("保存配置", async () => {
    const current = config();
    if (!current) return;
    await requestJson("/api/v1/config", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(current),
    });
    setNotice("配置已保存；播放相关变更将在重启播放进程后生效。");
  });
  const setPaused = (paused: boolean) => run(paused ? "暂停播放" : "恢复播放", async () => {
    await requestJson(`/api/v1/playback/${paused ? "pause" : "resume"}`, { method: "POST" });
    await refetchStatus();
    setNotice(paused ? "播放已暂停。" : "播放已恢复。");
  });

  return { status, config, connected, busy, notice, updateConfig, saveConfig, setPaused };
}

type AppState = ReturnType<typeof createAppState>;
const AppContext = createContext<AppState>();
function useApp() {
  const state = useContext(AppContext);
  if (!state) throw new Error("AppContext 未初始化");
  return state;
}

type IconName = "wallpapers" | "displays" | "libraries" | "settings";
function NavigationIcon(props: { name: IconName }) {
  return <svg class="navigation-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <Show when={props.name === "wallpapers"}><path d="M4.75 5.75A1.75 1.75 0 0 1 6.5 4h11a1.75 1.75 0 0 1 1.75 1.75v12.5H4.75V5.75Z" /><path d="m5 16 4.25-4.25 2.8 2.8 2.2-2.2L19 17.1M15.75 8.25h.01" /></Show>
    <Show when={props.name === "displays"}><rect x="3.5" y="4.5" width="17" height="12" rx="1.75" /><path d="M8.5 20h7M12 16.5V20" /></Show>
    <Show when={props.name === "libraries"}><path d="M3.75 7.25h6l1.5 2h9v8.5A2.25 2.25 0 0 1 19 20H5a2.25 2.25 0 0 1-1.25-2.25V7.25Z" /><path d="M5.25 7.25V5.5A1.5 1.5 0 0 1 6.75 4h4.5l1.5 2h4.5a1.5 1.5 0 0 1 1.5 1.5v1.75" /></Show>
    <Show when={props.name === "settings"}><circle cx="12" cy="12" r="3" /><path d="M19.25 13.1v-2.2l-2.05-.55a5.73 5.73 0 0 0-.6-1.45l1.06-1.84-1.56-1.55-1.84 1.06a5.73 5.73 0 0 0-1.45-.6L12.26 3.9h-2.2l-.55 2.06a5.73 5.73 0 0 0-1.45.6L6.22 5.5 4.67 7.06 5.73 8.9a5.73 5.73 0 0 0-.6 1.45l-2.06.55v2.2l2.06.55c.14.51.34 1 .6 1.45l-1.06 1.84 1.55 1.55 1.84-1.06c.45.26.94.46 1.45.6l.55 2.06h2.2l.55-2.06c.51-.14 1-.34 1.45-.6l1.84 1.06 1.56-1.55-1.06-1.84c.26-.45.46-.94.6-1.45l2.05-.55Z" /></Show>
  </svg>;
}

function AppShell(props: ParentProps) {
  const state = createAppState();
  const location = useLocation();
  createEffect(() => console.info(`[Better Wallpaper] 路由切换：${location.pathname}`));
  return <AppContext.Provider value={state}>
    <main class="shell">
      <aside class="sidebar">
        <div class="brand"><div class="brand-mark"><img src="/better-wallpaper-icon.png" alt="" /></div><div><h1>Better Wallpaper</h1><span classList={{ status: true, ok: state.connected() }}>{state.connected() ? state.status()?.backend : "offline"}</span></div></div>
        <nav class="navigation" aria-label="主导航">
          <A href="/wallpapers" activeClass="active"><NavigationIcon name="wallpapers" /><span>壁纸</span></A>
          <A href="/displays" activeClass="active"><NavigationIcon name="displays" /><span>显示器</span></A>
          <A href="/libraries" activeClass="active"><NavigationIcon name="libraries" /><span>壁纸库</span></A>
          <A href="/settings" activeClass="active"><NavigationIcon name="settings" /><span>设置与诊断</span></A>
        </nav>
      </aside>
      <section class="content"><Show when={state.notice()}><div class="notice" role="status">{state.notice()}</div></Show>{props.children}</section>
    </main>
  </AppContext.Provider>;
}

function PageHeader(props: { title: string; description: string }) {
  return <header class="page-header"><h2>{props.title}</h2><p>{props.description}</p></header>;
}

function WallpapersPage() {
  const state = useApp();
  const updateWallpaper = (key: keyof Config["wallpaper"], value: string | boolean | number | null) =>
    state.updateConfig((current) => ({ ...current, wallpaper: { ...current.wallpaper, [key]: value } }));
  return <><PageHeader title="壁纸" description="选择视频、调整画面方式并控制当前播放任务。" />
    <Show when={state.config()} fallback={<Loading />} >{(config) => <div class="settings-grid">
      <section class="page-panel preview-panel"><div class="video-preview"><span>VIDEO</span><strong>{fileName(config().wallpaper.path)}</strong></div><div class="playback-summary"><div><span>当前状态</span><strong>{playbackLabel(state.status()?.playback)}</strong></div><button class="command" disabled={state.busy() || !state.status()?.playback.running || state.status()?.playback.cancelled} onClick={() => state.setPaused(!state.status()?.playback.paused)}>{state.status()?.playback.paused ? "恢复播放" : "暂停播放"}</button></div></section>
      <section class="page-panel"><h3>视频配置</h3><label>视频路径<input value={config().wallpaper.path ?? ""} onInput={(event) => updateWallpaper("path", event.currentTarget.value || null)} placeholder="/home/user/Videos/wallpaper.mp4" /></label><label>缩放方式<select value={config().wallpaper.fill_mode} onChange={(event) => updateWallpaper("fill_mode", event.currentTarget.value)}><option value="cover">裁切铺满</option><option value="contain">完整显示</option><option value="stretch">拉伸</option></select></label><label class="check"><input type="checkbox" checked={config().wallpaper.loop_playback} onChange={(event) => updateWallpaper("loop_playback", event.currentTarget.checked)} />循环播放</label><button class="command" disabled={state.busy()} onClick={state.saveConfig}>保存配置</button></section>
    </div>}</Show>
  </>;
}

function DisplaysPage() {
  const state = useApp();
  const [newOutput, setNewOutput] = createSignal("");
  const toggle = (name: string) => state.updateConfig((current) => ({ ...current, outputs: current.outputs.map((output) => output.name === name ? { ...output, enabled: !output.enabled } : output) }));
  const add = () => {
    const name = newOutput().trim();
    if (!name) return;
    state.updateConfig((current) => current.outputs.some((output) => output.name === name) ? current : { ...current, outputs: [...current.outputs, { name, enabled: true }] });
    setNewOutput("");
  };
  return <><PageHeader title="显示器" description="配置启动时应创建视频背景层的输出名称。" /><Show when={state.config()} fallback={<Loading />}>{(config) => <>
    <section class="page-panel output-list"><Show when={config().outputs.length} fallback={<div class="empty">尚未配置输出；留空时自动选择当前输出。</div>}><For each={config().outputs}>{(output) => <button classList={{ display: true, active: output.enabled }} onClick={() => toggle(output.name)} disabled={state.busy()}><span>{output.name}</span><small>{output.enabled ? "已启用" : "已停用"}</small></button>}</For></Show></section>
    <div class="page-actions"><input value={newOutput()} onInput={(event) => setNewOutput(event.currentTarget.value)} placeholder="输出名称，例如 DP-1" /><button class="command secondary" disabled={!newOutput().trim()} onClick={add}>添加输出</button><button class="command" disabled={state.busy()} onClick={state.saveConfig}>保存显示器配置</button></div>
  </>}</Show></>;
}

function LibrariesPage() {
  const state = useApp();
  const [library, { refetch }] = createResource(() => requestJson<Library>("/api/v1/library"));
  const select = (entry: LibraryEntry) => {
    state.updateConfig((current) => ({ ...current, wallpaper: { ...current.wallpaper, path: entry.path } }));
    console.info(`[Better Wallpaper] 已从壁纸库选择：${entry.path}`);
  };
  return <><PageHeader title="壁纸库" description="扫描本地视频集合，并选择要应用的壁纸。" />
    <div class="library-toolbar"><div><strong>{library()?.entries.length ?? 0} 个视频</strong><small>{library()?.roots.join("、") || "正在读取扫描目录…"}</small></div><button class="command secondary" disabled={library.loading} onClick={() => refetch()}>重新扫描</button></div>
    <Show when={library()} fallback={<Loading />}>{(result) => <>
      <Show when={result().truncated}><div class="notice">结果已达到 1000 个条目的扫描上限。</div></Show>
      <section class="library-grid"><For each={result().entries} fallback={<div class="empty">扫描目录中没有支持的视频文件。</div>}>{(entry) => <article classList={{ "library-card": true, selected: state.config()?.wallpaper.path === entry.path }}><div class="library-preview"><video src={`/api/v1/library/media?path=${encodeURIComponent(entry.path)}`} muted loop preload="metadata" onMouseEnter={(event) => void event.currentTarget.play()} onMouseLeave={(event) => { event.currentTarget.pause(); event.currentTarget.currentTime = 0; }} /><span>悬停预览</span><strong>{entry.name}</strong></div><div class="library-meta"><small>{formatBytes(entry.size_bytes)}</small><button class="command" disabled={state.busy()} onClick={() => select(entry)}>{state.config()?.wallpaper.path === entry.path ? "已选择" : "选择"}</button></div></article>}</For></section>
      <div class="page-actions library-actions"><span></span><button class="command" disabled={state.busy() || !state.config()} onClick={state.saveConfig}>保存所选壁纸</button></div>
    </>}</Show>
  </>;
}

function SettingsPage() {
  const state = useApp();
  const updateGeneral = (key: keyof Config["general"], value: string | boolean) => state.updateConfig((current) => ({ ...current, general: { ...current.general, [key]: value } }));
  return <><PageHeader title="设置与诊断" description="调整启动和解码选项，检查桌面集成状态。" /><Show when={state.config()} fallback={<Loading />}>{(config) => <div class="settings-grid">
    <section class="page-panel"><h3>运行设置</h3><label>显示后端<select value={config().general.backend} onChange={(event) => updateGeneral("backend", event.currentTarget.value)}><option value="auto">自动检测</option><option value="niri">niri</option><option value="kde">KDE Plasma</option><option value="headless">Headless</option></select></label><label>硬件解码<select value={config().decode.hardware} onChange={(event) => state.updateConfig((current) => ({ ...current, decode: { hardware: event.currentTarget.value } }))}><option value="auto">自动</option><option value="software">软件解码</option></select></label><label>日志级别<select value={config().general.log_level} onChange={(event) => updateGeneral("log_level", event.currentTarget.value)}><option value="error">Error</option><option value="warn">Warn</option><option value="info">Info</option><option value="debug">Debug</option><option value="trace">Trace</option></select></label><label class="check"><input type="checkbox" checked={config().general.restore_on_start} onChange={(event) => updateGeneral("restore_on_start", event.currentTarget.checked)} />启动时恢复播放</label><button class="command" disabled={state.busy()} onClick={state.saveConfig}>保存设置</button></section>
    <section class="page-panel"><h3>诊断</h3><Show when={state.status()} fallback={<Loading />}>{(status) => <div class="diagnostics"><Diagnostic label="桌面环境" value={status().desktop} /><Diagnostic label="显示后端" value={status().backend} /><Diagnostic label="播放状态" value={playbackLabel(status().playback)} /><Diagnostic label="Plasma 实例" value={status().plasma_instances.map((instance) => instance.output).join("、") || "无"} /><Diagnostic label="检测依据" value={status().evidence} /><Diagnostic label="候选桌面" value={status().candidates.join("、") || "无"} /><Diagnostic label="API 版本" value={String(status().api_version)} /></div>}</Show></section>
  </div>}</Show></>;
}

function Diagnostic(props: { label: string; value: string }) { return <div><span>{props.label}</span><small>{props.value}</small></div>; }
function Loading() { return <div class="empty">正在读取服务数据…</div>; }
function fileName(path: string | null) { return path?.split("/").filter(Boolean).at(-1) ?? "尚未选择视频"; }
function playbackLabel(playback?: PlaybackStatus) { return !playback ? "未知" : playback.cancelled ? "已停止" : !playback.running ? "空闲" : playback.paused ? "已暂停" : "播放中"; }
function formatBytes(bytes: number) { return bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`; }

const root = document.getElementById("root");
if (!root) throw new Error("缺少 #root 挂载节点");
render(() => <Router root={AppShell}>
  <Route path="/" component={() => <Navigate href="/wallpapers" />} />
  <Route path="/wallpapers" component={WallpapersPage} />
  <Route path="/displays" component={DisplaysPage} />
  <Route path="/libraries" component={LibrariesPage} />
  <Route path="/settings" component={SettingsPage} />
  <Route path="*" component={() => <Navigate href="/wallpapers" />} />
</Router>, root);
