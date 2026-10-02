import type { Component } from 'svelte';
import IconChart from '$lib/components/icons/IconChart.svelte';
import IconSplit from '$lib/components/icons/IconSplit.svelte';
import IconTrophy from '$lib/components/icons/IconTrophy.svelte';
import IconTable from '$lib/components/icons/IconTable.svelte';
import IconBook from '$lib/components/icons/IconBook.svelte';
import IconSettings from '$lib/components/icons/IconSettings.svelte';
import type { MessageKey } from './messages';

/**
 * 全局路由表（骨架阶段用 $state 做「选中项」切换，不引入路由库）。
 *
 * 后续想换成真实路由时，只需要：
 *   1. pnpm add -D svelte-routing  (或 @sveltejs/kit / svelte-spa-router)
 *   2. 把这个文件的 id 换成 path
 *   3. App.svelte 里的 {#if} 分支换成 <Route>
 * 页面组件本身不需要改。
 *
 * ⚠️ 这里存的是**文案 key**，不是文案本身：导航文字要跟着界面语言变，而本表是模块级
 * 常量（只在模块加载时求值一次），存字符串会让语言切换失效。展示处统一写
 * `t(item.labelKey)`，那是响应式读点，切语言会自动重渲染。
 */

export type RouteId = 'wordfreq' | 'sentences' | 'leaderboard' | 'dicts' | 'tables' | 'settings';

export type NavItem = {
  id: RouteId;
  /** 侧边栏文字（文案 key） */
  labelKey: MessageKey;
  /** 顶部标题栏标题（文案 key） */
  titleKey: MessageKey;
  /** 顶部标题栏副标题（文案 key） */
  descriptionKey: MessageKey;
  icon: Component;
};

export const NAV_ITEMS: NavItem[] = [
  {
    id: 'wordfreq',
    labelKey: 'nav.wordfreq.label',
    titleKey: 'nav.wordfreq.title',
    descriptionKey: 'nav.wordfreq.description',
    icon: IconChart,
  },
  {
    id: 'sentences',
    labelKey: 'nav.sentences.label',
    titleKey: 'nav.sentences.title',
    descriptionKey: 'nav.sentences.description',
    icon: IconSplit,
  },
  {
    id: 'leaderboard',
    labelKey: 'nav.leaderboard.label',
    titleKey: 'nav.leaderboard.title',
    descriptionKey: 'nav.leaderboard.description',
    icon: IconTrophy,
  },
  {
    id: 'dicts',
    labelKey: 'nav.dicts.label',
    titleKey: 'nav.dicts.title',
    descriptionKey: 'nav.dicts.description',
    icon: IconBook,
  },
  {
    id: 'tables',
    labelKey: 'nav.tables.label',
    titleKey: 'nav.tables.title',
    descriptionKey: 'nav.tables.description',
    icon: IconTable,
  },
  {
    id: 'settings',
    labelKey: 'nav.settings.label',
    titleKey: 'nav.settings.title',
    descriptionKey: 'nav.settings.description',
    icon: IconSettings,
  },
];

export const DEFAULT_ROUTE: RouteId = 'wordfreq';

/**
 * 跨页面通信用的 DOM 事件名（页面组件之间不互相 import，也不共享路由状态）。
 *
 *   跳转：      window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'wordfreq' }))
 *   加入分析：  window.dispatchEvent(new CustomEvent(ANALYZE_WORD_EVENT, { detail: '语料库' }))
 *
 * 监听方在 App.svelte 里（一次订阅，负责真实的路由切换）。
 */
export const NAVIGATE_EVENT = 'voctier:navigate';
export const ANALYZE_WORD_EVENT = 'voctier:analyze-word';

export function isRouteId(value: unknown): value is RouteId {
  return typeof value === 'string' && NAV_ITEMS.some((item) => item.id === value);
}

export function findNavItem(id: RouteId): NavItem {
  const item = NAV_ITEMS.find((entry) => entry.id === id);
  // 内部错误（路由 id 写错），不是用户可见文案，故不走 i18n
  if (!item) throw new Error(`未知路由：${id}`);
  return item;
}
