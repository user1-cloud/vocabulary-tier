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
  paletteForTierKey,
  swatchStyle,
  tierKeyAt,
  tierKeysFrom,
  tierRangeLabelOfBounds,
} from '$lib/tier-colors';
import { t, tierLabels } from '$lib/i18n.svelte';
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
  emptyHint = t('tokenDetail.emptyHint'),
  class: className = '',
}: Props = $props();

const dark = $derived(isDark());

/** 查字表还是查词频表：以后端给的 single_cjk / table 为准 */
const isChar = $derived(token !== null && (token.single_cjk || token.table === 'char'));
const kind = $derived<'word' | 'char'>(isChar ? 'char' : 'word');

/**
 * **主表组**里这一类的那张表 —— 详情面板的一切数字都以它为准。
 *
 * 主表组由用户指定（`settings.primaryScope`），默认 `full`；产物里没有它时
 * 回落到该类的第一张表。**不能写死 `full/word`**：铺平之后 `full` 只是普通表组
 * 之一，用户完全可以把 `news` 或某张相加表设为主表 —— 那时详情里的前%、分组
 * 都必须按那张表算，否则和 token 上的颜色对不上。
 */
const primaryName = $derived(
  settings?.primaryScope
    ? settings.primaryScope
    : (meta?.tables.find((entry) => entry.kind === kind && entry.path === 'full')?.path ??
      meta?.tables.find((entry) => entry.kind === kind)?.path ??
      'full')
);
const primaryTable = $derived(
  meta ? (meta.tables.find((t) => t.path === primaryName && t.kind === kind) ?? findTable(meta.tables, kind)) : undefined
);
const tablePath = $derived(`${primaryTable?.path ?? primaryName}/${kind}`);

/** 展示用组名（按当前界面语言解析；见 `i18n.svelte.ts::tierLabels`） */
const names = $derived(tierLabels(meta));

/** 七组稳定标识：取色走它，不组名 */
const keys = $derived(tierKeysFrom(meta));

/**
 * 生效阈值 + 回退警告。`boundsInfo` 是「表管理页自定义分组」的唯一权威入口：
 * top_pct / rank / coverage / even 四种方式都在里面解算，用户填了非法值也会回退。
 */
const boundsState = $derived(
  meta ? boundsInfo(kind, meta, settings ?? defaultSettings(), curves, tablePath) : null
);
const bounds = $derived(boundsState?.bounds ?? []);
/** 回退警告（错误码）；渲染时才翻成文案 */
const boundsWarning = $derived(boundsState?.warning ?? null);

/** 全库表（`full/word` / `full/char`）——「前 %」与「占比」的分母都来自它 */
const fullTable = $derived(primaryTable);

/** 生效组号 0..6；未收录 / 没有 rank 时为 null */
const tierIndex = $derived(token === null ? null : tierIndexFromBounds(token.rank, bounds));

/** 显示用组名：生效阈值算出来的优先，否则退回后端给的组名 */
const tierName = $derived.by(() => {
  if (token === null) return null;
  if (tierIndex !== null) return names[tierIndex] ?? token.tier_name ?? null;
  return token.tier_name ?? null;
});

/**
 * 取色用的组号：生效组号优先，其次后端给的 `token.tier`。
 *
 * 后端给的是**组号**而不是组名，所以即使 meta 还没加载进来、`tierIndex` 算不出来，
 * 也照样能取到正确的颜色 —— 这就是「身份用组号」的实际好处。
 */
const paletteIndex = $derived(tierIndex ?? token?.tier ?? null);

const palette = $derived(paletteForTierKey(tierKeyAt(paletteIndex, keys)));

/** 「≤ N」/「> N」——第 7 组没有上界，显示成「> 第 6 组上界」 */
const rangeLabel = $derived(
  tierIndex !== null && bounds.length > 0 ? tierRangeLabelOfBounds(tierIndex, bounds) : ''
);

const groupLabel = $derived(tierIndex !== null ? (names[tierIndex] ?? tierName ?? '') : '');

/** 语料库里根本没有这个词（与「极少」区分开：极少是有排名的真实分组） */
const notCollected = $derived(token !== null && token.rank === null && token.tier === null);

/** 各表对比里至少有一项（否则这块完全没信息量） */
const hasTableRanks = $derived(token !== null && token.table_ranks.length > 0);

/**
 * 「前 X%」—— 排名 ÷ **主表条目数** × 100。
 *
 *   前% = rank / 表内词条数 × 100
 *
 * 例：主表 20 万条、rank = 200 → 前 0.1%，含义是「比 99.9% 的词条更靠前」。
 *
 * 优先用后端给的 `token.top_pct`（它就是按主表算的）；后端没给时才用主表的
 * `entries` 自己算一遍，保证分母与算 rank 的那张表一致。
 * `pct`（占比）看的是 token 总量，与它完全是两回事。
 */
const entryCount = $derived(fullTable?.entries ?? token?.entries ?? 0);

const topPercent = $derived.by(() => {
  const rank = token?.rank ?? null;
  if (rank === null) return null;
  if (token?.top_pct !== null && token?.top_pct !== undefined) return token.top_pct;
  if (entryCount <= 0) return null;
  return (rank / entryCount) * 100;
});

/** 占全部 token 的百分比：把算法写进 title，省得用户猜「占比」是什么意思 */
const pctTitle = $derived.by(() => {
  if (token === null || token.pct === null) return t('tokenDetail.pctNoRecord');
  const total = fullTable?.total_tokens ?? 0;
  const which = isChar ? t('tokenDetail.fullChar') : t('tokenDetail.fullWord');
  if (total <= 0) return t('tokenDetail.pctNoTotal', { which });
  return t('tokenDetail.pct', { count: formatInt(token.count), which, total: formatInt(total) });
});
</script>

{#if token === null}
  <p class={cn('text-muted-foreground', compact ? 'text-[11px]' : 'text-xs')}>{emptyHint}</p>
{:else}
  <div class={cn('min-w-0 break-words', compact ? 'text-[11px]' : 'text-xs', className)}>
    <!-- 骨架区：词头 + 分组徽标 + 四格数据 + 徽标行。
         ⚠️ 不要给它 sticky：用户要的是「详情块大小固定」，不是把其中一块钉住。
         高度一致靠「骨架恒定」——四种 token（多字词 / 单字 / 标点 / 未收录）
         都渲染同样的两行四格 + 同样的一行徽标，没数据的显示「—」。 -->
    <div class="pb-1">
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
          {tierName ?? t('tier.unknown')}
        </span>
      </div>

      <!-- 四格数据：标点 / 未收录也照样渲染，只是值是「—」。
           布局因此恒定，容器高度不会随选中的 token 种类变化。 -->
      <dl class={cn('mt-2 grid grid-cols-2 gap-x-3', compact ? 'gap-y-1' : 'gap-y-1.5')}>
        <div class="flex items-baseline justify-between gap-2">
          <dt class="text-muted-foreground">{t('tokenDetail.freq')}</dt>
          <dd class="font-medium tabular-nums">{token.accepted ? formatInt(token.count) : '—'}</dd>
        </div>
        <div class="flex items-baseline justify-between gap-2">
          <dt class="text-muted-foreground">{t('tokenDetail.rank')}</dt>
          <dd class="font-medium tabular-nums">
            {!token.accepted ? '—' : token.rank === null ? t('tier.unknown') : `#${formatInt(token.rank)}`}
          </dd>
        </div>
        <div class="flex items-baseline justify-between gap-2">
          <dt class="text-muted-foreground">{t('tokenDetail.top')}</dt>
          <dd class="font-medium tabular-nums" title={t('tokenDetail.topTitle')}>
            {token.accepted ? formatTopPercent(topPercent) : '—'}
          </dd>
        </div>
        <div class="flex items-baseline justify-between gap-2">
          <dt class="text-muted-foreground">{t('tokenDetail.share')}</dt>
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
              ? t('tokenDetail.dictUnknown')
              : token.in_dict
                ? t('tokenDetail.inDict')
                : t('tokenDetail.outDictHmm')}
          </span>
          {#if token.from_user}
            <span class="rounded border border-primary/40 px-1.5 py-0.5 text-[11px] whitespace-nowrap text-primary">
              {t('tokenDetail.fromUserDict')}
            </span>
          {/if}
          {#if pinned}
            <span class="rounded border border-primary/40 px-1.5 py-0.5 text-[11px] whitespace-nowrap text-primary">
              {t('tokenDetail.pinned')}
            </span>
          {/if}
        {:else}
          <span class="text-muted-foreground" data-testid="token-detail-skipped"
            >{t('tokenDetail.skipped')}</span
          >
        {/if}
      </div>
    </div>

    {#if token.accepted}
      {#if notCollected}
        <p
          class="mt-2 rounded-md border border-dashed border-border px-2 py-1 text-muted-foreground"
          data-testid="token-detail-unknown"
        >
          {t('tokenDetail.notCollected')}
        </p>
      {/if}

      {#if rangeLabel}
        <p class="mt-1.5 text-[11px] text-muted-foreground" data-testid="token-detail-range">
          {t('tokenDetail.rangeLine', { group: groupLabel, range: rangeLabel })}
        </p>
      {/if}
      {#if boundsWarning}
        <p class="mt-1 text-[11px] text-amber-600 dark:text-amber-400">
          {t('bounds.fallbackNotice', { message: t(boundsWarning.key, boundsWarning.params) })}
        </p>
      {/if}

      {#if hasTableRanks}
        <div class="mt-2 border-t border-border pt-2">
          <p class="mb-1 text-[11px] text-muted-foreground">{t('tokenDetail.tableRanksTitle')}</p>
          <div class="flex flex-wrap gap-x-3 gap-y-1">
            {#each token.table_ranks as item (item.scope)}
              <span class="text-[11px] whitespace-nowrap">
                <span class="text-muted-foreground">{item.scope}</span>
                <span class="ml-1 tabular-nums font-medium">
                  {item.top_pct === null
                    ? t('tier.unknown')
                    : t('tokenDetail.topValue', { pct: formatTopPercent(item.top_pct) })}
                </span>
                <span class="ml-1 text-muted-foreground tabular-nums">
                  {item.rank === null ? '' : `#${formatInt(item.rank)}`}
                </span>
              </span>
            {/each}
          </div>
        </div>
      {/if}
    {/if}
  </div>
{/if}
