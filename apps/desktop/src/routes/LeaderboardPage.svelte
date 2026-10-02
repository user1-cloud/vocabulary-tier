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
    copyText,
    datasetStatus,
    getSettings,
    isTauri,
    listRank,
    lookupWord,
    openDataset,
    searchWords,
  } from '$lib/api/bridge';
  import {
    findTable,
    formatInt,
    formatPct,
    formatTopPercent,
    formatTimestamp,
  } from '$lib/format';
  import { ANALYZE_WORD_EVENT, NAVIGATE_EVENT } from '$lib/navigation';
  import {
    colorsForTierName,
    fromUserFromFlags,
    inDictFromFlags,
    tierRangeLabelOfBounds,
  } from '$lib/tier-colors';
  import { activeBounds, activeTierIndex, tierNameAt, tierNamesOf } from '$lib/tiers.svelte';
  import { isDark } from '$lib/use-dark.svelte';
  import { cn } from '$lib/utils';
  import type { DatasetStatus, Meta, RankRow, WordHit } from '$lib/types';

  const PAGE_SIZE = 100;
  const SEARCH_LIMIT = 100;

  type Kind = 'word' | 'char';

  let kind = $state<Kind>('word');
  /** null = 全库 */
  let domain = $state<string | null>(null);
  let page = $state(1);

  let status = $state<DatasetStatus | null>(null);
  let statusLoading = $state(true);
  let statusError = $state('');
  /** 后端是否已把该产物目录装入缓存（open_dataset 成功） */
  let datasetLoaded = $state(false);
  let datasetLoadError = $state('');

  let rows = $state<RankRow[]>([]);
  let loading = $state(false);
  let listError = $state('');

  let query = $state('');
  let searching = $state(false);
  let searchRows = $state<RankRow[]>([]);
  let searchError = $state('');

  let activeTiers = $state<string[]>([]);

  let detail = $state<WordHit | null>(null);
  let detailError = $state('');
  let detailOpen = $state(false);

  let notice = $state('');
  let noticeTone = $state<'info' | 'error'>('info');

  let requestSeq = 0;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  // ---------------------------------------------------------------- 派生

  const meta: Meta | null = $derived(status?.exists ? status.meta : null);
  const ready = $derived(meta !== null);
  const dark = $derived(isDark());

  /**
   * 当前域 + 表种对应的表元数据（用于总条目数、total_tokens、分页与分组阈值）
   */
  const currentTable = $derived.by(() => {
    if (!meta) return undefined;
    if (domain === null) return findTable(meta.tables, kind);
    const selected = domain;
    return (
      meta.tables.find((table) => table.path === `domains/${selected}/${kind}`) ??
      meta.tables.find(
        (table) =>
          table.kind === kind &&
          table.path.includes(selected) &&
          table.path.endsWith(`/${kind}`)
      )
    );
  });

  /** 当前表实际生效的 6 个排名上界（分组自定义在这里生效） */
  const bounds = $derived(meta ? activeBounds(kind, meta, currentTable?.path) : []);

  /** 分组名顺序 */
  const tierNames = $derived(tierNamesOf(meta));

  const totalEntries = $derived(currentTable?.entries ?? 0);
  const totalTokens = $derived(currentTable?.total_tokens ?? 0);
  const totalPages = $derived(Math.max(1, Math.ceil(totalEntries / PAGE_SIZE)));

  const isSearchMode = $derived(query.trim().length > 0);

  const visibleRows = $derived(isSearchMode ? searchRows : rows);

  /** 当前页里被七组筛选留下的行（按生效阈值判组） */
  const filteredRows = $derived.by(() => {
    if (activeTiers.length === 0) return visibleRows;
    return visibleRows.filter((row) => {
      const name = tierNameOf(row.rank);
      return name !== null && activeTiers.includes(name);
    });
  });

  const rangeLabel = $derived.by(() => {
    if (isSearchMode) return `搜索结果 ${formatInt(visibleRows.length)} 条`;
    if (totalEntries === 0) return '—';
    const from = (page - 1) * PAGE_SIZE + 1;
    const to = Math.min(page * PAGE_SIZE, totalEntries);
    return `排名 ${formatInt(from)}–${formatInt(to)} / 共 ${formatInt(totalEntries)}`;
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
    const settings = await getSettings();
    const dir = settings.ok ? (settings.data.dataDir ?? settings.data.corpusDir) : null;
    const res = await datasetStatus(dir);
    if (!res.ok) {
      statusLoading = false;
      statusError = res.error;
      return;
    }
    status = res.data;
    if (!res.data.meta) {
      statusLoading = false;
      return;
    }

    // 让后端把产物装进缓存（并按 meta.tokenizer 重建分词器，见 DESIGN §8.1）。
    // 后端未实现该命令时只降级提示，不阻塞其它功能。
    datasetLoaded = false;
    datasetLoadError = '';
    const opened = await openDataset(res.data.dir);
    if (opened.ok) {
      datasetLoaded = true;
      status = { ...res.data, meta: opened.data };
    } else {
      datasetLoadError = opened.error;
    }
    statusLoading = false;
  }

  async function loadPage(nextDomain: string | null, nextKind: Kind, nextPage: number) {
    const seq = ++requestSeq;
    loading = true;
    listError = '';
    const from = (nextPage - 1) * PAGE_SIZE + 1;
    const res = await listRank(nextDomain, nextKind, from, PAGE_SIZE, status?.dir ?? null);
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
    const res = await searchWords(value, nextKind, SEARCH_LIMIT, domain, status?.dir ?? null);
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

  function toggleTier(name: string) {
    activeTiers = activeTiers.includes(name)
      ? activeTiers.filter((item) => item !== name)
      : [...activeTiers, name];
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
    showNotice(`已加入划句分析：${word}`);
  }

  async function showDetail(word: string) {
    detailOpen = true;
    detail = null;
    detailError = '';
    const res = await lookupWord(word, kind, status?.dir ?? null);
    if (!res.ok) {
      detailError = res.error;
      return;
    }
    detail = res.data;
  }

  async function copyRow(word: string) {
    const res = await copyText(word);
    showNotice(res.ok ? `已复制：${word}` : res.error, res.ok ? 'info' : 'error');
  }

  function goWordFreq() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'wordfreq' }));
  }

  function pctOf(row: RankRow): number | null {
    if (!totalTokens) return null;
    return (row.count * 100) / totalTokens;
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
   * 排名 → 组名（按生效阈值；分组自定义后与后端返回的 `tier_name` 可能不同）。
   * 未收录返回 null。
   */
  function tierNameOf(rank: number | null | undefined): string | null {
    if (!meta) return null;
    return tierNameAt(activeTierIndex(kind, rank, meta, currentTable?.path), tierNames);
  }

  /** 分组 pill 的内联样式（取该组当前主题的色阶） */
  function tierPillStyle(name: string | null): string {
    const colors = colorsForTierName(name, dark);
    const parts = [`color:${colors.fg}`];
    if (colors.bg !== 'transparent') parts.push(`background-color:${colors.bg}`);
    parts.push(`border-color:${colors.fg}55`);
    return parts.join(';');
  }

  function flagBadges(flags: number): { label: string; tone: string }[] {
    const badges: { label: string; tone: string }[] = [];
    if (!inDictFromFlags(flags)) {
      badges.push({
        label: '词典外',
        tone: 'border-amber-500/40 text-amber-600 dark:text-amber-400',
      });
    }
    if (fromUserFromFlags(flags)) {
      badges.push({ label: '用户词典', tone: 'border-primary/40 text-primary' });
    }
    return badges;
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      浏览器预览模式：正在使用内置演示数据，翻页 / 搜索 / 分组筛选都可以直接体验。
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
        正在检查语料库产物…
      </CardContent>
    </Card>
  {:else if statusError}
    <Card class="border-destructive/30">
      <CardContent class="py-6 text-xs text-destructive">读取数据集状态失败：{statusError}</CardContent>
    </Card>
  {:else if !ready}
    <Card class="border-dashed">
      <CardHeader>
        <div class="flex items-center gap-2">
          <CardTitle>还没有可用的词频表</CardTitle>
          <Badge variant="outline">dataset_status.exists = false</Badge>
        </div>
        <CardDescription>排行榜需要先生成频率表。当前检查的目录：</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <p class="selectable rounded-md bg-surface-muted/60 px-3 py-2 font-mono text-xs">
          {status?.dir || '（未设置输出目录）'}
        </p>
        <div class="flex gap-2">
          <Button onclick={goWordFreq}>去生成词频表</Button>
          <Button variant="outline" onclick={() => void bootstrap()}>重新检查</Button>
        </div>
      </CardContent>
    </Card>
  {:else}
    {#if !datasetLoaded}
      <div class="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
        <p class="font-medium">产物目录尚未装载到后端</p>
        <p class="mt-0.5">
          已找到 meta.json（{status?.dir}），但 `open_dataset` 未成功
          {datasetLoadError ? `：${datasetLoadError}` : '。'}。
          在 Rust 侧实现该命令前，列表接口可能返回空结果。
        </p>
      </div>
    {/if}

    <!-- 控制区 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>排行榜</CardTitle>
          <Badge variant="secondary">{kind === 'word' ? '词表' : '字表'}</Badge>
          <Badge variant="outline">{domain ?? '全库'}</Badge>
        </div>
        <CardDescription>
          {formatTimestamp(meta?.generated_at)} 生成 · 当前表 {formatInt(totalEntries)} 条目 ·
          {formatInt(totalTokens)} token
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <!-- 域切换 -->
        <div class="flex flex-wrap items-center gap-1.5">
          <span class="mr-1 text-xs font-medium">域</span>
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
            全库
          </button>
          {#each meta?.domains ?? [] as item (item.name)}
            <button
              type="button"
              class={cn(
                'rounded-md border px-2.5 py-1 text-xs transition-colors',
                domain === item.name
                  ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                  : 'border-border hover:bg-accent'
              )}
              onclick={() => switchDomain(item.name)}
            >
              {item.name}
              <span class="ml-1 text-[10px] text-muted-foreground">{formatInt(item.files)}</span>
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
              词表
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
              字表
            </button>
          </div>

          <!-- 前缀搜索 -->
          <div class="flex min-w-56 flex-1 items-center gap-2">
            <Input
              value={query}
              oninput={(event) => onQueryInput(event.currentTarget.value)}
              placeholder="按词首前缀搜索，例如「语」"
              class="text-xs"
              aria-label="前缀搜索"
            />
            {#if isSearchMode}
              <Button variant="ghost" size="sm" onclick={() => onQueryInput('')}>清除</Button>
            {/if}
          </div>

          {#if searching}
            <span class="text-[11px] text-muted-foreground">搜索中…</span>
          {/if}
        </div>

        <!-- 七组筛选 -->
        <div class="flex flex-col gap-2 rounded-lg border border-border p-3">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-xs font-medium">分组筛选</span>
            <span class="text-[11px] text-muted-foreground">
              仅作用于当前已加载的 {formatInt(visibleRows.length)} 条（后端按排名分页，无法跨页筛选）
            </span>
            {#if activeTiers.length > 0}
              <Button variant="ghost" size="sm" onclick={() => (activeTiers = [])}>清除筛选</Button>
            {/if}
          </div>
          <div class="flex flex-wrap gap-1.5">
            {#each tierNames as name, index (name)}
              {@const active = activeTiers.includes(name)}
              <button
                type="button"
                aria-pressed={active}
                class={cn(
                  'rounded-md border px-2 py-1 text-[11px] transition-colors',
                  active ? 'ring-1 ring-ring' : 'opacity-80 hover:opacity-100'
                )}
                style={tierPillStyle(name)}
                onclick={() => toggleTier(name)}
              >
                {name}
                <span class="ml-1 opacity-70">{tierRangeLabelOfBounds(index, bounds)}</span>
              </button>
            {/each}
          </div>
          <TierLegend names={tierNames} bounds={bounds} showUnknown={false} />
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
          <CardTitle>排名列表</CardTitle>
          <Badge variant="outline">{rangeLabel}</Badge>
          {#if activeTiers.length > 0}
            <Badge variant="secondary">筛选后 {formatInt(filteredRows.length)} 条</Badge>
          {/if}
          {#if loading}<Badge variant="outline">加载中…</Badge>{/if}
        </div>
        <CardDescription>点击任意一行可「加入分析」，把该词送到划句分析页。</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div class="scrollbar-thin max-h-[32rem] overflow-auto rounded-lg border border-border">
          <table class="w-full border-collapse text-xs">
            <thead class="sticky top-0 z-10 bg-surface-muted text-muted-foreground">
              <tr>
                <th class="w-20 px-3 py-2 text-right font-medium">排名</th>
                <th class="px-3 py-2 text-left font-medium">词</th>
                <th class="px-3 py-2 text-right font-medium">频次</th>
                <th class="px-3 py-2 text-right font-medium" title="占比 = 该词频次 ÷ 全库表总 token 数（不是词条数的比例）">
                  占比
                </th>
                <th class="px-3 py-2 text-left font-medium">分组</th>
                <th class="px-3 py-2 text-left font-medium">标记</th>
                <th class="px-3 py-2 text-right font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {#each filteredRows as row (row.rank + '-' + row.word)}
                {@const tierName = tierNameOf(row.rank)}
                <tr
                  class="cursor-pointer border-t border-border/70 transition-colors hover:bg-accent/50"
                  onclick={() => addToAnalysis(row.word)}
                  title="点击加入划句分析"
                >
                  <td class="px-3 py-1.5 text-right tabular-nums text-muted-foreground">
                    {formatInt(row.rank)}
                  </td>
                  <td class="px-3 py-1.5 font-mono text-[13px]">{row.word}</td>
                  <td class="px-3 py-1.5 text-right tabular-nums">{formatInt(row.count)}</td>
                  <td class="px-3 py-1.5 text-right tabular-nums text-muted-foreground">
                    {formatPct(pctOf(row))}
                  </td>
                  <td class="px-3 py-1.5">
                    <span
                      class="rounded-full border px-2 py-0.5 text-[11px] font-medium"
                      style={tierPillStyle(tierName)}
                    >
                      {tierName ?? '未收录'}
                    </span>
                  </td>
                  <td class="px-3 py-1.5">
                    {#if kind === 'char'}
                      <span class="text-[11px] text-muted-foreground">字表无词典标记</span>
                    {:else if flagBadges(row.flags).length === 0}
                      <span class="text-[11px] text-muted-foreground">词典内</span>
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
                        详情
                      </button>
                      <button
                        type="button"
                        class="rounded border border-border px-1.5 py-0.5 text-[11px] hover:bg-accent"
                        onclick={(event) => {
                          event.stopPropagation();
                          void copyRow(row.word);
                        }}
                      >
                        复制
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
                ? '加载中…'
                : isSearchMode
                  ? `没有以「${query.trim()}」开头的词条。`
                  : activeTiers.length > 0
                    ? '当前页没有符合分组筛选的词条，试试翻页或清除筛选。'
                    : '这一页没有数据。'}
            </p>
          {/if}
        </div>

        {#if !isSearchMode}
          <div class="flex flex-wrap items-center gap-2">
            <Button variant="outline" size="sm" disabled={page <= 1} onclick={() => gotoPage(1)}>
              首页
            </Button>
            <Button variant="outline" size="sm" disabled={page <= 1} onclick={() => gotoPage(page - 1)}>
              上一页
            </Button>
            <span class="text-xs text-muted-foreground">
              第
              <input
                type="number"
                min="1"
                max={totalPages}
                value={page}
                class="mx-1 w-16 rounded border border-input bg-surface px-1.5 py-0.5 text-center text-xs tabular-nums"
                onchange={(event) => gotoPage(Number(event.currentTarget.value))}
                aria-label="页码"
              />
              / {formatInt(totalPages)} 页
            </span>
            <Button
              variant="outline"
              size="sm"
              disabled={page >= totalPages}
              onclick={() => gotoPage(page + 1)}
            >
              下一页
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={page >= totalPages}
              onclick={() => gotoPage(totalPages)}
            >
              末页
            </Button>
            <span class="ml-auto text-[11px] text-muted-foreground">每页 {PAGE_SIZE} 条</span>
          </div>
        {/if}
      </CardContent>
    </Card>

    <!-- 查词详情 -->
    {#if detailOpen}
      <Card class="border-primary/30">
        <CardHeader>
          <div class="flex items-center gap-2">
            <CardTitle>词条详情</CardTitle>
            <Button variant="ghost" size="sm" class="ml-auto" onclick={() => (detailOpen = false)}>
              关闭
            </Button>
          </div>
        </CardHeader>
        <CardContent class="flex flex-col gap-2">
          {#if detailError}
            <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {detailError}
            </p>
          {:else if detail}
            {@const detailTier = tierNameOf(detail.rank)}
            <div class="flex flex-wrap items-center gap-3">
              <span class="text-lg font-semibold">{detail.word}</span>
              <span
                class="rounded-full border px-2 py-0.5 text-[11px] font-medium"
                style={tierPillStyle(detailTier)}
              >
                {detailTier ?? '未收录'}
              </span>
              <Badge variant="outline">
                第 {(detailTier === null ? detail.tier : tierNames.indexOf(detailTier)) + 1} 组
              </Badge>
            </div>
            <dl class="grid grid-cols-2 gap-3 text-xs sm:grid-cols-5">
              <div>
                <dt class="text-muted-foreground">排名</dt>
                <dd class="font-medium tabular-nums">#{formatInt(detail.rank)}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground" title="排名 ÷ 该表词条总数">前</dt>
                <dd class="font-medium tabular-nums">
                  {formatTopPercent(topPercentOf(detail.rank))}
                </dd>
              </div>
              <div>
                <dt class="text-muted-foreground">频次</dt>
                <dd class="font-medium tabular-nums">{formatInt(detail.count)}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground" title="该词频次 ÷ 全库表总 token 数">占比</dt>
                <dd class="font-medium tabular-nums">{formatPct(detail.pct)}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">词典</dt>
                <dd class="font-medium">{detail.in_dict ? 'jieba 词典内' : '词典外'}</dd>
              </div>
            </dl>
            <div class="flex gap-2">
              <Button size="sm" onclick={() => addToAnalysis(detail?.word ?? '')}>加入分析</Button>
              <Button variant="outline" size="sm" onclick={() => void copyRow(detail?.word ?? '')}>
                复制词
              </Button>
            </div>
          {:else}
            <p class="text-xs text-muted-foreground">查询中…</p>
          {/if}
        </CardContent>
      </Card>
    {/if}
  {/if}
</div>
