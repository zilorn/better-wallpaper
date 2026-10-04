import { A, Navigate, Route, Router } from "@solidjs/router";
import {
  For,
  ParentProps,
  Show,
  createContext,
  createMemo,
  onMount, onCleanup,
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
    engine_mode: boolean;
    wallpaper_type: "video" | "web" | "scene";
    loop_playback: boolean;
    muted: boolean;
    fill_mode: string;
    fps_limit: number;
  };
  scene: {
    quality: "low" | "medium" | "high";
    mouse: boolean;
    parallax: boolean;
    audio_processing: boolean;
    particle_limit: number;
    script_enabled: boolean;
    properties: Record<string, string>;
  } | null;
  library: { paths: string[] };
  decode: { hardware: string; max_height: number };
  outputs: Output[];
};
type SceneCompatibility = { level: number; level_name: string; supported_features?: string[]; unsupported_features: string[]; warnings: string[] };
type SceneProperty = { key: string; text: string; prop_type: { kind: string; min?: number; max?: number; step?: number; options?: [string, string][] }; value: string | number | boolean | null; condition?: string | null; order?: number | null };
type LibraryEntry = { name: string; path: string; engine_mode: boolean; wallpaper_type: "video" | "web" | "scene"; preview_path: string | null; size_bytes: number; modified_unix_seconds: number | null; scene_compatibility?: SceneCompatibility; scene_properties?: SceneProperty[] };
type Library = { entries: LibraryEntry[]; roots: string[]; engine_roots: string[]; truncated: boolean };

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
  const run = async (label: string, action: () => Promise<void>): Promise<boolean> => {
    setBusy(true);
    setNotice("");
    console.info(`[Better Wallpaper] ${label}开始`);
    try {
      await action();
      console.info(`[Better Wallpaper] ${label}完成`);
      return true;
    } catch (error) {
      console.error(`[Better Wallpaper] ${label}失败`, error);
      setNotice(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const updateConfig = (update: (current: Config) => Config) => {
    const current = config();
    if (current) setConfig(update(current));
  };
  const persistConfig = (current: Config) => run("保存配置", async () => {
    const result = await requestJson<{ config: Config; restart_required: boolean; reload_requested: boolean }>("/api/v1/config", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(current),
    });
    setConfig(result.config);
    setNotice(result.restart_required
      ? "配置已保存。显示后端尚未切换，请重启 Better Wallpaper 服务使其生效。"
      : result.reload_requested ? "配置已保存，已请求重新加载播放配置。" : "配置已保存。");
  });
  const saveConfig = () => {
    const current = config();
    return current ? persistConfig(current) : Promise.resolve(false);
  };
  const setPaused = (paused: boolean) => run(paused ? "暂停播放" : "恢复播放", async () => {
    await requestJson(`/api/v1/playback/${paused ? "pause" : "resume"}`, { method: "POST" });
    await refetchStatus();
    setNotice(paused ? "播放已暂停。" : "播放已恢复。");
  });

  return { status, config, connected, busy, notice, updateConfig, saveConfig, applyConfig: persistConfig, setPaused };
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
  return <AppContext.Provider value={state}>
    <main class="shell">
      <aside class="sidebar">
        <div class="brand"><div class="brand-mark"><img src="/better-wallpaper-icon.png" alt="" /></div><div><h1>Better Wallpaper</h1><span classList={{ status: true, ok: state.connected() }}>{state.connected() ? state.status()?.backend : "offline"}</span></div></div>
        <nav class="navigation" aria-label="主导航">
          <A href="/wallpapers" activeClass="active"><NavigationIcon name="wallpapers" /><span>壁纸</span></A>
          <A href="/displays" activeClass="active"><NavigationIcon name="displays" /><span>显示器</span></A>
          <A href="/libraries" activeClass="active"><NavigationIcon name="libraries" /><span>壁纸库</span></A>
          <A href="/settings" activeClass="active"><NavigationIcon name="settings" /><span>设置与诊断</span></A>
          <A href="/logs" activeClass="active"><NavigationIcon name="settings" /><span>日志</span></A>
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
  const [library] = createResource(() => requestJson<Library>("/api/v1/library"));
  const updateWallpaper = (key: keyof Config["wallpaper"], value: string | boolean | number | null) =>
    state.updateConfig((current) => ({ ...current, wallpaper: { ...current.wallpaper, [key]: value, ...(key === "path" ? { engine_mode: false, wallpaper_type: "video" as const } : {}) } }));
  const updateScene = <K extends keyof NonNullable<Config["scene"]>>(key: K, value: NonNullable<Config["scene"]>[K]) =>
    state.updateConfig((current) => ({
      ...current,
      scene: { ...(current.scene ?? defaultSceneConfig()), [key]: value },
    }));
  const selectedScene = createMemo(() => library()?.entries.find((entry) => entry.wallpaper_type === "scene" && entry.path === state.config()?.wallpaper.path));
  const selectedProperties = createMemo(() => selectedScene()?.scene_properties ?? []);
  const updateSceneProperty = (key: string, value: string) => updateScene("properties", {
    ...(state.config()?.scene?.properties ?? {}),
    [key]: value,
  });
  const resetSceneProperties = () => updateScene("properties", {});
  return <><PageHeader title="壁纸" description="配置当前视频、网页或场景壁纸，并控制播放任务。" />
    <Show when={state.config()} fallback={<Loading />} >{(config) => <div class="settings-grid">
      <section class="page-panel preview-panel"><div class="video-preview"><span>{config().wallpaper.wallpaper_type === "web" ? "WEB" : config().wallpaper.wallpaper_type === "scene" ? "SCENE" : "VIDEO"}</span><strong>{fileName(config().wallpaper.path)}</strong></div><div class="playback-summary"><div><span>当前状态</span><strong>{playbackLabel(state.status()?.playback)}</strong></div><button class="command" disabled={state.busy() || !state.status()?.playback.running || state.status()?.playback.cancelled} onClick={() => state.setPaused(!state.status()?.playback.paused)}>{state.status()?.playback.paused ? "恢复播放" : "暂停播放"}</button></div></section>
      <section class="page-panel">
        <Show when={config().wallpaper.wallpaper_type === "scene"} fallback={<>
          <h3>视频配置</h3><label>视频路径<input value={config().wallpaper.path ?? ""} onInput={(event) => updateWallpaper("path", event.currentTarget.value || null)} placeholder="/home/user/Videos/wallpaper.mp4" /></label><label>缩放方式<select value={config().wallpaper.fill_mode} onChange={(event) => updateWallpaper("fill_mode", event.currentTarget.value)}><option value="cover">裁切铺满</option><option value="contain">完整显示</option><option value="stretch">拉伸</option></select></label><label class="check"><input type="checkbox" checked={config().wallpaper.loop_playback} onChange={(event) => updateWallpaper("loop_playback", event.currentTarget.checked)} />循环播放</label><label class="check"><input type="checkbox" checked={!config().wallpaper.muted} onChange={(event) => updateWallpaper("muted", !event.currentTarget.checked)} />播放声音</label>
        </>}>
          <h3>场景配置</h3><label>质量档位<select value={config().scene?.quality ?? "high"} onChange={(event) => updateScene("quality", event.currentTarget.value as "low" | "medium" | "high")}><option value="high">高</option><option value="medium">中</option><option value="low">低</option></select></label>
          <p id="unsupported-scene-settings" class="setting-help">基础鼠标视差支持 niri：鼠标位于桌面背景时生效，移入其他窗口后回到中心。壁纸须包含视差参数；粒子数量上限尚未实现。</p>
          <label>粒子数量上限（尚未实现）<input type="number" disabled aria-describedby="unsupported-scene-settings" value={config().scene?.particle_limit ?? 10000} /></label>
          <label class="check"><input type="checkbox" aria-describedby="unsupported-scene-settings" checked={config().scene?.mouse ?? true} onChange={(event) => updateScene("mouse", event.currentTarget.checked)} />鼠标输入（niri 基础视差）</label>
          <label class="check"><input type="checkbox" aria-describedby="unsupported-scene-settings" checked={config().scene?.parallax ?? true} onChange={(event) => updateScene("parallax", event.currentTarget.checked)} />基础视差（niri）</label>
          <label class="check"><input type="checkbox" checked={config().scene?.audio_processing ?? false} onChange={(event) => updateScene("audio_processing", event.currentTarget.checked)} />音频响应</label><label class="check"><input type="checkbox" checked={config().scene?.script_enabled ?? false} onChange={(event) => updateScene("script_enabled", event.currentTarget.checked)} />实验性脚本（尚未执行）</label><Show when={selectedProperties().length}><div class="scene-properties-header"><div><h3>壁纸属性</h3><small>{selectedProperties().length} 个可配置条目</small></div><button class="command secondary" type="button" disabled={Object.keys(config().scene?.properties ?? {}).length === 0} onClick={resetSceneProperties}>恢复默认</button></div><For each={selectedProperties()}>{(property) => <Show when={scenePropertyVisible(property, selectedProperties(), config().scene?.properties ?? {})}><ScenePropertyInput property={property} value={config().scene?.properties[property.key]} onChange={(value) => updateSceneProperty(property.key, value)} /></Show>}</For></Show>
        </Show>
        <button class="command" disabled={state.busy()} onClick={state.saveConfig}>保存配置</button>
      </section>
    </div>}</Show>
  </>;
}

function ScenePropertyInput(props: { property: SceneProperty; value: string | undefined; onChange: (value: string) => void }) {
  const value = () => props.value ?? String(props.property.value ?? "");
  const label = () => props.property.text || props.property.key;
  if (props.property.prop_type.kind === "bool") {
    return <label class="check"><input type="checkbox" checked={value() === "true"} onChange={(event) => props.onChange(String(event.currentTarget.checked))} />{label()}</label>;
  }
  if (props.property.prop_type.kind === "slider") {
    return <label>{label()}<input type="number" min={props.property.prop_type.min} max={props.property.prop_type.max} step={props.property.prop_type.step ?? "any"} value={value()} onInput={(event) => props.onChange(event.currentTarget.value)} /></label>;
  }
  if (props.property.prop_type.kind === "combo") {
    return <label>{label()}<select value={value()} onChange={(event) => props.onChange(event.currentTarget.value)}><For each={props.property.prop_type.options ?? []}>{(option) => <option value={option[1]}>{option[0]}</option>}</For></select></label>;
  }
  if (["color", "text_input", "textinput", "file"].includes(props.property.prop_type.kind)) {
    return <label>{label()}<input value={value()} onInput={(event) => props.onChange(event.currentTarget.value)} /></label>;
  }
  return null;
}

function scenePropertyVisible(property: SceneProperty, properties: SceneProperty[], overrides: Record<string, string>): boolean {
  if (!property.condition) return true;
  const match = property.condition.match(/^([\w.-]+)\.value\s*(==|!=)\s*(.+)$/);
  if (!match) return true;
  const [, key, operator, rawExpected] = match;
  const source = properties.find((candidate) => candidate.key === key);
  const actual = overrides[key] ?? String(source?.value ?? "");
  const expected = rawExpected.trim().replace(/^(?:"([\s\S]*)"|'([\s\S]*)')$/, "$1$2");
  return operator === "==" ? actual === expected : actual !== expected;
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

function LibraryVideoPreview(props: { entry: LibraryEntry }) {
  let video: HTMLVideoElement | undefined;

  onMount(() => {
    console.info(`[Better Wallpaper] Starting library preview: ${props.entry.path}`);
    void video?.play().catch((error) => {
      console.warn("[Better Wallpaper] Library preview playback was blocked", error);
    });
  });
  onCleanup(() => {
    if (!video) return;
    video.pause();
    video.removeAttribute("src");
    video.load();
    console.info(`[Better Wallpaper] Released library preview: ${props.entry.path}`);
  });

  return <video
    ref={video}
    src={`/api/v1/library/media?path=${encodeURIComponent(props.entry.path)}`}
    muted
    loop
    playsinline
    preload="metadata"
  />;
}

function LibrariesPage() {
  const state = useApp();
  const [newPath, setNewPath] = createSignal("");
  const [previewPath, setPreviewPath] = createSignal<string | null>(null);
  const [library, { refetch }] = createResource(() => requestJson<Library>("/api/v1/library"));
  const addPath = () => {
    const path = newPath().trim();
    if (!path) return;
    state.updateConfig((current) => current.library.paths.includes(path) ? current : {
      ...current,
      library: { paths: [...current.library.paths, path] },
    });
    setNewPath("");
  };
  const removePath = (path: string) => state.updateConfig((current) => ({
    ...current,
    library: { paths: current.library.paths.filter((candidate) => candidate !== path) },
  }));
  const saveLibrary = async () => {
    await state.saveConfig();
    await refetch();
  };
  const select = async (entry: LibraryEntry) => {
    const current = state.config();
    if (!current) return;
    const next = {
      ...current,
      wallpaper: { ...current.wallpaper, path: entry.path, engine_mode: entry.engine_mode, wallpaper_type: entry.wallpaper_type },
      scene: entry.wallpaper_type === "scene" && current.wallpaper.path !== entry.path
        ? { ...(current.scene ?? defaultSceneConfig()), properties: {} }
        : current.scene,
    };
    state.updateConfig(() => next);
    console.info(`[Better Wallpaper] Applying wallpaper: ${entry.path}`);
    const saved = await state.applyConfig(next);
    if (!saved) state.updateConfig(() => current);
  };
  return <><PageHeader title="壁纸库" description="扫描本地壁纸，并自动发现 Steam 中的 Wallpaper Engine 项目。" />
    <Show when={state.config()}>{(config) => <section class="page-panel library-paths">
      <h3>扫描目录</h3>
      <Show when={config().library.paths.length} fallback={<div class="empty compact">尚未添加扫描目录。</div>}>
        <For each={config().library.paths}>{(path) => <div class="library-path"><span title={path}>{path}</span><button class="command secondary" disabled={state.busy()} onClick={() => removePath(path)}>移除</button></div>}</For>
      </Show>
      <div class="library-path-add"><input value={newPath()} onInput={(event) => setNewPath(event.currentTarget.value)} onKeyDown={(event) => { if (event.key === "Enter") addPath(); }} placeholder="目录路径，例如 ~/Videos/Wallpapers" /><button class="command secondary" disabled={!newPath().trim() || state.busy()} onClick={addPath}>添加目录</button><button class="command" disabled={state.busy()} onClick={() => void saveLibrary()}>保存并扫描</button></div>
    </section>}</Show>
    <div class="library-toolbar"><div><strong>{library()?.entries.length ?? 0} 个壁纸</strong><small>{[...(library()?.roots ?? []), ...(library()?.engine_roots ?? [])].join("、") || "未找到扫描目录"}</small></div><button class="command secondary" disabled={library.loading} onClick={() => refetch()}>重新扫描</button></div>
    <Show when={library()} fallback={<Loading />}>{(result) => <>
      <Show when={result().truncated}><div class="notice">结果已达到 1000 个条目的扫描上限。</div></Show>
      <section class="library-grid"><For each={result().entries} fallback={<div class="empty">扫描目录中没有支持的壁纸。</div>}>{(entry) => <article classList={{ "library-card": true, selected: state.config()?.wallpaper.path === entry.path }}><div class="library-preview" onPointerEnter={() => { if (entry.wallpaper_type === "video") setPreviewPath(entry.path); }} onPointerLeave={() => setPreviewPath((current) => current === entry.path ? null : current)}><img src={`/api/v1/library/thumbnail?path=${encodeURIComponent(entry.path)}`} alt="" loading="lazy" decoding="async" /><Show when={entry.wallpaper_type === "video" && previewPath() === entry.path}><LibraryVideoPreview entry={entry} /></Show><span>{entry.wallpaper_type === "scene" ? `场景壁纸 · ${entry.scene_compatibility?.level_name ?? "未解析"}` : entry.wallpaper_type === "web" ? "网页壁纸" : entry.engine_mode ? "视频壁纸 · WALLPAPER ENGINE" : "视频壁纸 · 悬停预览"}</span><strong>{entry.name}</strong></div><Show when={entry.wallpaper_type === "scene" && entry.scene_compatibility}>{(compatibility) => <div class="library-compatibility"><small title="按场景已识别的支持功能分级；仍需查看不支持项和警告">兼容等级 L{compatibility().level}</small><Show when={compatibility().supported_features?.length}><small title={compatibility().supported_features?.join("\n")}>已支持功能 {compatibility().supported_features?.length} 项</small></Show><Show when={compatibility().unsupported_features.length}><small title={compatibility().unsupported_features.join("\n")}>不支持功能 {compatibility().unsupported_features.length} 项</small></Show><Show when={compatibility().warnings.length}><small title={compatibility().warnings.join("\n")}>解析警告 {compatibility().warnings.length} 项</small></Show></div>}</Show><div class="library-meta"><small>{formatBytes(entry.size_bytes)}</small><button class="command" disabled={state.busy()} onClick={() => void select(entry)}>{state.config()?.wallpaper.path === entry.path ? "已选择" : "选择并应用"}</button></div></article>}</For></section>
    </>}</Show>
  </>;
}

function SettingsPage() {
  const state = useApp();
  const updateGeneral = (key: keyof Config["general"], value: string | boolean) => state.updateConfig((current) => ({ ...current, general: { ...current.general, [key]: value } }));
  return <><PageHeader title="设置与诊断" description="调整启动和解码选项，检查桌面集成状态。" /><Show when={state.config()} fallback={<Loading />}>{(config) => <div class="settings-grid">
    <section class="page-panel"><h3>运行设置</h3><label>显示后端<select value={config().general.backend} onChange={(event) => updateGeneral("backend", event.currentTarget.value)}><option value="auto">自动检测</option><option value="niri">niri</option><option value="kde">KDE Plasma</option><option value="headless">Headless</option></select></label><p>显示后端切换需要重启服务；诊断信息显示当前运行的后端。</p><label>硬件解码<select value={config().decode.hardware} onChange={(event) => state.updateConfig((current) => ({ ...current, decode: { ...current.decode, hardware: event.currentTarget.value } }))}><option value="auto">自动</option><option value="software">软件解码</option></select></label><label>日志级别<select value={config().general.log_level} onChange={(event) => updateGeneral("log_level", event.currentTarget.value)}><option value="info">Info（默认）</option><option value="debug">Debug（包含帧数日志）</option></select></label><label class="check"><input type="checkbox" checked={config().general.restore_on_start} onChange={(event) => updateGeneral("restore_on_start", event.currentTarget.checked)} />启动时恢复播放</label><button class="command" disabled={state.busy()} onClick={state.saveConfig}>保存设置</button></section>
    <section class="page-panel"><h3>诊断</h3><Show when={state.status()} fallback={<Loading />}>{(status) => <div class="diagnostics"><Diagnostic label="桌面环境" value={status().desktop} /><Diagnostic label="显示后端" value={status().backend} /><Diagnostic label="播放状态" value={playbackLabel(status().playback)} /><Diagnostic label="Plasma 实例" value={status().plasma_instances.map((instance) => instance.output).join("、") || "无"} /><Diagnostic label="检测依据" value={status().evidence} /><Diagnostic label="候选桌面" value={status().candidates.join("、") || "无"} /><Diagnostic label="API 版本" value={String(status().api_version)} /></div>}</Show></section>
  </div>}</Show></>;
}

function Diagnostic(props: { label: string; value: string }) { return <div><span>{props.label}</span><small>{props.value}</small></div>; }
function Loading() { return <div class="empty">正在读取服务数据…</div>; }
function fileName(path: string | null) { return path?.split("/").filter(Boolean).at(-1) ?? "尚未选择视频"; }
function defaultSceneConfig(): NonNullable<Config["scene"]> { return { quality: "high", mouse: true, parallax: true, audio_processing: false, particle_limit: 10000, script_enabled: false, properties: {} }; }
function playbackLabel(playback?: PlaybackStatus) { return !playback ? "未知" : playback.cancelled ? "已停止" : !playback.running ? "空闲" : playback.paused ? "已暂停" : "播放中"; }
function formatBytes(bytes: number) { return bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`; }

function LogsPage() {
  const [logLines, setLogLines] = createSignal<string[]>([]);
  const [paused, setPaused] = createSignal(false);
  let logContainer: HTMLDivElement | undefined;
  let logTimerId: number | undefined;
  onMount(() => {
    const fetchLogs = async () => {
      try {
        const data = await requestJson<{ lines: string[] }>('/api/v1/logs');
        setLogLines(data.lines);
        if (!paused() && logContainer) logContainer.scrollTop = logContainer.scrollHeight;
      } catch { /* network errors ignored */ }
    };
    fetchLogs();
    logTimerId = window.setInterval(fetchLogs, 1000);
  });
  onCleanup(() => { if (logTimerId !== undefined) window.clearInterval(logTimerId); });
  return <>
    <PageHeader title="日志" description="实时查看 Better Wallpaper 服务日志，用于诊断播放性能问题。" />
    <div class="log-page-toolbar">
      <span>{logLines().length} 条日志</span>
      <div class="log-page-actions">
        <button class="command secondary" onClick={() => setPaused(!paused())}>{paused() ? "恢复滚动" : "暂停滚动"}</button>
        <button class="command secondary" onClick={() => setLogLines([])}>清除</button>
      </div>
    </div>
    <div class="log-page-list" ref={logContainer}>
      <Show when={logLines().length > 0} fallback={<div class="empty">等待日志输出…</div>}>
        <For each={logLines()}>{(line) => <code>{line}</code>}</For>
      </Show>
    </div>
  </>;
}

const root = document.getElementById("root");
if (!root) throw new Error("缺少 #root 挂载节点");
render(() => <Router root={AppShell}>
  <Route path="/" component={() => <Navigate href="/wallpapers" />} />
  <Route path="/wallpapers" component={WallpapersPage} />
  <Route path="/displays" component={DisplaysPage} />
  <Route path="/libraries" component={LibrariesPage} />
  <Route path="/settings" component={SettingsPage} />
  <Route path="/logs" component={LogsPage} />
  <Route path="*" component={() => <Navigate href="/wallpapers" />} />
</Router>, root);
