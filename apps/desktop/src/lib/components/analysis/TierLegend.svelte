<script lang="ts">
/**
 * TierLegend —— 七组色阶图例。
 *
 * 组名与阈值都从调用方传入（来自 meta.tier_names / 生效阈值），不硬编码分组边界。
 * 颜色从 tier-colors.ts 的调色板按组名取。
 *
 * 阈值有两个来源，优先级：`bounds`（分组自定义后的生效上界）> `tiers`（meta 默认值）。
 * 这样已经把 `effectiveBounds` 接进来的页面不会退化成后端默认分组。
 */
import { cn } from '$lib/utils';
import {
  DEFAULT_TIER_NAMES,
  TIER_PALETTE,
  swatchStyle,
  tierRangeLabel,
  tierRangeLabelOfBounds,
} from '$lib/tier-colors';
import { isDark } from '$lib/use-dark.svelte';
import type { Tier } from '$lib/types';

type Props = {
  /** 分组顺序（meta.tier_names）；缺省用标准七组 */
  names?: string[];
  /** 阈值（meta.tables[*].tiers）；提供时在色块后附加「≤ N」 */
  tiers?: Tier[];
  /**
   * 生效阈值：6 个排名上界（`effectiveBounds` 的结果）。
   * 提供时优先用它在色块后附加「≤ N」，`tiers` 只作为兜底。
   */
  bounds?: number[];
  /** 是否显示「未收录」图例项 */
  showUnknown?: boolean;
  class?: string;
};

let {
  names = DEFAULT_TIER_NAMES,
  tiers = [],
  bounds = [],
  showUnknown = true,
  class: className = '',
}: Props = $props();

const dark = $derived(isDark());

function paletteOf(name: string) {
  return TIER_PALETTE.get(name) ?? { lightFg: '#98A2B3', lightBg: 'transparent', darkFg: '#667085', darkBg: 'transparent' };
}

/** 优先用生效上界；没有就用 meta 里的 tiers */
function rangeOf(index: number): string {
  if (bounds.length > 0) return tierRangeLabelOfBounds(index, bounds);
  if (tiers.length > index) return tierRangeLabel(index, tiers);
  return '';
}
</script>

<div class={cn('flex flex-wrap items-center gap-x-3 gap-y-1.5', className)} data-testid="tier-legend">
  {#each names as name, index (name)}
    {@const palette = paletteOf(name)}
    {@const rangeLabel = rangeOf(index)}
    <span class="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
      <span
        class="inline-block size-3.5 shrink-0 rounded-[3px] border"
        style={swatchStyle(palette, dark)}
      ></span>
      <span>{name}</span>
      {#if rangeLabel}
        <span class="tabular-nums opacity-70">{rangeLabel}</span>
      {/if}
    </span>
  {/each}

  {#if showUnknown}
    <span class="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
      <span
        class="inline-block size-3.5 shrink-0 rounded-[3px] border border-dashed"
        style="color:{dark ? '#667085' : '#98A2B3'};border-color:{dark ? '#667085' : '#98A2B3'}"
      ></span>
      <span>未收录</span>
    </span>
  {/if}
</div>
