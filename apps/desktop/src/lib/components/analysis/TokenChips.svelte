<script lang="ts">
/**
 * TokenChips —— 划句分析 / 悬浮小窗共用的 token 渲染器。
 *
 * 渲染规则（与需求一致）：
 *   - `accepted = false`（标点、空白）：按原样渲染，走中性的次要色，不参与统计。
 *   - 查到分组的 token：用该组对应的浅色/深色色阶（背景色 + 前景色）。
 *   - 未收录（语料库没有这个词）：灰色 + 虚线下划线。
 *   - 单字 token 额外加一条底部细线，提示「查的是字表」。
 *
 * 分组怎么来的（**身份是组号，不是组名**）：
 *   默认用后端返回的组号 `token.tier`；一旦调用方传入 `meta` + `settings`
 *   （也就是用户在「表管理」页自定义过分组阈值），就改用 `tierIndexFor` 按**生效阈值**
 *   重算组号。组号再经 `tierKeyAt` 换成稳定标识去取色，所以改界面语言或调整分组顺序
 *   都不会串色。逻辑只有一份，组件自己不复制边界。
 */
import { cn } from '$lib/utils';
import { colorsForTierKey, tierKeyAt, tierKeysFrom, tokenStyle } from '$lib/tier-colors';
import { t, tierLabels } from '$lib/i18n.svelte';
import { formatCount } from '$lib/number-locale';
import { formatTopPercent, tierIndexFor } from '$lib/format';
import { primaryTableKey } from '$lib/tiers.svelte';
import { isDark } from '$lib/use-dark.svelte';
import type { Meta, Settings, TierCurve, TokenInfo } from '$lib/types';

type Props = {
  tokens: TokenInfo[];
  /** 紧凑模式：小窗 / 摘要卡片用 */
  compact?: boolean;
  /** 点击某个 token（排行榜「加入分析」等场景可选） */
  onPick?: (token: TokenInfo) => void;
  /** 鼠标悬停到某个 token（由父组件决定要不要弹浮层） */
  onHover?: (token: TokenInfo | null, event: MouseEvent | null) => void;
  /** 分组自定义：传入 meta + settings 时按生效阈值重算组号 */
  meta?: Meta | null;
  settings?: Settings | null;
  /** 覆盖率曲线缓存（`tierMethod = 'coverage'` 时需要） */
  curves?: Record<string, TierCurve>;
  /**
   * 额外高亮某一个 token（下标）。
   *
   * 用于「固定位置详情面板」：鼠标移开后详情还留在面板里，靠这个高亮让用户
   * 知道面板里是哪个词。不传 = 只有原有的 hover 描边，行为与以前完全一致。
   */
  activeIndex?: number | null;
  class?: string;
};

let {
  tokens,
  compact = false,
  onPick,
  onHover,
  meta = null,
  settings = null,
  curves,
  activeIndex = null,
  class: className = '',
}: Props = $props();

const dark = $derived(isDark());

/** 空白类 token（换行/空格）需要保留换行语义 */
function isWhitespace(text: string): boolean {
  return text.length > 0 && text.trim().length === 0;
}

/**
 * 这个 token 最终属于哪一组 —— 返回**组号**（0..6），未收录 / 标点返回 null。
 *
 * `meta` + `settings` 都齐了就按生效阈值重算（自定义分组才真正生效）；
 * 否则回落到后端给的组号 `token.tier`（用户没动过分组设置时，两者完全一致）。
 */
function tierIndexOf(token: TokenInfo): number | null {
  if (token.accepted && meta && settings) {
    const kind = token.single_cjk ? 'char' : 'word';
    // 阈值取**主表组**那一张：铺平之后用户可以把任意表组设为主表，
    // 写死 `full/word` 会让颜色与详情面板里的前%对不上。
    const index = tierIndexFor(
      kind,
      token.rank,
      meta,
      settings,
      curves,
      primaryTableKey(meta, kind)
    );
    if (index !== null) return index;
  }
  return token.tier;
}

/** 稳定标识（配色与 `data-tier` 用它；未收录为 null） */
function tierKeyOf(token: TokenInfo): string | null {
  return tierKeyAt(tierIndexOf(token), tierKeysFrom(meta));
}

/** 展示用组名（tooltip 用）；组号算不出来时回落到后端给的组名。随界面语言变 */
function tierLabelOf(token: TokenInfo): string | null {
  const index = tierIndexOf(token);
  const labels = tierLabels(meta);
  if (index !== null && labels.length > index) return labels[index];
  return token.tier_name;
}

function styleFor(token: TokenInfo): string {
  if (!token.accepted) {
    // 标点：不参与统计，用中性色，不写背景
    return 'color:var(--muted-foreground)';
  }
  return tokenStyle(colorsForTierKey(tierKeyOf(token), dark));
}

function titleOf(token: TokenInfo): string {
  if (token.accepted && token.count !== null) {
    // 前% 优先用后端按**主表**算好的；没有就退回 rank（老产物）
    const top =
      token.top_pct !== null && token.top_pct !== undefined
        ? formatTopPercent(token.top_pct)
        : `#${token.rank}`;
    return `${token.text} · ${tierLabelOf(token) ?? t('tier.unknown')} · ${t('tokenDetail.topValue', { pct: top })} · ${formatCount(token.count)}`;
  }
  return token.text;
}
</script>

<div
  class={cn(
    'flex flex-wrap items-baseline gap-x-0.5 gap-y-1 leading-loose',
    compact ? 'text-[13px] leading-relaxed' : 'text-[15px] leading-loose',
    className
  )}
>
  {#each tokens as token, index (index)}
    {#if isWhitespace(token.text)}
      <span class="whitespace-pre-wrap" aria-hidden="true">{token.text}</span>
    {:else if onPick && token.accepted}
      <!-- 可点击时用真实 <button>：天然支持键盘操作，也没有 a11y 告警 -->
      <button
        type="button"
        data-tier={tierKeyOf(token) ?? 'unknown'}
        title={titleOf(token)}
        class={cn(
          'cursor-pointer rounded-[3px] px-[2px] py-px text-left transition-shadow hover:ring-1 hover:ring-ring',
          compact ? 'text-[13px]' : 'text-[15px]',
          index === activeIndex && 'ring-2 ring-primary ring-offset-1 ring-offset-background',
          token.single_cjk && 'shadow-[inset_0_-1px_0_0_currentColor]'
        )}
        style={styleFor(token)}
        onmouseenter={(event) => onHover?.(token, event)}
        onmouseleave={() => onHover?.(null, null)}
        onclick={() => onPick?.(token)}
      >
        {token.text}
      </button>
    {:else}
      <span
        role="note"
        data-tier={token.accepted ? (tierKeyOf(token) ?? 'unknown') : 'punct'}
        title={titleOf(token)}
        class={cn(
          'rounded-[3px] px-[2px] py-px transition-shadow',
          compact ? 'text-[13px]' : 'text-[15px]',
          token.accepted && 'cursor-help',
          onHover && 'hover:ring-1 hover:ring-ring/50',
          index === activeIndex && 'ring-2 ring-primary ring-offset-1 ring-offset-background',
          token.single_cjk && 'shadow-[inset_0_-1px_0_0_currentColor]'
        )}
        style={styleFor(token)}
        onmouseenter={(event) => onHover?.(token, event)}
        onmouseleave={() => onHover?.(null, null)}
      >
        {token.text}
      </span>
    {/if}
  {/each}
</div>
