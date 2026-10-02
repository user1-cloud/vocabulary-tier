<script lang="ts">
/**
 * TokenTip —— 悬停某个 token 时显示的浮层详情。
 *
 * 只负责「画一张卡片」，定位由父组件用一个 absolute 包装层负责：
 *
 *   <div class="relative">
 *     <TokenChips {tokens} onHover={(t) => (hovered = t)} />
 *     {#if hovered}
 *       <div class="pointer-events-none absolute left-0 top-full z-40 mt-2 w-72">
 *         <TokenTip token={hovered} bounds={bounds} names={names} />
 *       </div>
 *     {/if}
 *   </div>
 *
 * 为什么不给每个 token 单独挂浮层：1 万个 token 就是 1 万个绝对定位元素，
 * 一个共享浮层 + 一个 hovered 状态的开销是常数级。
 *
 * 分组与阈值：`name` 由父组件按生效阈值（`format.ts::tierIndexFor`）算好后传入；
 * 没传时退回 `token.tier_name`（用户没自定义分组时两者一致）。
 */
import { cn } from '$lib/utils';
import {
  DEFAULT_TIER_NAMES,
  paletteForTierName,
  swatchStyle,
  tierRangeLabel,
  tierRangeLabelOfBounds,
} from '$lib/tier-colors';
import { formatInt, formatPct } from '$lib/format';
import { tableLabel } from '$lib/segments';
import { isDark } from '$lib/use-dark.svelte';
import type { Tier, TokenInfo } from '$lib/types';

type Props = {
  token: TokenInfo;
  /** 生效的组名（由父组件用 tierIndexFor + meta.tier_names 算出来） */
  name?: string | null;
  /** 生效的组号 0..6（-1 / null = 未收录） */
  index?: number | null;
  /** 生效阈值：6 个排名上界（effectiveBounds 的结果） */
  bounds?: number[];
  /** 组名顺序（meta.tier_names），配合 bounds 用 */
  names?: string[];
  /** 兜底：meta.tables[*].tiers（没有传 bounds 时用） */
  tiers?: Tier[];
  class?: string;
};

let {
  token,
  name = null,
  index = null,
  bounds = [],
  names = DEFAULT_TIER_NAMES,
  tiers = [],
  class: className = '',
}: Props = $props();

const dark = $derived(isDark());

/** 显示用的组名：优先父组件算出来的，其次 token 自带的 */
const tierName = $derived(name ?? token.tier_name);

/**
 * 组号：优先父组件直接给的 `index`；没给就用组名在 `names`（生效分组名）里找；
 * 再退回 meta 的 tiers。父组件只传 `name` 时也能拿到正确的阈值文案。
 */
const tierIndex = $derived.by(() => {
  if (index !== null && index >= 0) return index;
  const named = names.indexOf(tierName ?? '');
  if (named >= 0) return named;
  return tiers.length > 0 ? tiers.findIndex((tier) => tier.name === tierName) : -1;
});

const palette = $derived(paletteForTierName(tierName));

const rangeLabel = $derived.by(() => {
  if (tierIndex < 0) return '';
  if (bounds.length > 0) return tierRangeLabelOfBounds(tierIndex, bounds);
  if (tiers.length > tierIndex) return tierRangeLabel(tierIndex, tiers);
  return '';
});

/** 这一组的组名（用于「≤ N」这类文案里说明是哪一档） */
const groupLabel = $derived(names[tierIndex] ?? tierName ?? '未收录');
</script>

<div
  class={cn(
    'rounded-lg border border-border bg-card p-3 text-xs shadow-xl',
    className
  )}
  role="tooltip"
>
  <!-- 词头 + 分组 -->
  <div class="flex items-start justify-between gap-3">
    <span class="selectable truncate text-sm font-semibold">{token.text}</span>
    <span
      class="shrink-0 rounded-full border px-2 py-0.5 text-[11px] font-medium"
      style={swatchStyle(palette, dark)}
    >
      {tierName ?? '未收录'}
    </span>
  </div>

  {#if !token.accepted}
    <p class="mt-2 text-muted-foreground">标点 / 空白，不参与统计。</p>
  {:else}
    <dl class="mt-2 grid grid-cols-2 gap-x-3 gap-y-1.5">
      <div class="flex items-baseline justify-between gap-2">
        <dt class="text-muted-foreground">频次</dt>
        <dd class="font-medium tabular-nums">{formatInt(token.count)}</dd>
      </div>
      <div class="flex items-baseline justify-between gap-2">
        <dt class="text-muted-foreground">排名</dt>
        <dd class="font-medium tabular-nums">{token.rank === null ? '未收录' : `#${formatInt(token.rank)}`}</dd>
      </div>
      <div class="flex items-baseline justify-between gap-2">
        <dt class="text-muted-foreground">占比</dt>
        <dd class="font-medium tabular-nums">{formatPct(token.pct)}</dd>
      </div>
      <div class="flex items-baseline justify-between gap-2">
        <dt class="text-muted-foreground">查表</dt>
        <dd class="font-medium">{tableLabel(token.table)}</dd>
      </div>
    </dl>

    {#if rangeLabel}
      <p class="mt-1.5 text-[11px] text-muted-foreground" data-testid="tip-range">
        {groupLabel} · 该组排名上界：{rangeLabel}
      </p>
    {/if}

    <div class="mt-2 flex flex-wrap gap-1.5 border-t border-border pt-2">
      <span
        class={cn(
          'rounded border px-1.5 py-0.5 text-[11px]',
          token.in_dict
            ? 'border-border text-muted-foreground'
            : 'border-amber-500/40 text-amber-600 dark:text-amber-400'
        )}
      >
        {token.in_dict === null ? '词典信息未知' : token.in_dict ? 'jieba 词典内' : '词典外（HMM 新词）'}
      </span>
      {#if token.from_user}
        <span class="rounded border border-primary/40 px-1.5 py-0.5 text-[11px] text-primary">
          来自用户词典
        </span>
      {/if}
    </div>

    {#if token.domain_ranks.length > 0}
      <div class="mt-2 border-t border-border pt-2">
        <p class="mb-1 text-[11px] text-muted-foreground">各分域排名</p>
        <div class="flex flex-wrap gap-x-3 gap-y-1">
          {#each token.domain_ranks as [domainName, rank] (domainName)}
            <span class="text-[11px]">
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
