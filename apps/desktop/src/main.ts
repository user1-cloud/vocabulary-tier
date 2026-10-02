import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import PopupApp from './PopupApp.svelte';

/**
 * 入口：按窗口 label 分流。
 *
 *   label === 'popup' → 悬浮小窗（无边框，只有输入框 + 着色卡片）
 *   其它               → 主窗口
 *
 * 判断方式优先用 @tauri-apps/api 的 getCurrentWindow().label；在浏览器里
 * 没有 Tauri 环境，直接挂主窗口。Tauri v2 内部也会在
 * window.__TAURI_INTERNALS__.metadata.currentWindow 里暴露 label，
 * 用它做同步兜底，避免首帧闪错组件。
 */

const target = document.getElementById('app');
if (!target) {
  throw new Error('找不到 #app 挂载点，请检查 index.html');
}

/** 同步读取当前窗口 label；读不到返回 'main' */
function readWindowLabel(): string {
  if (typeof window === 'undefined') return 'main';
  const internals = (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  if (!internals) return 'main';
  try {
    const metadata = (internals as { metadata?: { currentWindow?: { label?: string } } }).metadata;
    return metadata?.currentWindow?.label ?? 'main';
  } catch {
    return 'main';
  }
}

/**
 * 浏览器预览开关：`http://localhost:1420/?window=popup` 直接渲染悬浮小窗。
 *
 * 真实运行时窗口 label 由 Tauri 提供（src-tauri/tauri.conf.json 里的 popup 窗口）；
 * 这个 query 参数只在没有 Tauri 环境（即 isTauri() === false）时生效，
 * 方便在浏览器里检查小窗 UI，不会影响桌面端行为。
 */
function readPreviewLabel(): string {
  if (typeof window === 'undefined') return 'main';
  try {
    const params = new URLSearchParams(window.location.search);
    return params.get('window') === 'popup' ? 'popup' : 'main';
  } catch {
    return 'main';
  }
}

const label = (() => {
  const real = readWindowLabel();
  // 真实 Tauri 环境里 label 一定是 'popup' 或 'main'；
  // 浏览器里 readWindowLabel() 恒为 'main'，此时才允许用 query 参数预览小窗。
  if (real !== 'main') return real;
  return readPreviewLabel();
})();

if (label === 'popup') {
  document.documentElement.dataset.window = 'popup';
  document.body.classList.add('popup-window');
}

const app = mount(label === 'popup' ? PopupApp : App, { target });

export default app;
