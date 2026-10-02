import type { Component } from 'svelte';
import IconChart from '$lib/components/icons/IconChart.svelte';
import IconSplit from '$lib/components/icons/IconSplit.svelte';
import IconTrophy from '$lib/components/icons/IconTrophy.svelte';
import IconTable from '$lib/components/icons/IconTable.svelte';
import IconSettings from '$lib/components/icons/IconSettings.svelte';

/**
 * 全局路由表（骨架阶段用 $state 做「选中项」切换，不引入路由库）。
 *
 * 后续想换成真实路由时，只需要：
 *   1. pnpm add -D svelte-routing  (或 @sveltejs/kit / svelte-spa-router)
 *   2. 把这个文件的 id 换成 path
 *   3. App.svelte 里的 {#if} 分支换成 <Route>
 * 页面组件本身不需要改。
 */

export type RouteId = 'wordfreq' | 'sentences' | 'leaderboard' | 'tables' | 'settings';

export type NavItem = {
  id: RouteId;
  /** 侧边栏文字 */
  label: string;
  /** 顶部标题栏标题 */
  title: string;
  /** 顶部标题栏副标题 */
  description: string;
  icon: Component;
};

export const NAV_ITEMS: NavItem[] = [
  {
    id: 'wordfreq',
    label: '生成词频表',
    title: '生成词频表',
    description: '导入语料目录，统计字词出现频率并导出结果表。',
    icon: IconChart,
  },
  {
    id: 'sentences',
    label: '划句分析',
    title: '划句分析',
    description: '按句切分文本，逐句查看用词分布与难度层级。',
    icon: IconSplit,
  },
  {
    id: 'leaderboard',
    label: '排行榜',
    title: '排行榜',
    description: '按频次、覆盖率、词长等维度查看字词排名。',
    icon: IconTrophy,
  },
  {
    id: 'tables',
    label: '表管理',
    title: '表管理',
    description: '开关参与分域对比与排行榜的表，自定义七组分组阈值。',
    icon: IconTable,
  },
  {
    id: 'settings',
    label: '设置',
    title: '设置',
    description: '分词参数、输出格式、主题与界面偏好。',
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
  if (!item) throw new Error(`未知路由：${id}`);
  return item;
}
