<script lang="ts">
  /**
   * 排行榜 —— 全库 / 各分域的排名浏览。
   *
   *   - 域切换（全库 + 各分域）、词表 / 字表切换
   *   - 分页（每页 100，用 list_rank 的 from/limit，from 从 1 开始）
   *   - 前缀搜索（search_words）
   *   - 七组筛选（对当前已加载页做客户端过滤，界面上明确说明）
   *   - 点击某一行 → 「加入分析」把词送到划句分析页（DOM 自定义事件）
   *
   * 分组名一律通过 `meta.tables` 的阈值推算，绝不硬编码分组边界。
   */
  import { Badge } from '$lib/components/ui/badge';
  import { Button } from '$lib/components/ui/button';
  import {
    Card,
    CardContent,
    CardDescription,
    CardHeader,
    CardTitle,
  } from '$lib/components/ui/card';
  import Input from '$lib/components/ui/input/Input.svelte';
  import { Separator } from '$lib/components/ui/separator';
  import TierLegend from '$lib/components/analysis/TierLegend.svelte';
  import {
    activeDataset,
    activeTableDir,
    copyText,
    isTauri,
    libraryInfo,
    listRank,
    lookupWord,
    searchWords,
  } from '$lib/api/bridge';
  import {
    findTable,
    formatInt,
    formatPct,
    formatTopPercent,
    formatTimestamp,
    tableKeyOf,
  } from '$lib/format';
  import { ANALYZE_WORD_EVENT, NAVIGATE_EVENT } from '$lib/navigation';
  import {
    colorsForTierKey,
    fromUserFromFlags,
    inDictFromFlags,
    tierKeyAt,
    tierKeysFrom,
    tierRangeLabelOfBounds,
  } from '$lib/tier-colors';
  import {
    activeBounds,
    activeTierIndex,
    primaryScope,
    tierNameAt,
    tierNamesOf,
  } from '$lib/tiers.svelte';
  import { t } from '$lib/i18n.svelte';
  import { isDark } from '$lib/use-dark.svelte';
  import { cn } from '$lib/utils';
  import { metaScopes, type Meta, type RankRow, type WordHit } from '$lib/types';

  const PAGE_SIZE = 100;
  const SEARCH_LIMIT = 100;

  type Kind = 'word' | 'char';

  let kind = $state<Kind>('word');
  /** null = **主作用域**（用户指定的那张表），不是硬编码的 full */
  let domain = $state<string | null>(null);
  let page = $state(1);

  /**
   * 当前打开那张表的 `meta`（开机走 `active_dataset()`，不再自己拼产物目录）。
   *
   * `settings.dataDir` 现在是「数据文件夹」，拿它去 `dataset_status` 永远
   * `exists: false`，所以这个页面以前会一直显示"还没有可用的词频表"。
   */
  let activeMeta = $state<Meta | null>(null);
  let statusLoading = $state(true);
  let statusError = $state('');
  /** 后端当前打开的那张表的产物目录（列表接口传 null 即可，这里只用于展示） */
  let activeDir = $state<string | null>(null);

  let rows = $state<RankRow[]>([]);
  let loading = $state(false);
  let listError = $state('');

  let query = $state('');
  let searching = $state(false);
  let searchRows = $state<RankRow[]>([]);
  let searchError = $state('');

  /**
   * 分组筛选：存**组号**（0..6），不存组名。
   *
   * 组名是文案（将来会随界面语言变化），拿它当筛选状态的 key 会在切语言时整片失效。
   */
  let activeTiers = $state<number[]>([]);

  let detail = $state<WordHit | null>(null);
  let detailError = $state('');
  let detailOpen = $state(false);

  let notice = $state('');
  let noticeTone = $state<'info' | 'error'>('info');

  let requestSeq = 0;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  // ---------------------------------------------------------------- 派生

  const meta: Meta | null = $derived(activeMeta);
  const ready = $derived(meta !== null);
  const dark = $derived(isDark());

  /**
   * 当前作用域 + 表种对应的表元数据（用于条目数、total_tokens、分页与分组阈值）。
   *
   * `domain === null` = **主作用域**（用户指定的那张），不是硬编码的 `full`：
   * 铺平之后任意作用域都能当主表，排行榜必须跟着它走，否则同一页里"主表"标签
   * 与看到的排名是两张不同的表。
   */
  const currentTable = $derived.by(() => {
    if (!meta) return undefined;
    const scope = domain === null ? primaryScope(meta) : domain;
    return (
      meta.tables.find((table) => table.path === scope && table.kind === kind) ??
      findTable(meta.tables, kind)
    );
  });

  /** 当前表实际生效的 6 个排名上界（分组自定义在这里生效） */
  const bounds = $derived(
    meta ? activeBounds(kind, meta, currentTable ? tableKeyOf(currentTable) : undefined) : []
  );

  /** 分组标签（展示用；本地化在 `i18n.svelte.ts::tierLabels()`） */
  const tierNames = $derived(tierNamesOf(meta));

  /** 七组稳定标识：取色与身份都走它，不走组名 */
  const tierKeys = $derived(tierKeysFrom(meta));

  const totalEntries = $derived(currentTable?.entries ?? 0);
  const totalTokens = $derived(currentTable?.total_tokens ?? 0);
  const totalPages = $derived(Math.max(1, Math.ceil(totalEntries / PAGE_SIZE)));

  const isSearchMode = $derived(query.trim().length > 0);

  const visibleRows = $derived(isSearchMode ? searchRows : rows);

  /** 当前页里被七组筛选留下的行（按生效阈值判组；筛选状态存组号） */
  const filteredRows = $derived.by(() => {
    if (activeTiers.length === 0) return visibleRows;
    return visibleRows.filter((row) => {
      const index = tierIndexOfRank(row.rank);
      return index !== null && activeTiers.includes(index);
    });
  });

  const rangeLabel = $derived.by(() => {
    if (isSearchMode)
      return t('leaderboard.searchResultCount', { count: formatInt(visibleRows.length) });
    if (totalEntries === 0) return '—';
    const from = (page - 1) * PAGE_SIZE + 1;
    const to = Math.min(page * PAGE_SIZE, totalEntries);
    return t('leaderboard.rankRange', {
      from: formatInt(from),
      to: formatInt(to),
      total: formatInt(totalEntries),
    });
  });

  // ---------------------------------------------------------------- 生命周期

  $effect(() => {
    void bootstrap();
  });

  // 域 / 表种 / 页码变化时重新拉取
  $effect(() => {
    const nextDomain = domain;
    const nextKind = kind;
    const nextPage = page;
    const isReady = ready;
    const search = isSearchMode;
    if (!isReady || search) return;
    void loadPage(nextDomain, nextKind, nextPage);
  });

  async function bootstrap() {
    statusLoading = true;
    statusError = '';
    const res = await activeDataset();
    if (!res.ok) {
      statusLoading = false;
      statusError = res.error;
      activeMeta = null;
      return;
    }
    activeMeta = res.data;
    // 顺带记下产物目录（只用于页面展示）
    const infoRes = await libraryInfo();
    if (infoRes.ok) activeDir = activeTableDir(infoRes.data);
    statusLoading = false;
  }

  async function loadPage(nextDomain: string | null, nextKind: Kind, nextPage: number) {
    const seq = ++requestSeq;
    loading = true;
    listError = '';
    const from = (nextPage - 1) * PAGE_SIZE + 1;
    // dir 传 null = 用后端当前打开的那张表
    const res = await listRank(nextDomain, nextKind, from, PAGE_SIZE, null);
    if (seq !== requestSeq) return;
    loading = false;
    if (!res.ok) {
      listError = res.error;
      rows = [];
      return;
    }
    rows = res.data;
  }

  function onQueryInput(value: string) {
    query = value;
    if (searchTimer !== null) clearTimeout(searchTimer);
    if (!value.trim()) {
      searching = false;
      searchRows = [];
      searchError = '';
      return;
    }
    searching = true;
    searchTimer = setTimeout(() => {
      void runSearch(value.trim(), kind);
    }, 250);
  }

  async function runSearch(value: string, nextKind: Kind) {
    const seq = ++requestSeq;
    const res = await searchWords(value, nextKind, SEARCH_LIMIT, domain, null);
    if (seq !== requestSeq) return;
    searching = false;
    if (!res.ok) {
      searchError = res.error;
      searchRows = [];
      return;
    }
    searchError = '';
    searchRows = res.data;
  }

  function switchKind(next: Kind) {
    kind = next;
    page = 1;
    activeTiers = [];
    if (isSearchMode) void runSearch(query.trim(), next);
  }

  function switchDomain(next: string | null) {
    domain = next;
    page = 1;
  }

  function gotoPage(next: number) {
    page = Math.min(Math.max(1, next), totalPages);
  }

  /** 切换某一组（组号 0..6）的筛选状态 */
  function toggleTier(index: number) {
    activeTiers = activeTiers.includes(index)
      ? activeTiers.filter((item) => item !== index)
      : [...activeTiers, index];
  }

  function showNotice(message: string, tone: 'info' | 'error' = 'info') {
    notice = message;
    noticeTone = tone;
    window.setTimeout(() => {
      if (notice === message) notice = '';
    }, 3500);
  }

  /** 点击行 → 加入划句分析 */
  function addToAnalysis(word: string) {
    if (!word) return;
    window.dispatchEvent(new CustomEvent(ANALYZE_WORD_EVENT, { detail: word }));
    showNotice(t('leaderboard.addedToAnalysis', { word }));
  }

  async function showDetail(word: string) {
    detailOpen = true;
    detail = null;
    detailError = '';
    const res = await lookupWord(word, kind, null);
    if (!res.ok) {
      detailError = res.error;
      return;
    }
    detail = res.data;
  }

  async function copyRow(word: string) {
    const res = await copyText(word);
    showNotice(
      res.ok ? t('leaderboard.copied', { word }) : res.error,
      res.ok ? 'info' : 'error'
    );
  }

  function goWordFreq() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'wordfreq' }));
  }

  function goDicts() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'dicts' }));
  }

  function pctOf(row: RankRow): number | null {
    // 后端在 RankRow 上已经算好了占比与前%，总 token 拿不到时才用页面上那份兜底
    if (row.pct !== null && row.pct !== undefined) return row.pct;
    if (!totalTokens) return null;
    return (row.count * 100) / totalTokens;
  }

  /**
   * 「前 X%」= 排名 ÷ 表内词条总数（**默认口径**，见 TokenDetail.svelte 的 `topPercent`）。
   * 优先用后端给的 `top_pct`；没有就按当前表的条目数自己算。
   */
  function topPctOf(row: RankRow): number | null {
    if (row.top_pct !== null && row.top_pct !== undefined) return row.top_pct;
    return topPercentOf(row.rank);
  }

  /**
   * 「前 X%」= 排名 ÷ 表内词条总数，与「词条详情」组件里的口径一致
   * （见 TokenDetail.svelte 的 `topPercent`）。没有排名 / 没有词条数时为 null。
   */
  function topPercentOf(rank: number | null | undefined): number | null {
    if (!rank || rank < 1 || !totalEntries) return null;
    return (rank / totalEntries) * 100;
  }

  /**
   * 排名 → **组号**（按生效阈值；分组自定义后与后端返回的 `tier` 可能不同）。
   * 未收录返回 null。
   *
   * 返回组号而不是组名：组名是文案，组号才是身份（筛选、取色都靠它）。
   */
  function tierIndexOfRank(rank: number | null | undefined): number | null {
    if (!meta) return null;
    return activeTierIndex(kind, rank, meta, currentTable ? tableKeyOf(currentTable) : undefined);
  }

  /** 组号 → 展示用组名 */
  function tierLabelOf(index: number | null): string | null {
    return tierNameAt(index, tierNames);
  }

  /** 分组 pill 的内联样式（按组号取该组当前主题的色阶） */
  function tierPillStyle(index: number | null): string {
    const colors = colorsForTierKey(tierKeyAt(index, tierKeys), dark);
    const parts = [`color:${colors.fg}`];
    if (colors.bg !== 'transparent') parts.push(`background-color:${colors.bg}`);
    parts.push(`border-color:${colors.fg}55`);
    return parts.join(';');
  }

  function flagBadges(flags: number): { label: string; tone: string }[] {
    const badges: { label: string; tone: string }[] = [];
    if (!inDictFromFlags(flags)) {
      badges.push({
        label: t('leaderboard.flagOutOfDict'),
        tone: 'border-amber-500/40 text-amber-600 dark:text-amber-400',
      });
    }
    if (fromUserFromFlags(flags)) {
      badges.push({ label: t('leaderboard.flagUserDict'), tone: 'border-primary/40 text-primary' });
    }
    return badges;
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      {t('leaderboard.browserPreview')}
    </div>
  {/if}

  {#if notice}
    <div
      class={cn(
        'rounded-lg border px-3 py-2 text-xs',
        noticeTone === 'error'
          ? 'border-destructive/30 bg-destructive/5 text-destructive'
          : 'border-primary/30 bg-primary/5 text-primary'
      )}
    >
      {notice}
    </div>
  {/if}

  {#if statusLoading}
    <Card>
      <CardContent class="py-8 text-center text-xs text-muted-foreground">
        {t('leaderboard.checking')}
      </CardContent>
    </Card>
  {:else if statusError}
    <Card class="border-destructive/30">
      <CardContent class="py-6 text-xs text-destructive"
        >{t('leaderboard.statusFailed', { error: statusError })}</CardContent
      >
    </Card>
  {:else if !ready}
    <Card class="border-dashed">
      <CardHeader>
        <div class="flex items-center gap-2">
          <CardTitle>{t('leaderboard.noTable.title')}</CardTitle>
          <Badge variant="outline">active_dataset = null</Badge>
        </div>
        <CardDescription>{t('leaderboard.noTable.description')}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <p class="selectable rounded-md bg-surface-muted/60 px-3 py-2 font-mono text-xs">
          {activeDir || t('leaderboard.noDir')}
        </p>
        <div class="flex gap-2">
          <Button onclick={goWordFreq}>{t('leaderboard.goWordFreq')}</Button>
          <Button variant="outline" onclick={goDicts}>{t('leaderboard.goDicts')}</Button>
          <Button variant="outline" onclick={() => void bootstrap()}>{t('leaderboard.recheck')}</Button>
        </div>
      </CardContent>
    </Card>
  {:else}
    <!-- 控制区 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>{t('leaderboard.title')}</CardTitle>
          <Badge variant="secondary">{kind === 'word' ? t('table.word') : t('table.char')}</Badge>
          <Badge variant="outline">{domain ?? primaryScope(meta)}</Badge>
          {#if domain === null}
            <!-- 主作用域是全局设置，这里明确标出来，免得用户以为排行榜看的是 full -->
            <Badge variant="secondary">{t('leaderboard.primaryBadge')}</Badge>
          {/if}
        </div>
        <CardDescription>
          {t('leaderboard.summary', {
            generated: formatTimestamp(meta?.generated_at),
            entries: formatInt(totalEntries),
            tokens: formatInt(totalTokens),
          })}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <!-- 作用域切换：铺平之后每个作用域都是一张平等的表 -->
        <div class="flex flex-wrap items-center gap-1.5">
          <span class="mr-1 text-xs font-medium">{t('leaderboard.scope')}</span>
          <button
            type="button"
            class={cn(
              'rounded-md border px-2.5 py-1 text-xs transition-colors',
              domain === null
                ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                : 'border-border hover:bg-accent'
            )}
            onclick={() => switchDomain(null)}
          >
            {t('leaderboard.primaryScope', { scope: primaryScope(meta) })}
          </button>
          {#each metaScopes(meta) as name (name)}
            {@const info = meta?.tables.find((x) => x.path === name && x.kind === kind)}
            <button
              type="button"
              class={cn(
                'rounded-md border px-2.5 py-1 text-xs transition-colors',
                domain === name
                  ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                  : 'border-border hover:bg-accent'
              )}
              onclick={() => switchDomain(name)}
            >
              {name}
              {#if info}
                <span class="ml-1 text-[10px] text-muted-foreground">{formatInt(info.entries)}</span>
              {/if}
            </button>
          {/each}
        </div>

        <Separator />

        <div class="flex flex-wrap items-center gap-2">
          <!-- 词表 / 字表 -->
          <div class="flex items-center gap-1 rounded-md border border-border p-0.5">
            <button
              type="button"
              aria-pressed={kind === 'word'}
              class={cn(
                'rounded px-2.5 py-1 text-xs transition-colors',
                kind === 'word' ? 'bg-primary text-primary-foreground' : 'hover:bg-accent'
              )}
              onclick={() => switchKind('word')}
            >
              {t('table.word')}
            </button>
            <button
              type="button"
              aria-pressed={kind === 'char'}
              class={cn(
                'rounded px-2.5 py-1 text-xs transition-colors',
                kind === 'char' ? 'bg-primary text-primary-foreground' : 'hover:bg-accent'
              )}
              onclick={() => switchKind('char')}
            >
              {t('table.char')}
            </button>
          </div>

          <!-- 前缀搜索 -->
          <div class="flex min-w-56 flex-1 items-center gap-2">
            <Input
              value={query}
              oninput={(event) => onQueryInput(event.currentTarget.value)}
              placeholder={t('leaderboard.searchPlaceholder')}
              class="text-xs"
              aria-label={t('leaderboard.searchAria')}
            />
            {#if isSearchMode}
              <Button variant="ghost" size="sm" onclick={() => onQueryInput('')}
                >{t('common.clear')}</Button
              >
            {/if}
          </div>

          {#if searching}
            <span class="text-[11px] text-muted-foreground">{t('leaderboard.searching')}</span>
          {/if}
        </div>

        <!-- 七组筛选 -->
        <div class="flex flex-col gap-2 rounded-lg border border-border p-3">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-xs font-medium">{t('leaderboard.tierFilter')}</span>
            <span class="text-[11px] text-muted-foreground">
              {t('leaderboard.tierFilterHint', { count: formatInt(visibleRows.length) })}
            </span>
            {#if activeTiers.length > 0}
              <Button variant="ghost" size="sm" onclick={() => (activeTiers = [])}
                >{t('leaderboard.clearFilter')}</Button
              >
            {/if}
          </div>
          <div class="flex flex-wrap gap-1.5">
            {#each tierNames as name, index (index)}
              {@const active = activeTiers.includes(index)}
              <button
                type="button"
                aria-pressed={active}
                class={cn(
                  'rounded-md border px-2 py-1 text-[11px] transition-colors',
                  active ? 'ring-1 ring-ring' : 'opacity-80 hover:opacity-100'
                )}
                style={tierPillStyle(index)}
                onclick={() => toggleTier(index)}
              >
                {name}
                <span class="ml-1 opacity-70">{tierRangeLabelOfBounds(index, bounds)}</span>
              </button>
            {/each}
          </div>
          <TierLegend names={tierNames} keys={tierKeys} bounds={bounds} showUnknown={false} />
        </div>

        {#if listError}
          <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
            {listError}
          </p>
        {/if}
        {#if searchError}
          <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
            {searchError}
          </p>
        {/if}
      </CardContent>
    </Card>

    <!-- 表格 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>{t('leaderboard.rankListTitle')}</CardTitle>
          <Badge variant="outline">{rangeLabel}</Badge>
          {#if activeTiers.length > 0}
            <Badge variant="secondary"
              >{t('leaderboard.filteredCount', { count: formatInt(filteredRows.length) })}</Badge
            >
          {/if}
          {#if loading}<Badge variant="outline">{t('leaderboard.loading')}</Badge>{/if}
        </div>
        <CardDescription>{t('leaderboard.rowHint')}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div class="scrollbar-thin max-h-[32rem] overflow-auto rounded-lg border border-border">
          <table class="w-full border-collapse text-xs">
            <thead class="sticky top-0 z-10 bg-surface-muted text-muted-foreground">
              <tr>
                <th class="w-20 px-3 py-2 text-right font-medium">{t('leaderboard.col.rank')}</th>
                <th class="px-3 py-2 text-left font-medium">{t('leaderboard.col.word')}</th>
                <th class="px-3 py-2 text-right font-medium">{t('leaderboard.col.count')}</th>
                <th class="px-3 py-2 text-right font-medium" title={t('leaderboard.col.topTitle')}>
                  {t('leaderboard.col.top')}
                </th>
                <th
                  class="px-3 py-2 text-right font-medium"
                  title={t('leaderboard.col.shareTitle')}
                >
                  {t('leaderboard.col.share')}
                </th>
                <th class="px-3 py-2 text-left font-medium">{t('leaderboard.col.tier')}</th>
                <th class="px-3 py-2 text-left font-medium">{t('leaderboard.col.flags')}</th>
                <th class="px-3 py-2 text-right font-medium">{t('leaderboard.col.actions')}</th>
              </tr>
            </thead>
            <tbody>
              {#each filteredRows as row (row.rank + '-' + row.word)}
                {@const tierIdx = tierIndexOfRank(row.rank)}
                <tr
                  class="cursor-pointer border-t border-border/70 transition-colors hover:bg-accent/50"
                  onclick={() => addToAnalysis(row.word)}
                  title={t('leaderboard.rowTitle')}
                >
                  <td class="px-3 py-1.5 text-right tabular-nums text-muted-foreground">
                    {formatInt(row.rank)}
                  </td>
                  <td class="px-3 py-1.5 font-mono text-[13px]">{row.word}</td>
                  <td class="px-3 py-1.5 text-right tabular-nums">{formatInt(row.count)}</td>
                  <td class="px-3 py-1.5 text-right tabular-nums text-muted-foreground">
                    {formatTopPercent(topPctOf(row))}
                  </td>
                  <td class="px-3 py-1.5 text-right tabular-nums text-muted-foreground">
                    {formatPct(pctOf(row))}
                  </td>
                  <td class="px-3 py-1.5">
                    <span
                      class="rounded-full border px-2 py-0.5 text-[11px] font-medium"
                      style={tierPillStyle(tierIdx)}
                    >
                      {tierLabelOf(tierIdx) ?? t('tier.unknown')}
                    </span>
                  </td>
                  <td class="px-3 py-1.5">
                    {#if kind === 'char'}
                      <span class="text-[11px] text-muted-foreground">{t('leaderboard.charNoFlag')}</span>
                    {:else if flagBadges(row.flags).length === 0}
                      <span class="text-[11px] text-muted-foreground">{t('leaderboard.inDict')}</span>
                    {:else}
                      <span class="flex flex-wrap gap-1">
                        {#each flagBadges(row.flags) as badge (badge.label)}
                          <span class={cn('rounded border px-1.5 py-0.5 text-[11px]', badge.tone)}>
                            {badge.label}
                          </span>
                        {/each}
                      </span>
                    {/if}
                  </td>
                  <td class="px-3 py-1.5 text-right">
                    <span class="inline-flex gap-1">
                      <button
                        type="button"
                        class="rounded border border-border px-1.5 py-0.5 text-[11px] hover:bg-accent"
                        onclick={(event) => {
                          event.stopPropagation();
                          void showDetail(row.word);
                        }}
                      >
                        {t('leaderboard.detail')}
                      </button>
                      <button
                        type="button"
                        class="rounded border border-border px-1.5 py-0.5 text-[11px] hover:bg-accent"
                        onclick={(event) => {
                          event.stopPropagation();
                          void copyRow(row.word);
                        }}
                      >
                        {t('leaderboard.copy')}
                      </button>
                    </span>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>

          {#if filteredRows.length === 0}
            <p class="px-4 py-10 text-center text-xs text-muted-foreground">
              {loading
                ? t('leaderboard.loading')
                : isSearchMode
                  ? t('leaderboard.emptySearch', { query: query.trim() })
                  : activeTiers.length > 0
                    ? t('leaderboard.emptyFiltered')
                    : t('leaderboard.emptyPage')}
            </p>
          {/if}
        </div>

        {#if !isSearchMode}
          <div class="flex flex-wrap items-center gap-2">
            <Button variant="outline" size="sm" disabled={page <= 1} onclick={() => gotoPage(1)}>
              {t('leaderboard.firstPage')}
            </Button>
            <Button variant="outline" size="sm" disabled={page <= 1} onclick={() => gotoPage(page - 1)}>
              {t('leaderboard.prevPage')}
            </Button>
            <span class="text-xs text-muted-foreground">
              {t('leaderboard.pageLabel')}
              <input
                type="number"
                min="1"
                max={totalPages}
                value={page}
                class="mx-1 w-16 rounded border border-input bg-surface px-1.5 py-0.5 text-center text-xs tabular-nums"
                onchange={(event) => gotoPage(Number(event.currentTarget.value))}
                aria-label={t('leaderboard.pageAria')}
              />
              {t('leaderboard.totalPages', { count: formatInt(totalPages) })}
            </span>
            <Button
              variant="outline"
              size="sm"
              disabled={page >= totalPages}
              onclick={() => gotoPage(page + 1)}
            >
              {t('leaderboard.nextPage')}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={page >= totalPages}
              onclick={() => gotoPage(totalPages)}
            >
              {t('leaderboard.lastPage')}
            </Button>
            <span class="ml-auto text-[11px] text-muted-foreground"
              >{t('leaderboard.perPage', { count: PAGE_SIZE })}</span
            >
          </div>
        {/if}
      </CardContent>
    </Card>

    <!-- 查词详情 -->
    {#if detailOpen}
      <Card class="border-primary/30">
        <CardHeader>
          <div class="flex items-center gap-2">
            <CardTitle>{t('leaderboard.detailTitle')}</CardTitle>
            <Button variant="ghost" size="sm" class="ml-auto" onclick={() => (detailOpen = false)}>
              {t('leaderboard.close')}
            </Button>
          </div>
        </CardHeader>
        <CardContent class="flex flex-col gap-2">
          {#if detailError}
            <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {detailError}
            </p>
          {:else if detail}
            {@const detailTier = tierIndexOfRank(detail.rank)}
            <div class="flex flex-wrap items-center gap-3">
              <span class="text-lg font-semibold">{detail.word}</span>
              <span
                class="rounded-full border px-2 py-0.5 text-[11px] font-medium"
                style={tierPillStyle(detailTier)}
              >
                {tierLabelOf(detailTier) ?? t('tier.unknown')}
              </span>
              <Badge variant="outline">
                {t('leaderboard.groupN', { index: (detailTier ?? detail.tier) + 1 })}
              </Badge>
            </div>
            <dl class="grid grid-cols-2 gap-3 text-xs sm:grid-cols-5">
              <div>
                <dt class="text-muted-foreground">{t('leaderboard.col.rank')}</dt>
                <dd class="font-medium tabular-nums">#{formatInt(detail.rank)}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground" title={t('leaderboard.detail.topTitle')}
                  >{t('leaderboard.detail.top')}</dt
                >
                <dd class="font-medium tabular-nums">
                  {formatTopPercent(
                    detail.top_pct !== null && detail.top_pct !== undefined
                      ? detail.top_pct
                      : topPercentOf(detail.rank)
                  )}
                </dd>
              </div>
              <div>
                <dt class="text-muted-foreground">{t('leaderboard.col.count')}</dt>
                <dd class="font-medium tabular-nums">{formatInt(detail.count)}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground" title={t('leaderboard.detail.shareTitle')}
                  >{t('leaderboard.col.share')}</dt
                >
                <dd class="font-medium tabular-nums">{formatPct(detail.pct)}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">{t('leaderboard.detail.dict')}</dt>
                <dd class="font-medium"
                  >{detail.in_dict ? t('leaderboard.detail.inDict') : t('leaderboard.detail.outDict')}</dd
                >
              </div>
            </dl>
            <div class="flex gap-2">
              <Button size="sm" onclick={() => addToAnalysis(detail?.word ?? '')}
                >{t('leaderboard.addToAnalysis')}</Button
              >
              <Button variant="outline" size="sm" onclick={() => void copyRow(detail?.word ?? '')}>
                {t('leaderboard.copyWord')}
              </Button>
            </div>
          {:else}
            <p class="text-xs text-muted-foreground">{t('leaderboard.detailLoading')}</p>
          {/if}
        </CardContent>
      </Card>
    {/if}
  {/if}
</div>
