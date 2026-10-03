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
 * 侧边栏是三层树形导航：分组（NavGroup）> 页面（NavPage）> 区块（NavSection）。
 *   分组 —— 侧栏顶层的可折叠分类
 *   页面 —— 一个路由（App.svelte 用它切页）
 *   区块 —— 页面内的可导航小节（点击自动滚动到对应 data-section 锚点）
 *
 * 区块的 labelKey 直接复用页面已有的 CardTitle 文案 key，不新增文案。
 * 锚点跳转：区块点击带 (pageId, sectionId)，App.svelte 切页后滚到
 * `[data-section="{sectionId}"]` 所在区块（见 App.svelte 的 pendingAnchor 逻辑）。
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

/** 区块小点：页面内可导航的一节（锚点 key + 复用页面 CardTitle 文案） */
export type NavSection = {
  /** 锚点 key，页面里对应 `data-section={id}`；同时用于区块高亮 */
  id: string;
  labelKey: MessageKey;
};

export type NavPage = {
  id: RouteId;
  /** 侧边栏页面名（文案 key） */
  labelKey: MessageKey;
  /** 顶部标题栏标题（文案 key） */
  titleKey: MessageKey;
  /** 顶部标题栏副标题（文案 key） */
  descriptionKey: MessageKey;
  icon: Component;
  /** 页面内的区块小点，顺序即显示顺序 */
  sections: NavSection[];
};

/** 侧边栏分组：一组 = 一个可展开/收起的目录 */
export type NavGroup = {
  /** 分组唯一标识（用于展开状态记忆） */
  id: string;
  /** 分组名（文案 key） */
  labelKey: MessageKey;
  /** 组内页面，顺序即显示顺序 */
  pages: NavPage[];
};

/**
 * 分组与顺序按「数据流水线」排，而不是按页面实现先后：
 *   分析 —— 高频日常入口，放最前
 *   数据 —— 先备词典 → 再统计产出词频表 → 最后管理/激活产物
 *   系统 —— 全局设置
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    id: 'analyze',
    labelKey: 'nav.group.analyze.label',
    pages: [
      {
        id: 'sentences',
        labelKey: 'nav.sentences.label',
        titleKey: 'nav.sentences.title',
        descriptionKey: 'nav.sentences.description',
        icon: IconSplit,
        sections: [
          { id: 'dataset', labelKey: 'sentences.dataset.title' },
          { id: 'input', labelKey: 'sentences.input.title' },
          { id: 'result', labelKey: 'sentences.result.title' },
        ],
      },
      {
        id: 'leaderboard',
        labelKey: 'nav.leaderboard.label',
        titleKey: 'nav.leaderboard.title',
        descriptionKey: 'nav.leaderboard.description',
        icon: IconTrophy,
        sections: [
          { id: 'control', labelKey: 'leaderboard.title' },
          { id: 'ranklist', labelKey: 'leaderboard.rankListTitle' },
        ],
      },
    ],
  },
  {
    id: 'data',
    labelKey: 'nav.group.data.label',
    pages: [
      {
        id: 'dicts',
        labelKey: 'nav.dicts.label',
        titleKey: 'nav.dicts.title',
        descriptionKey: 'nav.dicts.description',
        icon: IconBook,
        sections: [
          { id: 'datadir', labelKey: 'dicts.dataDirTitle' },
          { id: 'list', labelKey: 'dicts.listTitle' },
        ],
      },
      {
        id: 'wordfreq',
        labelKey: 'nav.wordfreq.label',
        titleKey: 'nav.wordfreq.title',
        descriptionKey: 'nav.wordfreq.description',
        icon: IconChart,
        sections: [
          { id: 'step1', labelKey: 'wordfreq.step1.title' },
          { id: 'step2', labelKey: 'wordfreq.step2.title' },
          { id: 'step3', labelKey: 'wordfreq.step3.title' },
        ],
      },
      {
        id: 'tables',
        labelKey: 'nav.tables.label',
        titleKey: 'nav.tables.title',
        descriptionKey: 'nav.tables.description',
        icon: IconTable,
        sections: [
          { id: 'library', labelKey: 'tables.library.title' },
          { id: 'list', labelKey: 'tables.listTitle' },
          { id: 'compose', labelKey: 'tables.compose.title' },
          { id: 'tier', labelKey: 'tables.tierConfigTitle' },
        ],
      },
    ],
  },
  {
    id: 'system',
    labelKey: 'nav.group.system.label',
    pages: [
      {
        id: 'settings',
        labelKey: 'nav.settings.label',
        titleKey: 'nav.settings.title',
        descriptionKey: 'nav.settings.description',
        icon: IconSettings,
        // 只放配置类区块；「保存」是操作、「关于」是静态信息，不进导航
        sections: [
          { id: 'hotkey', labelKey: 'settings.hotkey.title' },
          { id: 'popup', labelKey: 'settings.popup.title' },
          { id: 'appearance', labelKey: 'settings.appearance.title' },
          { id: 'language', labelKey: 'settings.language.title' },
          { id: 'tokenize', labelKey: 'settings.tokenize.title' },
          { id: 'paths', labelKey: 'settings.paths.title' },
        ],
      },
    ],
  },
];

/** 扁平视图：分组展平为页面列表，供 findNavItem / isRouteId 查找用（不再直接用于渲染） */
export const NAV_ITEMS: NavPage[] = NAV_GROUPS.flatMap((group) => group.pages);

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

export function findNavItem(id: RouteId): NavPage {
  const item = NAV_ITEMS.find((entry) => entry.id === id);
  // 内部错误（路由 id 写错），不是用户可见文案，故不走 i18n
  if (!item) throw new Error(`未知路由：${id}`);
  return item;
}
