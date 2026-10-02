/**
 * 主题响应式辅助 —— 纯 TS + runes（.svelte.ts 才能用 $state）。
 *
 * theme.svelte.ts 里已经有全局主题状态，但组件里经常只需要一个
 * 「当前是不是深色」的布尔值（用于从 tier-colors.ts 里选浅色还是深色色阶）。
 * 这里把它包成一个模块级共享的状态，避免每个组件各挂一个 matchMedia 监听。
 *
 * ⚠️ 关键约束：`isDark()` 会被大量用在 `$derived` / 模板表达式里，而
 * **在 $derived 里写 state 会抛 state_unsafe_mutation**。因此 matchMedia
 * 监听的注册（会写 systemDark.value）必须放在模块初始化时做一次，
 * `isDark()` 本身只读不写。
 */

import { theme } from './theme.svelte';

/** 系统是否偏好深色（模块级单例，import 时初始化一次） */
const systemDark = $state({ value: false });

let watcherAttached = false;

function prefersDark(): boolean {
  if (typeof window === 'undefined' || !window.matchMedia) return false;
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

/**
 * 注册系统主题监听。只在模块加载时调用（组件初始化上下文，可以安全写 state）。
 * `initialized` 只是防止在 HMR 里重复注册。
 */
function attachWatcher(): void {
  if (watcherAttached) return;
  if (typeof window === 'undefined' || !window.matchMedia) return;
  watcherAttached = true;
  systemDark.value = prefersDark();
  const mql = window.matchMedia('(prefers-color-scheme: dark)');
  mql.addEventListener('change', (event) => {
    systemDark.value = event.matches;
  });
}

attachWatcher();

/**
 * 当前生效的深色状态。**只读**，可以安全地放进 `$derived` 与模板表达式，
 * 也会跟着 `theme.mode`（用户在顶栏 / 设置页切换）与系统偏好一起更新。
 */
export function isDark(): boolean {
  if (theme.mode === 'system') return systemDark.value;
  return theme.mode === 'dark';
}

/** 供需要显式刷新系统偏好的场景调用（一般不需要） */
export function refreshSystemDark(): void {
  systemDark.value = prefersDark();
}
