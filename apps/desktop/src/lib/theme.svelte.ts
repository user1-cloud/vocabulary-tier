import { emit } from './events';

/**
 * 主题状态（Svelte 5 runes，模块级 .svelte.ts 是官方推荐的共享状态写法）。
 *
 * 三档：'light' | 'dark' | 'system'
 * - 实际生效值由 system 偏好 + 显式选择决定
 * - 真正落到 DOM 上的是 <html class="dark">
 * - localStorage key 必须与 index.html 里的首屏脚本保持一致
 */

export const THEME_STORAGE_KEY = 'voctier-theme';

export type ThemeMode = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';

/** 首屏脚本用的同一个 key，SSR/桌面环境下都安全 */
export function readStoredTheme(): ThemeMode {
  if (typeof localStorage === 'undefined') return 'system';
  const raw = localStorage.getItem(THEME_STORAGE_KEY);
  return raw === 'light' || raw === 'dark' || raw === 'system' ? raw : 'system';
}

function prefersDark(): boolean {
  if (typeof window === 'undefined' || !window.matchMedia) return false;
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

/** 当前用户选择（响应式） */
export const theme = $state<{ mode: ThemeMode }>({ mode: readStoredTheme() });

/** 系统是否偏好深色（响应式，由下面的 initTheme 保持更新） */
const systemDark = $state<{ value: boolean }>({ value: prefersDark() });

/** 实际生效的主题 */
export function resolvedTheme(): ResolvedTheme {
  if (theme.mode === 'system') return systemDark.value ? 'dark' : 'light';
  return theme.mode;
}

/** 切换到下一档（浅色 → 深色 → 跟随系统） */
export function cycleTheme(): void {
  setTheme(nextThemeMode(theme.mode));
}

/**
 * ⚠️ 这个模块只管**本窗口**的主题状态与 DOM class，不做跨窗口同步。
 *
 * 跨窗口同步（主窗口 ↔ 悬浮小窗）在 `$lib/theme-sync.ts`：那里同时用了
 * StorageEvent 与 Tauri 事件总线两条路，并且要用到 `api/bridge`。放在那边
 * 是为了不让这个被大量组件 import 的模块去拉起 @tauri-apps/api。
 */
export function setTheme(mode: ThemeMode): void {
  theme.mode = mode;
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(THEME_STORAGE_KEY, mode);
    } catch {
      /* 隐私模式忽略 */
    }
  }
  applyTheme();
  emit('theme-changed', { theme: mode });
}

/** 当前 mode 的下一个（浅色 → 深色 → 跟随系统 → 浅色）。小窗的一键切换用它 */
export function nextThemeMode(mode: ThemeMode): ThemeMode {
  const order: ThemeMode[] = ['light', 'dark', 'system'];
  return order[(order.indexOf(mode) + 1) % order.length];
}

/** 只做 DOM 副作用，不写 localStorage（供 $effect 调用） */
export function applyTheme(): void {
  if (typeof document === 'undefined') return;
  const resolved = resolvedTheme();
  document.documentElement.classList.toggle('dark', resolved === 'dark');
}

/**
 * 在根组件里调用一次：
 * - 监听系统主题变化
 * - 把当前主题打到 <html> 上
 *
 * 返回清理函数。
 */
export function initTheme(): () => void {
  const mql =
    typeof window !== 'undefined' && window.matchMedia
      ? window.matchMedia('(prefers-color-scheme: dark)')
      : null;

  const onChange = (event: MediaQueryListEvent) => {
    systemDark.value = event.matches;
    applyTheme();
  };

  if (mql) {
    systemDark.value = mql.matches;
    mql.addEventListener('change', onChange);
  }
  applyTheme();

  return () => {
    mql?.removeEventListener('change', onChange);
  };
}

/** 供 UI 显示的中文标签 */
export const THEME_LABELS: Record<ThemeMode, string> = {
  light: '浅色',
  dark: '深色',
  system: '跟随系统',
};
