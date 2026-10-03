import { emit } from './events';
import { t } from './i18n.svelte';
import type { MessageKey } from './messages';

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

/** 校验任意值是不是合法档位（跨窗口的两条同步路都靠它把关） */
export function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'light' || value === 'dark' || value === 'system';
}

/** 首屏脚本用的同一个 key，SSR/桌面环境下都安全 */
export function readStoredTheme(): ThemeMode {
  if (typeof localStorage === 'undefined') return 'system';
  const raw = localStorage.getItem(THEME_STORAGE_KEY);
  return isThemeMode(raw) ? raw : 'system';
}

/** 本机是否已经存过明确选择（用来区分「首装 / 清过 storage」与「用户已经选过」） */
function hasStoredTheme(): boolean {
  if (typeof localStorage === 'undefined') return true;
  try {
    return isThemeMode(localStorage.getItem(THEME_STORAGE_KEY));
  } catch {
    /* 隐私模式 / 存储被禁：当作没有记录，随后用设置文件里的值兜底 */
    return false;
  }
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

/**
 * 采用设置文件里的主题（启动时的兜底）。
 *
 * 规则与界面语言的 `adoptLocaleFromSettings()` **完全一致**：只在本机**还没有明确
 * 选择**（首次安装 / 清过 storage）时才采用设置文件里的值。
 *
 * 原因：顶栏与小窗的切换是立即生效的（`setTheme` 写 localStorage + 广播），而设置
 * 文件里的 theme 要用户点「保存设置」才更新。启动时无条件覆盖，就会把用户刚切、
 * 还没保存的主题顶回旧值。
 */
export function adoptThemeFromSettings(raw: string | null | undefined): void {
  if (!isThemeMode(raw)) return;
  if (hasStoredTheme()) return;
  setTheme(raw);
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

/** 三档主题对应的文案 key（`Record<ThemeMode, …>`：新模式漏配会编译失败） */
const THEME_KEYS: Record<ThemeMode, MessageKey> = {
  light: 'theme.light',
  dark: 'theme.dark',
  system: 'theme.system',
};

/**
 * 供 UI 显示的主题标签。
 *
 * 刻意做成函数而不是 `THEME_LABELS` 常量表：常量表在模块加载时求值一次，
 * 切成英文后界面上还会是中文。这里是响应式读点，切语言会自动跟着变。
 */
export function themeLabel(mode: ThemeMode): string {
  return t(THEME_KEYS[mode]);
}
