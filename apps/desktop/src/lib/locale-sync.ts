/**
 * 界面语言跨窗口同步。
 *
 * 主窗口（label = 'main'）与悬浮小窗（label = 'popup'）是两个独立 WebView，
 * 各自的 JS 上下文互不可见，但界面语言必须一致：小窗是常驻浮层，主窗口切了语言
 * 之后小窗还是旧语言会很别扭。
 *
 * 与 `theme-sync.ts` **完全同一套两条路**，任何一条通就够：
 *   1. **StorageEvent**（首选）：两个窗口同 origin，lib.rs 里小窗复用了主窗口的
 *      WebView2 数据目录，localStorage 应该是共享的。不需要额外权限，浏览器预览下也能用。
 *   2. **Tauri 事件总线**（兜底）：万一宿主把两边的存储分区隔开了，就靠
 *      `voctier:locale` 广播。注意 `emit` 是发给所有窗口的，本窗口也会收到自己的
 *      回声，所以回调里要先比对当前值。
 *
 * 触发点是 `i18n.svelte.ts::setLocale()` 写 localStorage —— 那是所有语言改动的唯一
 * 入口（设置页下拉），这里统一转发，不需要每个调用点都记得「顺便广播一下」。
 *
 * 单独放一个文件而不是塞进 `i18n.svelte.ts`：后者被大量组件 import，
 * 让它依赖 `api/bridge`（会拉起 @tauri-apps/api）会拖慢首屏并制造 import 环。
 */
import { emitLocale, onLocale, LOCALE_EVENT } from './api/bridge';
import { locale, setLocale, LOCALE_STORAGE_KEY } from './i18n.svelte';
import { isLocale } from './messages';

let attached = false;

/**
 * 在入口处调用一次（见 `main.ts`）。不返回清理函数：两个窗口都是常驻的，
 * 生命周期与页面一致；重复调用会被 `attached` 标记挡住。
 */
export function setupLocaleSync(): void {
  if (attached) return;
  attached = true;

  // 1. StorageEvent：别的窗口改了 localStorage 里的语言 key
  if (typeof window !== 'undefined') {
    window.addEventListener('storage', (event: StorageEvent) => {
      if (event.key !== LOCALE_STORAGE_KEY) return;
      const next = event.newValue;
      if (!isLocale(next) || next === locale.value) return;
      // setLocale 自己会把语言落到 <html lang> 与标题上
      setLocale(next);
    });
  }

  // 2. Tauri 事件总线兜底（本窗口自己发的回声也会走到这里，被值比对挡掉）
  void onLocale((payload) => {
    const next = payload?.locale;
    if (!isLocale(next) || next === locale.value) return;
    setLocale(next);
  });
}

/**
 * 切换语言并广播。设置页直接调这个，别直接调 `setLocale()`：
 * 前者会走完「本地状态 + localStorage + 广播」三条路。
 */
export function changeLocale(next: string): void {
  if (!isLocale(next)) return;
  setLocale(next);
  void emitLocale(next);
}

/** 事件名转导出，方便测试与对照 */
export { LOCALE_EVENT };
