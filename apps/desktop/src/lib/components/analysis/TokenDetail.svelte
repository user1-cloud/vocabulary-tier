<script lang="ts">
/**
 * TokenDetail —— 单个 token 的详情**纯展示组件**。
 *
 * 它是 `TokenTip`（旧的跟随鼠标浮层）里详情部分的抽取版：定位逻辑已经被删除，
 * 由调用方放在**固定位置**的容器里（划句分析页右侧面板 / 悬浮小窗底部区域），
 * 这样详情永远不会因为靠近窗口边缘被裁掉。
 *
 * 分组与阈值：不接收「算好的组名」，而是拿到 `meta` + `settings` + `curves` 后
 * 自己调用 `format.ts::boundsInfo` / `tierIndexFromBounds` 现算 —— 分组边界只有
 * 一个权威实现（`effectiveBounds`），用户在「表管理」页自定义阈值后这里立刻跟着变。
 * 千万不要用后端返回的 `token.tier_name` 去反推上界：那是 meta 默认阈值下的组名。
 *
 * 纯展示：不持有 state、不写 state，可以在任何位置复用。
 *
 * 布局约定（很重要，别改回去）：**无论选中词条、标点还是未收录，渲染的骨架完全一样**
 * —— 同样的两行四格 + 同样的一行徽标。以前标点时只剩一行文字、选中词时突然多出两行，
 * 容器高度跟着上下跳，小窗底部的固定区域看起来像在抖。现在所有字段常驻，
 * 没有数据的显示「—」，高度只由 `compact` 与字号决定。
 *
 * 「还没悬停过任何词」时不要在这里补提示语：小窗底部只有 112px 高，
 * 一段两行的提示会把四格数据顶出可视区（实测过）。小窗那边直接展示第一条词条。
 */
import { cn } from '$lib/utils';
import {
  DEFAULT_TIER_NAMES,
  paletteForTierName,
  swatchStyle,
  tierRangeLabelOfBounds,
} from '$lib/tier-colors';
import {
  boundsInfo,
  findTable,
  formatInt,
  formatPct,
  formatTopPercent,
  tierIndexFromBounds,
} from '$lib/format';
import { defaultSettings } from '$lib/api/bridge';
import { tableLabel } from '$lib/segments';
import { isDark } from '$lib/use-dark.svelte';
import type { Meta, Settings, TierCurve, TokenInfo } from '$lib/types';

type Props = {
  /** 当前展示的 token；null = 还没有悬停/钉住任何词，显示空态提示 */
  token: TokenInfo | null;
  /** 产物元数据：分组名与默认阈值都来自它 */
  meta?: Meta | null;
  /** 全局设置（分组自定义就在里面）；不传时按默认设置算 */
  settings?: Settings | null;
  /** 覆盖率曲线缓存（`tierMethod = 'coverage'` 时需要） */
  curves?: Record<string, TierCurve>;
  /** 紧凑模式：悬浮小窗底部那种窄区域用 */
  compact?: boolean;
  /** 是否处于「钉住」状态（仅影响文案） */
  pinned?: boolean;
  /** 空态提示文案 */
  emptyHint?: string;
  class?: string;
};

let {
  token,
  meta = null,
  settings = null,
  curves,
  compact = false,
  pinned = false,
  emptyHint = '悬停上方任意词条，这里会显示它的频次、排名与分组详情。',
  class: className = '',
}: Props = $props();

const dark = $derived(isDark());

/** 查字表还是查词表：以后端给的 single_cjk / table 为准 */
const isChar = $derived(token !== null && (token.single_cjk || token.table === 'char'));
const kind = $derived<'word' | 'char'>(isChar ? 'char' : 'word');
const tablePath = $derived(isChar ? 'full/char' : 'full/word');

const names = $derived(meta?.tier_names?.length ? meta.tier_names : DEFAULT_TIER_NAMES);

/**
 * 生效阈值 + 回退警告。`boundsInfo` 是「表管理页自定义分组」的唯一权威入口：
 * rank / coverage / even 三种方式都在里面解算，用户填了非法值也会回退到 meta 默认。
 */
const boundsState = $derived(
  meta ? boundsInfo(kind, meta, settings ?? defaultSettings(), curves, tablePath) : null
);
const bounds = $derived(boundsState?.bounds ?? []);
const boundsWarning = $derived(boundsState?.warning ?? '');

/** 全库表（`full/word` / `full/char`）——「前 %」与「占比」的分母都来自它 */
const fullTable = $derived(meta ? findTable(meta.tables, kind) : undefined);

/** 生效组号 0..6；未收录 / 没有 rank 时为 null */
const tierIndex = $derived(token === null ? null : tierIndexFromBounds(token.rank, bounds));

/** 显示用组名：生效阈值算出来的优先，否则退回后端给的组名 */
const tierName = $derived.by(() => {
  if (token === null) return null;
  if (tierIndex !== null) return names[tierIndex] ?? token.tier_name ?? null;
  return token.tier_name ?? null;
});

const palette = $derived(paletteForTierName(tierName));

/** 「≤ N」/「> N」——第 7 组没有上界，显示成「> 第 6 组上界」 */
const rangeLabel = $derived(
  tierIndex !== null && bounds.length > 0 ? tierRangeLabelOfBounds(tierIndex, bounds) : ''
);

const groupLabel = $derived(tierIndex !== null ? (names[tierIndex] ?? tierName ?? '') : '');

/** 语料库里根本没有这个词（与「极少」区分开：极少是有排名的真实分组） */
const notCollected = $derived(token !== null && token.rank === null && token.tier === null);

/** 分域排名里至少有一个排名（否则这块完全没信息量） */
const hasDomainRanks = $derived(token !== null && token.domain_ranks.length > 0);

/**
 * 「前 X%」—— 语料库里排名不低于它的词条占比。
 *
 *   前% = rank / 表内词条数 × 100
 *
 * 例：词表 20 万条、rank = 200 → 前 0.1%，含义是「比 99.9% 的词条更靠前」。
 * 词条总数取**全库表**（`full/word` / `full/char`）的 entries，与后端算 rank 的
 * 那张表一致；`pct`（占比）看的是 token 总量，两者不是一回事。
 *
 * ⚠️ 这里算的是「排名位置」，不是「按频次排序后有多少词条频次 ≤ 它」。
 *    Zipf 分布下同一个名次区间里的频次差异很大，两者在头部会差一截；
 *    要精确版本得让 Rust 侧在 VFR 里补一份累计词条数，属于后端改动。
 */
const entryCount = $derived(fullTable?.entries ?? 0);

const topPercent = $derived.by(() => {
  const rank = token?.rank ?? null;
  if (rank === null || entryCount <= 0) return null;
  return (rank / entryCount) * 100;
});

/** 占全部 token 的百分比：把算法写进 title，省得用户猜「占比」是什么意思 */
const pctTitle = $derived.by(() => {
  if (token === null || token.pct === null) return '该词条在全库表里没有记录，因此没有占比。';
  const total = fullTable?.total_tokens ?? 0;
  const which = isChar ? '全库字表' : '全库词表';
  if (total <= 0) return `占比 = 该词条出现次数 ÷ ${which}的总 token 数`;
  return `占比 = 该词条出现次数 ${formatInt(token.count)} ÷ ${which}总 token 数 ${formatInt(total)}`;
});
</script>

{#if token === null}
  <p class={cn('text-muted-foreground', compact ? 'text-[11px]' : 'text-xs')}>{emptyHint}</p>
{:else}
  <div class={cn('min-w-0 break-words', compact ? 'text-[11px]' : 'text-xs', className)}>
    <!-- 词头 + 分组。这一行常驻，所以选中标点还是长词，详情区高度都不变 -->
    <div class="flex items-start justify-between gap-2">
      <span
        class={cn('min-w-0 flex-1 truncate font-semibold', compact ? 'text-xs' : 'text-sm')}
        title={token.text}
        data-testid="token-detail-text"
      >
        {token.text}
      </span>
      <span
        class="shrink-0 rounded-full border px-2 py-0.5 text-[11px] font-medium whitespace-nowrap"
        style={swatchStyle(palette, dark)}
        data-testid="token-detail-tier"
      >
        {tierName ?? '未收录'}
      </span>
    </div>

    <!-- 四格数据：标点 / 未收录也照样渲染，只是值是「—」。
         布局因此恒定，容器高度不会随选中的 token 种类变化。 -->
    <dl class={cn('mt-2 grid grid-cols-2 gap-x-3', compact ? 'gap-y-1' : 'gap-y-1.5')}>
      <div class="flex items-baseline justify-between gap-2">
        <dt class="text-muted-foreground">频次</dt>
        <dd class="font-medium tabular-nums">{token.accepted ? formatInt(token.count) : '—'}</dd>
      </div>
      <div class="flex items-baseline justify-between gap-2">
        <dt class="text-muted-foreground">排名</dt>
        <dd class="font-medium tabular-nums">
          {!token.accepted ? '—' : token.rank === null ? '未收录' : `#${formatInt(token.rank)}`}
        </dd>
      </div>
      <div class="flex items-baseline justify-between gap-2">
        <dt class="text-muted-foreground">前</dt>
        <dd class="font-medium tabular-nums" title="排名 ÷ 该表词条总数">
          {token.accepted ? formatTopPercent(topPercent) : '—'}
        </dd>
      </div>
      <div class="flex items-baseline justify-between gap-2">
        <dt class="text-muted-foreground">占比</dt>
        <dd class="font-medium tabular-nums" title={pctTitle}>
          {token.accepted ? formatPct(token.pct) : '—'}
        </dd>
      </div>
    </dl>

    <div class="mt-2 flex flex-wrap items-center gap-1.5 border-t border-border pt-2">
      <span class="rounded border border-border px-1.5 py-0.5 text-[11px] whitespace-nowrap text-muted-foreground">
        {tableLabel(token.table)}
      </span>
      {#if token.accepted}
        <span
          class={cn(
            'rounded border px-1.5 py-0.5 text-[11px] whitespace-nowrap',
            token.in_dict
              ? 'border-border text-muted-foreground'
              : 'border-amber-500/40 text-amber-600 dark:text-amber-400'
          )}
        >
          {token.in_dict === null
            ? '词典信息未知'
            : token.in_dict
              ? 'jieba 词典内'
              : '词典外（HMM 新词）'}
        </span>
        {#if token.from_user}
          <span class="rounded border border-primary/40 px-1.5 py-0.5 text-[11px] whitespace-nowrap text-primary">
            来自用户词典
          </span>
        {/if}
        {#if pinned}
          <span class="rounded border border-primary/40 px-1.5 py-0.5 text-[11px] whitespace-nowrap text-primary">
            已钉住
          </span>
        {/if}
      {:else}
        <span class="text-muted-foreground" data-testid="token-detail-skipped">标点 / 空白，不参与统计</span>
      {/if}
    </div>

    {#if token.accepted}
      {#if notCollected}
        <p
          class="mt-2 rounded-md border border-dashed border-border px-2 py-1 text-muted-foreground"
          data-testid="token-detail-unknown"
        >
          语料库未收录：词表 / 字表里都没有这个词条，因此没有频次、排名与分组
          （与「极少」不同，那一组是有排名的真实分组）。
        </p>
      {/if}

      {#if rangeLabel}
        <p class="mt-1.5 text-[11px] text-muted-foreground" data-testid="token-detail-range">
          {groupLabel} · 生效排名上界 {rangeLabel}
        </p>
      {/if}
      {#if boundsWarning}
        <p class="mt-1 text-[11px] text-amber-600 dark:text-amber-400">
          生效阈值有回退：{boundsWarning}
        </p>
      {/if}

      {#if hasDomainRanks}
        <div class="mt-2 border-t border-border pt-2">
          <p class="mb-1 text-[11px] text-muted-foreground">各分域排名</p>
          <div class="flex flex-wrap gap-x-3 gap-y-1">
            {#each token.domain_ranks as [domainName, rank] (domainName)}
              <span class="text-[11px] whitespace-nowrap">
                <span class="text-muted-foreground">{domainName}</span>
                <span class="ml-1 tabular-nums font-medium">
                  {rank === null ? '未收录' : `#${formatInt(rank)}`}
                </span>
              </span>
            {/each}
          </div>
        </div>
      {/if}
    {/if}
  </div>
{/if}
