<script lang="ts">
  /**
   * 划句分析 —— 核心页面。
   *
   * 交互链路：
   *   文本框粘贴 / 划选 → 选中「分析全文 / 只分析选中」→ analyze_text
   *   → 按 token 渲染着色（颜色来自 tier-colors.ts，分组阈值来自 meta.tables）
   *   → 悬停浮层显示词频 / 排名 / 占比 / 分域排名 / 词典标记。
   *
   * 分域过滤：`domains` 为空数组表示「查全部分域」（后端约定）。
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
  import { Separator } from '$lib/components/ui/separator';
  import TokenChips from '$lib/components/analysis/TokenChips.svelte';
  import TokenTip from '$lib/components/analysis/TokenTip.svelte';
  import TierLegend from '$lib/components/analysis/TierLegend.svelte';
  import {
    analyzeText,
    captureSelection,
    copyText,
    datasetStatus,
    getSettings,
    isTauri,
    openDataset,
    openPopup,
  } from '$lib/api/bridge';
  import { formatInt, formatPct, formatTimestamp, findTable } from '$lib/format';
  import { NAVIGATE_EVENT } from '$lib/navigation';
  import { summarizeTokens } from '$lib/segments';
  import { activeBounds, activeBoundsInfo, activeTierIndex, appSettings, tierNameAt, tierNamesOf } from '$lib/tiers.svelte';
  import { cn } from '$lib/utils';
  import type { DatasetStatus, Meta, TokenInfo } from '$lib/types';

  type Props = {
    /** 从排行榜「加入分析」或悬浮小窗回传带过来的初始文本 */
    initialText?: string;
  };

  let { initialText = '' }: Props = $props();

  /** 分析范围 */
  type Mode = 'all' | 'selection';
  let mode = $state<Mode>('all');

  let text = $state('');

  // 初始文本（由 App 在挂载时注入）只消费一次，之后文本框自己持有状态
  $effect.pre(() => {
    if (initialText && !text) text = initialText;
  });

  /** 文本框里当前划选的字符区间 */
  let selectionStart = $state(0);
  let selectionEnd = $state(0);

  let tokens = $state<TokenInfo[]>([]);
  let analyzing = $state(false);
  let analyzeError = $state('');

  let status = $state<DatasetStatus | null>(null);
  let statusLoading = $state(true);
  let statusError = $state('');
  /** 后端是否已把该产物目录装入缓存（open_dataset 成功） */
  let datasetLoaded = $state(false);
  let datasetLoadError = $state('');

  /** 选中的分域（空数组 = 全部分域） */
  let selectedDomains = $state<string[]>([]);

  /** 悬停浮层 */
  let hovered = $state<TokenInfo | null>(null);
  let tipPos = $state({ x: 0, y: 0 });
  let tipFlipped = $state(false);

  /** 点击钉住的 token 详情 */
  let pinned = $state<TokenInfo | null>(null);

  let notice = $state('');
  let noticeTone = $state<'info' | 'error'>('info');

  let wrapper = $state<HTMLDivElement | null>(null);
  let textarea = $state<HTMLTextAreaElement | null>(null);

  /** 递增序号：保证只有最后一次分析的响应被采用（防抖 + 乱序保护） */
  let requestSeq = 0;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  // ---------------------------------------------------------------- 派生

  const meta: Meta | null = $derived(status?.exists ? status.meta : null);
  const ready = $derived(meta !== null);

  const selectionText = $derived(text.slice(selectionStart, selectionEnd));

  /** 实际送去分析的文本 */
  const payload = $derived.by(() => {
    if (mode === 'selection') return selectionText;
    return text;
  });

  const summary = $derived(summarizeTokens(tokens, tierNameOfToken));

  const settings = $derived(appSettings.value);

  /** 生效阈值：默认 = meta 里的默认分组；用户在「表管理」页改过就是自定义的 */
  const wordBounds = $derived(meta ? activeBounds('word', meta) : []);
  const charBounds = $derived(meta ? activeBounds('char', meta) : []);
  const wordBoundsWarning = $derived(meta ? activeBoundsInfo('word', meta).warning : '');
  const charBoundsWarning = $derived(meta ? activeBoundsInfo('char', meta).warning : '');

  const wordTable = $derived(meta ? findTable(meta.tables, 'word') : undefined);
  const charTable = $derived(meta ? findTable(meta.tables, 'char') : undefined);

  const lineCount = $derived(text.length === 0 ? 0 : text.split('\n').length);

  const legendNames = $derived(tierNamesOf(meta));

  /** 悬停 / 钉住的 token 用哪一套阈值（单字查字表，其余查词表） */
  const tipIsChar = $derived(
    (pinned ?? hovered)?.single_cjk === true || (pinned ?? hovered)?.table === 'char'
  );
  const tipBounds = $derived(tipIsChar ? charBounds : wordBounds);

  /** 一个 token 实际落在哪一组（自定义阈值下与后端返回的 tier 可能不同） */
  function tierNameOfToken(token: TokenInfo): string | null {
    if (!meta) return token.tier_name;
    const isChar = token.single_cjk || token.table === 'char';
    const tablePath = isChar ? 'full/char' : 'full/word';
    const index = activeTierIndex(isChar ? 'char' : 'word', token.rank, meta, tablePath);
    return tierNameAt(index, legendNames) ?? token.tier_name;
  }

  // ---------------------------------------------------------------- 生命周期

  $effect(() => {
    void bootstrap();
  });

  // 文本 / 模式 / 分域 / 数据集变化时重新分析（带防抖）
  $effect(() => {
    // 显式读取依赖
    const nextPayload = payload;
    const nextDomains = selectedDomains;
    const isReady = ready;
    const nextDir = status?.dir ?? null;
    return scheduleAnalyze(nextPayload, nextDomains, isReady, nextDir);
  });

  async function bootstrap() {
    statusLoading = true;
    const settingsRes = await getSettings();
    const dir = settingsRes.ok ? (settingsRes.data.dataDir ?? settingsRes.data.corpusDir) : null;
    const res = await datasetStatus(dir);
    statusLoading = false;
    if (!res.ok) {
      statusError = res.error;
      return;
    }
    status = res.data;
    if (!res.data.meta) return;

    // 默认查全部分域（空数组），与后端约定一致
    selectedDomains = [];

    // 目录存在时让后端把产物装进缓存（并按 meta.tokenizer 重建分词器）。
    // 后端没实现这个命令时返回错误，这里只降级提示，不影响其它功能。
    datasetLoaded = false;
    datasetLoadError = '';
    const opened = await openDataset(res.data.dir);
    if (opened.ok) {
      datasetLoaded = true;
      // 用后端返回的 meta 覆盖一次，确保阈值与分词口径都以它为准
      status = { ...res.data, meta: opened.data };
    } else {
      datasetLoadError = opened.error;
    }
  }

  function scheduleAnalyze(
    nextPayload: string,
    nextDomains: string[],
    isReady: boolean,
    nextDir: string | null
  ) {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      void runAnalyze(nextPayload, nextDomains, isReady, nextDir);
    }, 220);
    return () => {
      if (debounceTimer !== null) {
        clearTimeout(debounceTimer);
        debounceTimer = null;
      }
    };
  }

  async function runAnalyze(
    nextPayload: string,
    nextDomains: string[],
    isReady: boolean,
    nextDir: string | null
  ) {
    if (!isReady) {
      tokens = [];
      return;
    }
    if (!nextPayload.trim()) {
      tokens = [];
      analyzeError = '';
      return;
    }
    const seq = ++requestSeq;
    analyzing = true;
    const res = await analyzeText(nextPayload, nextDomains, nextDir);
    if (seq !== requestSeq) return; // 已有更新的请求，丢弃这次结果
    analyzing = false;
    if (!res.ok) {
      analyzeError = res.error;
      tokens = [];
      return;
    }
    analyzeError = '';
    tokens = res.data;
  }

  // ---------------------------------------------------------------- 交互

  function syncSelection() {
    const element = textarea;
    if (!element) return;
    selectionStart = element.selectionStart ?? 0;
    selectionEnd = element.selectionEnd ?? 0;
  }

  function setMode(next: Mode) {
    mode = next;
    syncSelection();
  }

  function toggleDomain(name: string, checked: boolean) {
    selectedDomains = checked
      ? [...selectedDomains, name]
      : selectedDomains.filter((item) => item !== name);
  }

  function selectAllDomains() {
    selectedDomains = meta ? meta.domains.map((domain) => domain.name) : [];
  }

  function clearDomains() {
    selectedDomains = [];
  }

  /** 悬停时把浮层定位到光标附近（相对 token 容器，并在容器内夹取） */
  function onHover(token: TokenInfo | null, event: MouseEvent | null) {
    if (!token || !event || !wrapper) {
      hovered = null;
      return;
    }
    const rect = wrapper.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const tipWidth = 288; // w-72
    const tipHeight = 220;
    const maxX = Math.max(8, rect.width - tipWidth - 8);
    tipPos = {
      x: Math.min(Math.max(8, x), maxX),
      y: Math.max(8, y),
    };
    // 下方空间不够就翻到光标上方
    tipFlipped = y + tipHeight > rect.height && y > tipHeight;
    hovered = token;
  }

  function showNotice(message: string, tone: 'info' | 'error' = 'info') {
    notice = message;
    noticeTone = tone;
    window.setTimeout(() => {
      if (notice === message) notice = '';
    }, 4000);
  }

  async function sendToPopup() {
    if (!text.trim()) {
      showNotice('文本框是空的，没有可发送的内容。', 'error');
      return;
    }
    const res = await openPopup(text);
    if (!res.ok) {
      showNotice(res.error, 'error');
      return;
    }
    showNotice('已把全文发送到悬浮小窗。');
  }

  async function doCaptureSelection() {
    const res = await captureSelection();
    if (!res.ok) {
      showNotice(res.error, 'error');
      return;
    }
    if (!res.data.trim()) {
      showNotice('当前没有检测到全局选中的文本。');
      return;
    }
    text = res.data;
    mode = 'all';
    showNotice('已取到全局选中的文本并重新分析。');
  }

  async function copyAll() {
    if (!text.trim()) {
      showNotice('文本框是空的。', 'error');
      return;
    }
    const res = await copyText(text);
    showNotice(res.ok ? '全文已复制到剪贴板。' : res.error, res.ok ? 'info' : 'error');
  }

  function clearAll() {
    text = '';
    selectionStart = 0;
    selectionEnd = 0;
    tokens = [];
    pinned = null;
  }

  function goWordFreq() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'wordfreq' }));
  }

  function tableSummaryLabel(kind: 'word' | 'char'): string {
    const table = kind === 'word' ? wordTable : charTable;
    if (!table) return '—';
    return `${formatInt(table.entries)} 条`;
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      浏览器预览模式：正在使用内置演示数据，「发到悬浮小窗 / 手动取词」需要在桌面端运行。
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
    <!-- 表不存在：引导去生成词频表 -->
    <Card class="border-dashed">
      <CardHeader>
        <div class="flex items-center gap-2">
          <CardTitle>还没有可用的词频表</CardTitle>
          <Badge variant="outline">dataset_status.exists = false</Badge>
        </div>
        <CardDescription>
          划句分析依赖「生成词频表」产出的 meta.json 与 .vfr 索引表。当前检查的目录：
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <p class="selectable rounded-md bg-surface-muted/60 px-3 py-2 font-mono text-xs">
          {status?.dir || '（未设置输出目录）'}
        </p>
        <ul class="flex flex-col gap-1 text-xs text-muted-foreground">
          <li class="flex gap-2"><span class="mt-1.5 size-1.5 shrink-0 rounded-full bg-primary/60"></span>去「生成词频表」页选择语料库并开始统计</li>
          <li class="flex gap-2"><span class="mt-1.5 size-1.5 shrink-0 rounded-full bg-primary/60"></span>统计完成后回到本页，这里会自动加载新产物</li>
        </ul>
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
          在 Rust 侧实现该命令前，划句分析会返回「未收录」。
        </p>
      </div>
    {/if}

    <!-- 数据集概览 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>数据集</CardTitle>
          <Badge variant="success">已就绪</Badge>
          <Badge variant="secondary">schema v{meta?.schema_version}</Badge>
          {#if meta?.tokenizer}
            <Badge variant="outline">{meta.tokenizer.engine} {meta.tokenizer.version}</Badge>
            <Badge variant="outline">HMM {meta.tokenizer.hmm ? '开' : '关'}</Badge>
            {#if meta.tokenizer.user_dict}
              <Badge variant="outline">含用户词典</Badge>
            {/if}
          {/if}
        </div>
        <CardDescription>
          {formatTimestamp(meta?.generated_at)} 生成 · 全库 {formatInt(meta?.totals.tokens)} token ·
          词表 {tableSummaryLabel('word')} · 字表 {tableSummaryLabel('char')}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <p class="selectable truncate font-mono text-[11px] text-muted-foreground">{status?.dir}</p>
        <TierLegend names={legendNames} bounds={wordBounds} />
        {#if wordBoundsWarning || charBoundsWarning}
          <p class="rounded-md border border-amber-500/40 bg-amber-500/5 px-2 py-1 text-[11px] text-amber-700 dark:text-amber-300">
            {wordBoundsWarning || charBoundsWarning}
          </p>
        {/if}
      </CardContent>
    </Card>

    <!-- 输入区 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>文本输入</CardTitle>
          <div class="ml-auto flex items-center gap-1 rounded-md border border-border p-0.5">
            <button
              type="button"
              aria-pressed={mode === 'all'}
              class={cn(
                'rounded px-2.5 py-1 text-xs transition-colors',
                mode === 'all' ? 'bg-primary text-primary-foreground' : 'hover:bg-accent'
              )}
              onclick={() => setMode('all')}
            >
              分析全文
            </button>
            <button
              type="button"
              aria-pressed={mode === 'selection'}
              class={cn(
                'rounded px-2.5 py-1 text-xs transition-colors',
                mode === 'selection' ? 'bg-primary text-primary-foreground' : 'hover:bg-accent'
              )}
              onclick={() => setMode('selection')}
            >
              只分析选中
            </button>
          </div>
        </div>
        <CardDescription>
          粘贴文字，或在文本框里划选一段文字；下方的着色结果会实时更新。
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <textarea
          bind:this={textarea}
          bind:value={text}
          onselect={syncSelection}
          onmouseup={syncSelection}
          onkeyup={syncSelection}
          oninput={syncSelection}
          placeholder="在这里粘贴要分析的中文文本……"
          spellcheck="false"
          class={cn(
            'scrollbar-thin min-h-52 w-full resize-y rounded-lg border border-input bg-surface p-3',
            'text-sm leading-relaxed text-foreground placeholder:text-muted-foreground',
            'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background'
          )}
        ></textarea>

        <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
          <span>{formatInt(text.length)} 字 · {formatInt(lineCount)} 行</span>
          {#if mode === 'selection'}
            <span>
              已选 {formatInt(selectionText.length)} 字
              {#if selectionText.length === 0}
                <span class="text-amber-600 dark:text-amber-400">（请在文本框中划选一段文字）</span>
              {/if}
            </span>
          {:else}
            <span>分析范围：全文</span>
          {/if}
          {#if analyzing}<span>分析中…</span>{/if}
        </div>

        <!-- 分域过滤 -->
        <div class="flex flex-col gap-2 rounded-lg border border-border p-3">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-xs font-medium">分域过滤</span>
            <span class="text-[11px] text-muted-foreground">
              未选择任何分域 = 查全部分域（domains = []）
            </span>
            <span class="ml-auto flex gap-1">
              <Button variant="ghost" size="sm" onclick={selectAllDomains}>全选</Button>
              <Button variant="ghost" size="sm" onclick={clearDomains}>清空</Button>
            </span>
          </div>
          <div class="flex flex-wrap gap-x-4 gap-y-2">
            {#each meta?.domains ?? [] as domain (domain.name)}
              <label class="flex cursor-pointer items-center gap-1.5 text-xs">
                <input
                  type="checkbox"
                  class="size-3.5 accent-[var(--primary)]"
                  checked={selectedDomains.includes(domain.name)}
                  onchange={(event) => toggleDomain(domain.name, event.currentTarget.checked)}
                />
                <span>{domain.name}</span>
                <span class="text-[11px] text-muted-foreground">{formatInt(domain.files)} 文件</span>
              </label>
            {/each}
          </div>
        </div>

        <Separator />

        <div class="flex flex-wrap items-center gap-2">
          <Button size="sm" onclick={sendToPopup}>发到悬浮小窗</Button>
          <Button variant="outline" size="sm" onclick={() => void doCaptureSelection()}>手动取词</Button>
          <Button variant="outline" size="sm" onclick={() => void copyAll()}>复制全文</Button>
          <Button variant="ghost" size="sm" onclick={clearAll}>清空</Button>
        </div>
      </CardContent>
    </Card>

    <!-- 分析结果 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>分析结果</CardTitle>
          <Badge variant="outline">{formatInt(tokens.length)} token</Badge>
          <Badge variant="secondary">计入统计 {formatInt(summary.accepted)}</Badge>
          <Badge variant="outline">标点/空白 {formatInt(summary.skipped)}</Badge>
          {#if summary.unknownTotal > 0}
            <Badge variant="outline">
              未收录 {formatInt(summary.unknownUnique)} 种 / {formatInt(summary.unknownTotal)} 次
            </Badge>
          {/if}
        </div>
        <CardDescription>
          悬停任意词查看频次、排名、占比与分域排名；点击可钉住详情。
          <span class="ml-1">词表与字表的七组阈值都可以在「表管理」页自定义，这里按生效阈值着色。</span>
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        {#if analyzeError}
          <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
            {analyzeError}
          </p>
        {/if}

        {#if tokens.length === 0}
          <p class="rounded-lg border border-dashed border-border px-4 py-8 text-center text-xs text-muted-foreground">
            {mode === 'selection' && selectionText.length === 0
              ? '请在文本框中划选一段文字，或切换到「分析全文」。'
              : '暂无可分析的内容，先粘贴一段文字试试。'}
          </p>
        {:else}
          <!-- 分组命中分布 -->
          <div class="flex flex-wrap gap-2">
            {#each legendNames as name (name)}
              {@const hit = summary.byTier.get(name) ?? 0}
              <span
                class={cn(
                  'rounded-md border border-border px-2 py-1 text-[11px]',
                  hit === 0 && 'opacity-50'
                )}
              >
                <span class="text-muted-foreground">{name}</span>
                <span class="ml-1.5 tabular-nums font-medium">{formatInt(hit)}</span>
              </span>
            {/each}
          </div>

          <!-- 共享浮层的定位容器 -->
          <div bind:this={wrapper} class="relative">
            <TokenChips
              {tokens}
              onHover={onHover}
              onPick={(token) => (pinned = token)}
              {meta}
              {settings}
            />

            {#if hovered}
              <div
                class="pointer-events-none absolute z-40 w-72"
                style="left:{tipPos.x}px;top:{tipPos.y}px;transform:translateY({tipFlipped ? '-100%' : '0'}) translateY({tipFlipped ? '-10px' : '14px'})"
              >
                <TokenTip
                  token={hovered}
                  name={tierNameOfToken(hovered)}
                  bounds={tipBounds}
                  names={legendNames}
                />
              </div>
            {/if}
          </div>

          {#if pinned}
            <div class="flex flex-col gap-2 rounded-lg border border-primary/30 bg-primary/5 p-3">
              <div class="flex items-center justify-between">
                <span class="text-xs font-medium">已钉住的词条详情</span>
                <Button variant="ghost" size="sm" onclick={() => (pinned = null)}>关闭</Button>
              </div>
              <TokenTip
                token={pinned}
                name={tierNameOfToken(pinned)}
                bounds={tipBounds}
                names={legendNames}
                class="border-primary/20"
              />
            </div>
          {/if}

          <p class="text-[11px] text-muted-foreground">
            平均每 token 占比基准：词表 {formatPct(wordTable && wordTable.total_tokens > 0 ? (1 / wordTable.total_tokens) * 100 : null)} ·
            字表 {formatPct(charTable && charTable.total_tokens > 0 ? (1 / charTable.total_tokens) * 100 : null)}
          </p>
        {/if}
      </CardContent>
    </Card>
  {/if}
</div>
