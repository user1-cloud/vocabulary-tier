/**
 * 主题跨窗口同步。
 *
 * 主窗口（label = 'main'）与悬浮小窗（label = 'popup'）是两个独立的 WebView，
 * 各自的 JS 上下文互不可见，但主题必须保持一致：小窗是常驻浮层，主窗口切到深色
 * 之后小窗还亮着会非常刺眼。
 *
 * 走两条路，任何一条通就够：
 *   1. **StorageEvent**（首选）：两个窗口同 origin，lib.rs 里小窗复用了主窗口的
 *      WebView2 数据目录，localStorage 应该是共享的；一个窗口写 key，另一个窗口
 *      会收到 StorageEvent。不需要额外权限，浏览器预览下也能用。
 *   2. **Tauri 事件总线**（兜底）：万一宿主把两边的存储分区隔开了，就靠
 *      `voctier:theme` 广播。注意 `emit` 是发给所有窗口的，本窗口也会收到自己的
 *      回声，所以回调里要先比对当前 mode。
 *
 * 触发点是 `theme.svelte.ts` 里的本地 DOM 事件 `theme-changed` —— 那个模块是所有
 * 主题改动的唯一入口（顶栏、设置页、小窗按钮都走 setTheme），在这里统一转发，
 * 就不需要每个调用点都记得「顺便广播一下」。
 *
 * 这个模块单独放 `lib/` 而不是塞进 `theme.svelte.ts`：后者被大量组件 import，
 * 让它依赖 `api/bridge`（会拉起 @tauri-apps/api）会拖慢首屏并制造 import 环。
 */
import { on } from './events';
import { emitTheme, onTheme, THEME_EVENT } from './api/bridge';
import { applyTheme, theme, THEME_LABELS, type ThemeMode } from './theme.svelte';

/** 与 Rust 侧 / 另一个窗口对齐的 localStorage key（见 theme.svelte.ts） */
const STORAGE_KEY = 'voctier-theme';

function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'light' || value === 'dark' || value === 'system';
}

let attached = false;

/**
 * 在入口处调用一次（见 main.ts）。不返回清理函数：两个窗口都是常驻的，
 * 生命周期与页面一致；重复调用会被 `attached` 标记挡住。
 */
export function setupThemeSync(): void {
  if (attached) return;
  attached = true;

  // 0. 本窗口的主题变化 → 广播出去（覆盖顶栏、设置页、小窗按钮所有入口）
  on('theme-changed', ({ theme: mode }) => {
    void emitTheme(mode);
  });

  // 1. StorageEvent：别的窗口改了 localStorage 里的主题 key
  if (typeof window !== 'undefined') {
    window.addEventListener('storage', (event: StorageEvent) => {
      if (event.key !== STORAGE_KEY) return;
      const mode = event.newValue;
      if (!isThemeMode(mode) || mode === theme.mode) return;
      theme.mode = mode;
      applyTheme();
    });
  }

  // 2. Tauri 事件总线兜底（本窗口自己发的回声也会走到这里，被 mode 比对挡掉）
  void onTheme((payload) => {
    const mode = payload?.theme;
    if (!isThemeMode(mode) || mode === theme.mode) return;
    theme.mode = mode;
    applyTheme();
  });
}

/**
 * 下一档主题（浅色 → 深色 → 跟随系统）。
 *
 * 直接调 `setTheme()` 就行：写 localStorage 会触发 StorageEvent 那条路，
 * 本地 `theme-changed` 事件会触发 Tauri 广播那条路，两条路都由 setupThemeSync 接管。
 */
export function nextThemeMode(mode: ThemeMode): ThemeMode {
  const order: ThemeMode[] = ['light', 'dark', 'system'];
  return order[(order.indexOf(mode) + 1) % order.length];
}

/** 供按钮 title 用：`深色（点击切到浅色）` */
export function themeToggleHint(mode: ThemeMode): string {
  return `${THEME_LABELS[mode]}（点击切到${THEME_LABELS[nextThemeMode(mode)]}）`;
}

/** 事件名转导出，方便测试与对照 */
export { THEME_EVENT };
