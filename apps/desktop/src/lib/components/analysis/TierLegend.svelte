<script lang="ts">
/**
 * TierLegend —— 七组色阶图例。
 *
 * 取色路径是**组号 → 稳定标识 → 调色板**，不按组名取色：组名是文案（将来会随界面
 * 语言变化），只有 `keys` 缺失时才回落到 `tier-colors.ts` 的前端常量。
 * 标签走 `names`（来自 `tierNamesOf(meta)`，i18n 的接入点），本组件不自己读 meta。
 *
 * 阈值有两个来源，优先级：`bounds`（分组自定义后的生效上界）> `tiers`（meta 默认值）。
 * 这样已经把 `effectiveBounds` 接进来的页面不会退化成后端默认分组。
 */
import { cn } from '$lib/utils';
import {
  paletteForTierKey,
  swatchStyle,
  tierKeyAt,
  tierRangeLabel,
  tierRangeLabelOfBounds,
} from '$lib/tier-colors';
import { isDark } from '$lib/use-dark.svelte';
import { t } from '$lib/i18n.svelte';
import type { Tier } from '$lib/types';

type Props = {
  /**
   * 分组标签（展示用，来自 `tierNamesOf(meta)`）。
   *
   * **必填**：本组件是纯展示组件，不自己读 meta 也不读 i18n —— 标签一律由调用方
   * （页面）从那个唯一接缝解析后传下来，这样切语言只需要那个接缝跟着变。
   */
  names: readonly string[];
  /** 七组稳定标识（`tierKeysFrom(meta)`）；缺省用前端常量 */
  keys?: readonly string[];
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
  names,
  keys,
  tiers = [],
  bounds = [],
  showUnknown = true,
  class: className = '',
}: Props = $props();

const dark = $derived(isDark());

/** 优先用生效上界；没有就用 meta 里的 tiers */
function rangeOf(index: number): string {
  if (bounds.length > 0) return tierRangeLabelOfBounds(index, bounds);
  if (tiers.length > index) return tierRangeLabel(index, tiers);
  return '';
}
</script>

<div class={cn('flex flex-wrap items-center gap-x-3 gap-y-1.5', className)} data-testid="tier-legend">
  {#each names as name, index (index)}
    {@const palette = paletteForTierKey(tierKeyAt(index, keys))}
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
      <span>{t('tier.unknown')}</span>
    </span>
  {/if}
</div>
