<script lang="ts">
  /**
   * 划句分析 —— 核心页面。
   *
   * 交互链路：
   *   文本框粘贴 / 划选 → 选中「分析全文 / 只分析选中」→ analyze_text
   *   → 按 token 渲染着色（颜色来自 tier-colors.ts，分组阈值来自 meta.tables）
   *   → 悬停 token 时在**右侧固定面板**里显示词频 / 排名 / 占比 / 分域排名 / 词典标记。
   *
   * 详情为什么不做成跟随鼠标的浮层：浮层靠近窗口边缘会被裁掉，读不全；
   * 固定面板永远在窗口内，内容长了自己滚。布局样式见文件末尾。
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
  import TokenDetail from '$lib/components/analysis/TokenDetail.svelte';
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
  import { activeBounds, activeBoundsInfo, activeTierIndex, appSettings, tierCurves, tierNameAt, tierNamesOf } from '$lib/tiers.svelte';
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

  /**
   * 右侧「词条详情」面板显示的 token。
   *
   * 用**下标**记而不是 token 对象：重新分析会整批换掉 token 对象，用下标才能在
   * 结果刷新后继续指向同一个位置。鼠标移开时**不清空**（否则鼠标移向右侧面板
   * 去读详情时会闪没），所以这里不再需要 hovered / tipPos 那套浮层定位状态。
   */
  let activeIndex = $state<number | null>(null);
  /** 点击钉住的下标：钉住后悬停别的词不改变面板内容 */
  let pinnedIndex = $state<number | null>(null);

  let notice = $state('');
  let noticeTone = $state<'info' | 'error'>('info');

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
  /**
   * 生效阈值：默认 = meta 里的默认分组；用户在「表管理」页改过就是自定义的。
   *
   * 词表阈值给图例用；字表阈值不用在这里算 —— 详情面板（TokenDetail）自己按
   * token 是词还是字调用 `boundsInfo` 现算，保证与着色用同一套权威实现。
   */
  const wordBounds = $derived(meta ? activeBounds('word', meta) : []);
  const wordBoundsWarning = $derived(meta ? activeBoundsInfo('word', meta).warning : '');
  const charBoundsWarning = $derived(meta ? activeBoundsInfo('char', meta).warning : '');

  const wordTable = $derived(meta ? findTable(meta.tables, 'word') : undefined);
  const charTable = $derived(meta ? findTable(meta.tables, 'char') : undefined);

  const lineCount = $derived(text.length === 0 ? 0 : text.split('\n').length);

  const legendNames = $derived(tierNamesOf(meta));

  /** 详情面板里展示哪一条：钉住的优先，其次最后一次悬停的（鼠标移开也保留） */
  const shownIndex = $derived(pinnedIndex ?? activeIndex);
  const shownToken = $derived(shownIndex === null ? null : (tokens[shownIndex] ?? null));

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

  /**
   * token 在结果数组里的下标。
   *
   * 不直接用 `tokens.indexOf(token)`：`$state` 数组是深层代理，交给组件的
   * token 与数组里的元素在正常情况下是同一份代理，但下标比较走「位置 + 文本」
   * 更稳（代理身份比较在不同渲染路径下不保证相等）。
   */
  function indexOfToken(token: TokenInfo): number {
    return tokens.findIndex(
      (item) =>
        item === token || (item.byte_start === token.byte_start && item.text === token.text)
    );
  }

  /**
   * 悬停某个 token → 右侧固定面板跟着变。
   *
   * `token === null` 表示鼠标移开：**什么都不做**，保留最后一次的详情，
   * 这样鼠标移向右侧面板阅读时不会闪没。
   */
  function onHover(token: TokenInfo | null) {
    if (!token) return;
    const index = indexOfToken(token);
    if (index < 0) return;
    activeIndex = index;
  }

  /** 点击钉住 / 取消钉住 */
  function onPick(token: TokenInfo) {
    const index = indexOfToken(token);
    if (index < 0) return;
    pinnedIndex = pinnedIndex === index ? null : index;
    activeIndex = index;
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
    activeIndex = null;
    pinnedIndex = null;
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

    <!--
      两栏布局：左侧 = 输入框 + 着色 token 展示；右侧 = 固定宽度的「词条详情」面板。
      详情不再跟随鼠标（浮层靠近窗口边缘会被裁掉），而是停在固定位置、自己滚动；
      窄屏（< 1100px）自动堆叠成上下布局，面板落到下方，同样不会被裁。
    -->
    <div class="analysis-layout">
      <div class="analysis-main">
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
              悬停任意词，右侧「词条详情」面板显示它的频次、排名、占比与分域排名；点击词条可钉住详情。
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

              <!-- 着色 token：悬停 → 右侧固定面板；点击 → 钉住 -->
              <TokenChips
                {tokens}
                {meta}
                {settings}
                curves={tierCurves}
                activeIndex={shownIndex}
                onHover={onHover}
                onPick={onPick}
              />

              <p class="text-[11px] text-muted-foreground">
                平均每 token 占比基准：词表 {formatPct(wordTable && wordTable.total_tokens > 0 ? (1 / wordTable.total_tokens) * 100 : null)} ·
                字表 {formatPct(charTable && charTable.total_tokens > 0 ? (1 / charTable.total_tokens) * 100 : null)}
              </p>
            {/if}
          </CardContent>
        </Card>
      </div>

      <!-- 固定位置的「词条详情」面板：宽屏时吸在右侧并独立滚动，永远不会被窗口裁掉 -->
      <aside
        class="detail-panel rounded-xl border border-border bg-card text-card-foreground shadow-sm"
        data-testid="token-detail-panel"
        aria-label="词条详情"
      >
        <div class="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
          <span class="text-sm font-semibold">词条详情</span>
          {#if pinnedIndex !== null}
            <Badge variant="secondary">已钉住</Badge>
          {/if}
          <span class="ml-auto text-[11px] text-muted-foreground">悬停查看 · 点击钉住</span>
        </div>

        <div class="detail-panel-body scrollbar-thin p-3">
          <TokenDetail
            token={shownToken}
            {meta}
            {settings}
            curves={tierCurves}
            pinned={pinnedIndex !== null}
            emptyHint="悬停左侧任意词条，这里会固定显示它的频次、排名、占比、分组与各分域排名。"
          />
        </div>

        {#if pinnedIndex !== null}
          <div class="flex items-center justify-between gap-2 border-t border-border px-2 py-1">
            <span class="pl-1 text-[11px] text-muted-foreground">钉住后悬停别的词不会改变这里</span>
            <Button variant="ghost" size="sm" onclick={() => (pinnedIndex = null)}>取消钉住</Button>
          </div>
        {/if}
      </aside>
    </div>
  {/if}
</div>

<style>
  /*
    两栏布局只在这里定义（用普通 CSS + 媒体查询，方便精确控制 1100px 这个断点）：
      宽屏：左内容自适应 + 右侧 320px 固定详情面板（sticky，面板内部独立滚动）；
      窄屏：上下堆叠，面板落到下方。
    两列都用 minmax(0, 1fr) / min-width: 0，避免长词条把网格撑出横向滚动。
  */
  .analysis-layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 1rem;
    align-items: start;
  }

  .analysis-main {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 1rem;
  }

  .detail-panel {
    display: flex;
    min-width: 0;
    flex-direction: column;
  }

  .detail-panel-body {
    min-width: 0;
    /* 内容再宽也不会溢出面板（长词条靠 break-words 换行） */
    overflow-x: hidden;
  }

  @media (min-width: 1100px) {
    .analysis-layout {
      grid-template-columns: minmax(0, 1fr) 320px;
    }

    .detail-panel {
      position: sticky;
      top: 0;
    }

    .detail-panel-body {
      max-height: min(62vh, 560px);
      overflow-y: auto;
    }
  }
</style>
