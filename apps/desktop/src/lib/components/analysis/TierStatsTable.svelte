<script lang="ts">
  /**
   * TierStatsTable —— 分组覆盖率表格。
   *
   * 表格的每一行都来自 `meta.tables[*].tier_stats`，阈值列来自同一张表的
   * `tiers`。**分组边界从不硬编码**，换语料库 / 换表都会自动跟着变。
   *
   * 取色路径是**行下标（组号）→ 稳定标识 → 调色板**，不按 `stat.name` 取色：
   * 组名是文案（将来会随界面语言变化），标签则优先用调用方传入的 `labels`
   * （来自 `tierNamesOf(meta)`，i18n 的接入点），没有时才回落到产物自带的名字。
   */
  import { cn } from '$lib/utils';
  import { formatInt, formatRatio, formatMaxRank } from '$lib/format';
  import { paletteForTierKey, swatchStyle, tierKeyAt, tierRangeLabel } from '$lib/tier-colors';
  import { isDark } from '$lib/use-dark.svelte';
  import { t } from '$lib/i18n.svelte';
  import type { Tier, TierStat } from '$lib/types';

  type Props = {
    stats: TierStat[];
    /** 可选：直接用这张表的 thresholds（没有时退回 stats 自带的 max_rank） */
    tiers?: Tier[];
    /** 分组标签（展示用，来自 `tierNamesOf(meta)`）；缺省用 `stat.name` */
    labels?: readonly string[];
    /** 七组稳定标识（`tierKeysFrom(meta)`）；缺省用前端常量 */
    keys?: readonly string[];
    /** 是否画累计覆盖率的条形图 */
    showBar?: boolean;
    class?: string;
  };

  let {
    stats,
    tiers = [],
    labels,
    keys,
    showBar = true,
    class: className = '',
  }: Props = $props();

  const dark = $derived(isDark());

  /** 累计覆盖率 → 进度条宽度（clamp 到 0..100） */
  function barWidth(cumulative: number): string {
    const pct = Math.min(100, Math.max(0, cumulative * 100));
    return `${pct}%`;
  }

  function rangeOf(index: number, stat: TierStat): string {
    if (tiers.length > index) return tierRangeLabel(index, tiers);
    return formatMaxRank(stat.max_rank);
  }

  function labelOf(index: number, stat: TierStat): string {
    return labels?.[index] ?? stat.name;
  }
</script>

<div class={cn('overflow-hidden rounded-lg border border-border', className)}>
  <table class="w-full border-collapse text-xs">
    <thead class="bg-surface-muted/60 text-muted-foreground">
      <tr>
        <th class="px-3 py-2 text-left font-medium">{t('tierStats.col.group')}</th>
        <th class="px-3 py-2 text-right font-medium">{t('tierStats.col.upper')}</th>
        <th class="px-3 py-2 text-right font-medium">{t('tierStats.col.entries')}</th>
        <th class="px-3 py-2 text-right font-medium">{t('tierStats.col.tokens')}</th>
        <th class="px-3 py-2 text-right font-medium">{t('tierStats.col.coverage')}</th>
        <th class="px-3 py-2 text-right font-medium">{t('tierStats.col.cumulative')}</th>
      </tr>
    </thead>
    <tbody>
      {#each stats as stat, index (index)}
        {@const palette = paletteForTierKey(tierKeyAt(index, keys))}
        <tr class="border-t border-border/70">
          <td class="px-3 py-2">
            <span class="inline-flex items-center gap-1.5">
              <span
                class="inline-block size-3 shrink-0 rounded-[3px] border"
                style={swatchStyle(palette, dark)}
              ></span>
              <span class="font-medium">{labelOf(index, stat)}</span>
            </span>
          </td>
          <td class="px-3 py-2 text-right tabular-nums text-muted-foreground">
            {rangeOf(index, stat)}
          </td>
          <td class="px-3 py-2 text-right tabular-nums">{formatInt(stat.entries)}</td>
          <td class="px-3 py-2 text-right tabular-nums">{formatInt(stat.tokens)}</td>
          <td class="px-3 py-2 text-right tabular-nums">{formatRatio(stat.coverage, { digits: 3 })}</td>
          <td class="px-3 py-2 text-right">
            <div class="flex items-center justify-end gap-2">
              {#if showBar}
                <span class="hidden h-1.5 w-16 overflow-hidden rounded-full bg-surface-muted sm:block">
                  <span
                    class="block h-full rounded-full bg-primary/70"
                    style="width: {barWidth(stat.cumulative)}"
                  ></span>
                </span>
              {/if}
              <span class="w-16 tabular-nums font-medium">
                {formatRatio(stat.cumulative, { digits: 2 })}
              </span>
            </div>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>
