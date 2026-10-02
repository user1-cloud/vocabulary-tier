<script lang="ts">
  /**
   * 划句分析 —— 核心页面。
   *
   * 交互链路：
   *   文本框粘贴 / 划选 → 选中「分析全文 / 只分析选中」→ analyze_text
   *   → 按 token 渲染着色（颜色来自 tier-colors.ts，分组阈值来自 meta.tables）
   *   → 悬停 token 时在**右侧固定面板**里显示词频 / 排名 / 前% / 各表对比 / 词典标记。
   *
   * 详情为什么不做成跟随鼠标的浮层：浮层靠近窗口边缘会被裁掉，读不全；
   * 固定面板永远在窗口内，内容长了自己滚。布局样式见文件末尾。
   *
   * **着色与分组只看主词频表**（设置里的 `primaryScope`）；这里的「对比范围」勾的是
   * 对比列里显示哪些**作用域**，空数组 = 全显示（后端 `domains` 参数沿用旧名）。
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
    activeDataset,
    activeTableDir,
    analyzeText,
    captureSelection,
    copyText,
    isTauri,
    libraryInfo,
    openPopup,
  } from '$lib/api/bridge';
  import { formatInt, formatPct, formatTimestamp, findTable } from '$lib/format';
  import { NAVIGATE_EVENT } from '$lib/navigation';
  import { summarizeTokens } from '$lib/segments';
  import { tierKeysFrom } from '$lib/tier-colors';
  import { t } from '$lib/i18n.svelte';
  import { activeBounds, activeBoundsInfo, activeTierIndex, appSettings, primaryScope, primaryTableKey, tierCurves, tierNamesOf } from '$lib/tiers.svelte';
  import { cn } from '$lib/utils';
  import { metaScopes, resolvedDicts, type Meta, type TokenInfo } from '$lib/types';

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

  /**
   * 当前打开那张表的 `meta`。
   *
   * 开机引导统一走 `active_dataset()`：后端启动时就按 `activeTable` /
   * `activeTablePath` 打开好表了，页面**只需要取**。不要再拿 `settings.dataDir`
   * 去拼路径 —— 它现在是「数据文件夹」（里面是 `dicts\` 与 `tables\`），
   * 下面没有 `meta.json`，拼出来必然是「尚未打开词频表」。
   */
  let activeMeta = $state<Meta | null>(null);
  let statusLoading = $state(true);
  let statusError = $state('');
  /** 后端当前打开的那张表的产物目录（只用于展示，`analyze_text` 不再需要它） */
  let activeDir = $state<string | null>(null);

  /** 对比列里保留哪些**作用域**（空数组 = 全部；着色只看主表） */
  let compareScopes = $state<string[]>([]);

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

  const meta: Meta | null = $derived(activeMeta);
  const ready = $derived(meta !== null);

  const selectionText = $derived(text.slice(selectionStart, selectionEnd));

  /** 实际送去分析的文本 */
  const payload = $derived.by(() => {
    if (mode === 'selection') return selectionText;
    return text;
  });

  const summary = $derived(summarizeTokens(tokens, tierIndexOfToken));

  const settings = $derived(appSettings.value);

  /** 生效阈值：默认 = meta 里的默认分组；用户在「表管理」页改过就是自定义的 */
  /**
   * 生效阈值：默认 = meta 里的默认分组；用户在「表管理」页改过就是自定义的。
   *
   * 词表阈值给图例用；字表阈值不用在这里算 —— 详情面板（TokenDetail）自己按
   * token 是词还是字调用 `boundsInfo` 现算，保证与着色用同一套权威实现。
   */
  const wordBounds = $derived(meta ? activeBounds('word', meta) : []);
  const wordBoundsWarning = $derived(meta ? activeBoundsInfo('word', meta).warning : null);
  const charBoundsWarning = $derived(meta ? activeBoundsInfo('char', meta).warning : null);

  const wordTable = $derived(meta ? findTable(meta.tables, 'word') : undefined);
  const charTable = $derived(meta ? findTable(meta.tables, 'char') : undefined);

  const lineCount = $derived(text.length === 0 ? 0 : text.split('\n').length);

  /** 分组标签（展示用；本地化在 `i18n.svelte.ts::tierLabels()`） */
  const legendNames = $derived(tierNamesOf(meta));

  /** 七组稳定标识：取色与身份都走它，不走组名 */
  const legendKeys = $derived(tierKeysFrom(meta));

  /** 详情面板里展示哪一条：钉住的优先，其次最后一次悬停的（鼠标移开也保留） */
  const shownIndex = $derived(pinnedIndex ?? activeIndex);
  const shownToken = $derived(shownIndex === null ? null : (tokens[shownIndex] ?? null));

  /**
   * 一个 token 实际落在哪一组 —— 返回**组号**（0..6），未收录返回 null。
   *
   * 自定义阈值下与后端返回的 `tier` 可能不同；算不出生效组号时回落到 `token.tier`。
   */
  function tierIndexOfToken(token: TokenInfo): number | null {
    if (!meta) return token.tier;
    const kind = token.single_cjk || token.table === 'char' ? 'char' : 'word';
    // 阈值取自**主作用域**那张表：铺平之后它可能是 news 或某张相加表，
    // 写死 full/word 会让这里的颜色与详情面板里的前%对不上。
    const key = primaryTableKey(meta, kind);
    const index = activeTierIndex(kind, token.rank, meta, key);
    return index ?? token.tier;
  }

  // ---------------------------------------------------------------- 生命周期

  $effect(() => {
    void bootstrap();
  });

  // 文本 / 模式 / 对比范围 / 数据集变化时重新分析（带防抖）
  $effect(() => {
    // 显式读取依赖
    const nextPayload = payload;
    const nextScopes = compareScopes;
    const isReady = ready;
    // 不再传产物目录：`analyze_text` 的 dir 为 null 时用后端当前打开的那张表
    return scheduleAnalyze(nextPayload, nextScopes, isReady, null);
  });

  async function bootstrap() {
    statusLoading = true;
    statusError = '';
    const res = await activeDataset();
    if (!res.ok) {
      statusError = res.error;
      activeMeta = null;
      statusLoading = false;
      return;
    }
    activeMeta = res.data;
    // 后端按记录的词库链重建分词器是在激活表时做的，这里只取 meta；
    // 分域默认查全部（空数组），与后端约定一致。
    compareScopes = [];
    // 顺带记下产物目录（只用于页面展示）
    const infoRes = await libraryInfo();
    if (infoRes.ok) activeDir = activeTableDir(infoRes.data);
    statusLoading = false;
  }

  function scheduleAnalyze(
    nextPayload: string,
    compareScopes: string[],
    isReady: boolean,
    nextDir: string | null
  ) {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      void runAnalyze(nextPayload, compareScopes, isReady, nextDir);
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
    compareScopes: string[],
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
    const res = await analyzeText(nextPayload, compareScopes, nextDir);
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

  function toggleScope(name: string, checked: boolean) {
    compareScopes = checked
      ? [...compareScopes, name]
      : compareScopes.filter((item) => item !== name);
  }

  /** 全部**作用域**（表侧）：对比列里可选的那些 */
  const allScopes = $derived(metaScopes(meta));

  function selectAllScopes() {
    compareScopes = [...allScopes];
  }

  function clearScopes() {
    compareScopes = [];
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
      showNotice(t('sentences.notice.emptyNoSend'), 'error');
      return;
    }
    const res = await openPopup(text);
    if (!res.ok) {
      showNotice(res.error, 'error');
      return;
    }
    showNotice(t('sentences.notice.sentToPopup'));
  }

  async function doCaptureSelection() {
    const res = await captureSelection();
    if (!res.ok) {
      showNotice(res.error, 'error');
      return;
    }
    if (!res.data.trim()) {
      showNotice(t('sentences.notice.captureEmpty'));
      return;
    }
    text = res.data;
    mode = 'all';
    showNotice(t('sentences.notice.captureOk'));
  }

  async function copyAll() {
    if (!text.trim()) {
      showNotice(t('sentences.notice.emptyText'), 'error');
      return;
    }
    const res = await copyText(text);
    showNotice(res.ok ? t('sentences.notice.copied') : res.error, res.ok ? 'info' : 'error');
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

  function goDicts() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'dicts' }));
  }

  function tableSummaryLabel(kind: 'word' | 'char'): string {
    const table = kind === 'word' ? wordTable : charTable;
    if (!table) return '—';
    return t('sentences.tableEntries', { entries: formatInt(table.entries) });
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      {t('sentences.browserPreview')}
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
        {t('sentences.checking')}
      </CardContent>
    </Card>
  {:else if statusError}
    <Card class="border-destructive/30">
      <CardContent class="py-6 text-xs text-destructive">
        {t('sentences.statusFailed', { error: statusError })}
      </CardContent>
    </Card>
  {:else if !ready}
    <!-- 还没有打开任何词表：引导去生成词频表 / 词库管理 -->
    <Card class="border-dashed">
      <CardHeader>
        <div class="flex items-center gap-2">
          <CardTitle>{t('sentences.noTable.title')}</CardTitle>
          <Badge variant="outline">active_dataset = null</Badge>
        </div>
        <CardDescription>
          {t('sentences.noTable.description')}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <p class="selectable rounded-md bg-surface-muted/60 px-3 py-2 font-mono text-xs">
          {activeDir || t('sentences.noTable.noDir')}
        </p>
        <ul class="flex flex-col gap-1 text-xs text-muted-foreground">
          <li class="flex gap-2"><span class="mt-1.5 size-1.5 shrink-0 rounded-full bg-primary/60"></span>{t('sentences.noTable.step1')}</li>
          <li class="flex gap-2"><span class="mt-1.5 size-1.5 shrink-0 rounded-full bg-primary/60"></span>{t('sentences.noTable.step2')}</li>
        </ul>
        <div class="flex gap-2">
          <Button onclick={goWordFreq}>{t('sentences.noTable.go')}</Button>
          <Button variant="outline" onclick={goDicts}>{t('sentences.noTable.goDicts')}</Button>
          <Button variant="outline" onclick={() => void bootstrap()}>
            {t('sentences.noTable.recheck')}
          </Button>
        </div>
      </CardContent>
    </Card>
  {:else}
    <!-- 数据集概览 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>{t('sentences.dataset.title')}</CardTitle>
          <Badge variant="success">{t('sentences.dataset.ready')}</Badge>
          <Badge variant="secondary">schema v{meta?.schema_version}</Badge>
          {#if meta?.tokenizer}
            <Badge variant="outline">{meta.tokenizer.engine} {meta.tokenizer.version}</Badge>
            <Badge variant="outline">
              {t('sentences.dataset.hmm', {
                value: meta.tokenizer.hmm ? t('common.on') : t('common.off'),
              })}
            </Badge>
            {#if resolvedDicts(meta.tokenizer).length > 0}
              <Badge variant="outline">
                {t('sentences.dataset.dictChain', {
                  count: resolvedDicts(meta.tokenizer).length,
                  names: resolvedDicts(meta.tokenizer)
                    .map((ref) => ref.name || ref.id)
                    .join(t('common.listSeparator')),
                })}
              </Badge>
            {/if}
          {/if}
        </div>
        <CardDescription>
          {t('sentences.dataset.summary', {
            generated: formatTimestamp(meta?.generated_at),
            tokens: formatInt(meta?.totals.tokens),
            wordTable: tableSummaryLabel('word'),
            charTable: tableSummaryLabel('char'),
          })}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <p class="selectable truncate font-mono text-[11px] text-muted-foreground">{activeDir ?? meta?.corpus_root}</p>
        <TierLegend names={legendNames} keys={legendKeys} bounds={wordBounds} />
        {#if wordBoundsWarning || charBoundsWarning}
          {@const boundsWarning = wordBoundsWarning ?? charBoundsWarning!}
          <p class="rounded-md border border-amber-500/40 bg-amber-500/5 px-2 py-1 text-[11px] text-amber-700 dark:text-amber-300">
            {t(boundsWarning.key, boundsWarning.params)}
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
              <CardTitle>{t('sentences.input.title')}</CardTitle>
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
                  {t('sentences.input.modeAll')}
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
                  {t('sentences.input.modeSelection')}
                </button>
              </div>
            </div>
            <CardDescription>
              {t('sentences.input.description')}
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
              placeholder={t('sentences.input.placeholder')}
              spellcheck="false"
              class={cn(
                'scrollbar-thin min-h-52 w-full resize-y rounded-lg border border-input bg-surface p-3',
                'text-sm leading-relaxed text-foreground placeholder:text-muted-foreground',
                'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background'
              )}
            ></textarea>

            <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
              <span>
                {t('sentences.input.counter', {
                  chars: formatInt(text.length),
                  lines: formatInt(lineCount),
                })}
              </span>
              {#if mode === 'selection'}
                <span>
                  {t('sentences.input.selected', { count: formatInt(selectionText.length) })}
                  {#if selectionText.length === 0}
                    <span class="text-amber-600 dark:text-amber-400">
                      {t('sentences.input.selectHint')}
                    </span>
                  {/if}
                </span>
              {:else}
                <span>{t('sentences.input.scopeAll')}</span>
              {/if}
              {#if analyzing}<span>{t('sentences.input.analyzing')}</span>{/if}
            </div>

            <!-- 分域过滤 -->
            <div class="flex flex-col gap-2 rounded-lg border border-border p-3">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-xs font-medium">{t('sentences.domains.title')}</span>
                <span class="text-[11px] text-muted-foreground">
                  {t('sentences.domains.hint')}
                </span>
                <span class="ml-auto flex gap-1">
                  <Button variant="ghost" size="sm" onclick={selectAllScopes}>
                    {t('sentences.domains.selectAll')}
                  </Button>
                  <Button variant="ghost" size="sm" onclick={clearScopes}>
                    {t('sentences.clear')}
                  </Button>
                </span>
              </div>
              <div class="flex flex-wrap gap-x-4 gap-y-2">
                {#each allScopes as name (name)}
                  {@const table = meta?.tables.find((x) => x.path === name && x.kind === 'word')}
                  <label class="flex cursor-pointer items-center gap-1.5 text-xs">
                    <input
                      type="checkbox"
                      class="size-3.5 accent-[var(--primary)]"
                      checked={compareScopes.includes(name)}
                      onchange={(event) => toggleScope(name, event.currentTarget.checked)}
                    />
                    <span>{name}</span>
                    {#if name === primaryScope(meta)}
                      <span class="rounded border border-primary/40 px-1 text-[10px] text-primary">
                        {t('leaderboard.primaryBadge')}
                      </span>
                    {/if}
                    {#if table}
                      <span class="text-[11px] text-muted-foreground">
                        {t('sentences.domains.entries', { entries: formatInt(table.entries) })}
                      </span>
                    {/if}
                  </label>
                {/each}
              </div>
            </div>

            <Separator />

            <div class="flex flex-wrap items-center gap-2">
              <Button size="sm" onclick={sendToPopup}>{t('sentences.actions.sendToPopup')}</Button>
              <Button variant="outline" size="sm" onclick={() => void doCaptureSelection()}>
                {t('sentences.actions.capture')}
              </Button>
              <Button variant="outline" size="sm" onclick={() => void copyAll()}>
                {t('sentences.actions.copy')}
              </Button>
              <Button variant="ghost" size="sm" onclick={clearAll}>{t('sentences.clear')}</Button>
            </div>
          </CardContent>
        </Card>

        <!-- 分析结果 -->
        <Card>
          <CardHeader>
            <div class="flex flex-wrap items-center gap-2">
              <CardTitle>{t('sentences.result.title')}</CardTitle>
              <Badge variant="outline">
                {t('sentences.result.tokenCount', { count: formatInt(tokens.length) })}
              </Badge>
              <Badge variant="secondary">
                {t('sentences.result.accepted', { count: formatInt(summary.accepted) })}
              </Badge>
              <Badge variant="outline">
                {t('sentences.result.skipped', { count: formatInt(summary.skipped) })}
              </Badge>
              {#if summary.unknownTotal > 0}
                <Badge variant="outline">
                  {t('sentences.result.unknown', {
                    unique: formatInt(summary.unknownUnique),
                    total: formatInt(summary.unknownTotal),
                  })}
                </Badge>
              {/if}
            </div>
            <CardDescription>
              {t('sentences.result.description')}
              <span class="ml-1">{t('sentences.result.thresholdHint')}</span>
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
                  ? t('sentences.empty.selection')
                  : t('sentences.empty.none')}
              </p>
            {:else}
              <!-- 分组命中分布 -->
              <div class="flex flex-wrap gap-2">
                {#each legendNames as name, index (index)}
                  {@const hit = summary.byTier.get(index) ?? 0}
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
                {t('sentences.result.averageBaseline', {
                  word: formatPct(
                    wordTable && wordTable.total_tokens > 0 ? (1 / wordTable.total_tokens) * 100 : null
                  ),
                  char: formatPct(
                    charTable && charTable.total_tokens > 0 ? (1 / charTable.total_tokens) * 100 : null
                  ),
                })}
              </p>
            {/if}
          </CardContent>
        </Card>
      </div>

      <!-- 固定位置的「词条详情」面板：宽屏时吸在右侧并独立滚动，永远不会被窗口裁掉 -->
      <aside
        class="detail-panel rounded-xl border border-border bg-card text-card-foreground shadow-sm"
        data-testid="token-detail-panel"
        aria-label={t('sentences.detail.aria')}
      >
        <div class="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
          <span class="text-sm font-semibold">{t('sentences.detail.title')}</span>
          {#if pinnedIndex !== null}
            <Badge variant="secondary">{t('sentences.detail.pinned')}</Badge>
          {/if}
          <span class="ml-auto text-[11px] text-muted-foreground">{t('sentences.detail.hint')}</span>
        </div>

        <div class="detail-panel-body scrollbar-thin p-3">
          <TokenDetail
            token={shownToken}
            {meta}
            {settings}
            curves={tierCurves}
            pinned={pinnedIndex !== null}
            emptyHint={t('sentences.detail.emptyHint')}
          />
        </div>

        {#if pinnedIndex !== null}
          <div class="flex items-center justify-between gap-2 border-t border-border px-2 py-1">
            <span class="pl-1 text-[11px] text-muted-foreground">
              {t('sentences.detail.pinnedHint')}
            </span>
            <Button variant="ghost" size="sm" onclick={() => (pinnedIndex = null)}>
              {t('sentences.detail.unpin')}
            </Button>
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

  /*
    详情面板正文必须有**固定高度**，不能只给 max-height。
    原因（实测，1440×900）：TokenDetail 的骨架（词头 + 四格 + 徽标行）恒定 108px，
    但它下面的「各分域排名 / 未收录说明 / 阈值回退」长度随 token 变化 ——
    标点几乎为空、未收录最长。高度自适应内容时整个面板会跟着内容变长变短：
      面板总高：标点 171 / 多字词 261 / 未收录 325  →  用户看到的「标点时很小、
      词汇时突然变大」就是它。
    改成固定高度后，面板尺寸恒定，超出的部分在正文里滚动。
    min-height 兜底：内容再短也不会缩成一条（这正是「标点时很小」的另一半原因）。
    106 + 38 + 37 + 34 ≈ 215px 是「骨架 + 徽标行」所需的最小高度，取 300 留出余量。
  */
  .detail-panel-body {
    min-width: 0;
    /* 内容再宽也不会溢出面板（长词条靠 break-words 换行） */
    overflow-x: hidden;
    /* 固定高度（不是 max-height）：375px ≈ 骨架 215px + 常见附加信息余量 */
    height: 375px;
    min-height: 300px;
    overflow-y: auto;
  }

  @media (min-width: 1100px) {
    .analysis-layout {
      grid-template-columns: minmax(0, 1fr) 320px;
    }

    .detail-panel {
      position: sticky;
      top: 0;
    }
  }
</style>
