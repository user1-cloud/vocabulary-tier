<script module lang="ts">
  import type { NavSection } from '$lib/navigation';

  /** 页面内可导航区块：navigation.ts 直接组合它，新增区块只需在这里加一项 + 一个 <SectionCard> */
  export const PAGE_SECTIONS = {
    step1: { id: 'step1', labelKey: 'wordfreq.step1.title' },
    step2: { id: 'step2', labelKey: 'wordfreq.step2.title' },
    step3: { id: 'step3', labelKey: 'wordfreq.step3.title' },
  } as const satisfies Record<string, NavSection>;
</script>

<script lang="ts">
  /**
   * 生成词频表 —— 三步向导。
   *
   *   ① 选语料库目录 → 「探测」调 plan_corpus，展示识别到的表组 / 文件数 / 体积 / 解析规则
   *   ② 配置参数（线程、HMM、数字词、单字、最小词频、表组、TSV、自定义词典）
   *   ③ 选输出目录 → 「开始统计」调 start_scan，订阅 scan:progress 显示进度与日志
   *
   * 所有 Tauri 调用都走 $lib/api/bridge（页面不直接 import @tauri-apps/api）。
   * 浏览器预览时 bridge 会返回内置演示数据，因此整条流程在 `vite dev` 里也能点通。
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
  import { SectionCard } from '$lib/components/ui/section-card';
  import { Separator } from '$lib/components/ui/separator';
  import { Switch } from '$lib/components/ui/switch';
  import TierStatsTable from '$lib/components/analysis/TierStatsTable.svelte';
  import {
    activeDataset,
    cancelScan,
    datasetStatus,
    dictList,
    getSettings,
    isTauri,
    libraryReady,
    onScanDone,
    onScanError,
    onScanProgress,
    openDataset,
    openExternal,
    pickDirectory,
    planCorpus,
    setSettings,
    startScan,
    suggestTableDir,
    type Result,
  } from '$lib/api/bridge';
  import { formatBytes, formatDuration, formatInt, formatTimestamp } from '$lib/format';
  import { tierKeysFrom } from '$lib/tier-colors';
  import { splitMessage, t } from '$lib/i18n.svelte';
  import { NAVIGATE_EVENT } from '$lib/navigation';
  import { takeScanPrefill } from '$lib/scan-prefill.svelte';
  import { tierNamesOf } from '$lib/tiers.svelte';
  import { cn } from '$lib/utils';
  import {
    resolvedDicts,
    type CorpusPlan,
    type DictItem,
    type DomainPlan,
    type Meta,
    type ScanParams,
    type ScanProgress,
    type Settings,
    type TierStat,
  } from '$lib/types';

  // ---------------------------------------------------------------- 状态

  /** 表单状态（②③ 的参数也在里面，保存设置时一并写回） */
  let form = $state({
    corpus: '',
    /** 新增表的表名：只用来问 `suggest_table_dir` 预填输出目录，不进 ScanParams */
    tableName: '',
    out: '',
    threads: 0,
    hmm: true,
    keepDigit: true,
    keepLatin: true,
    skipSingleChar: false,
    minCount: 1,
    skipDomainTables: false,
    writeTsv: true,
  });

  /**
   * 词典链（`dicts\` 下的文件名，**顺序有意义：第一个是主词典**）。
   *
   * 空数组 = 用数据文件夹里全部 `.dict`（后端按文件名排序）。这是"我没特别指定"
   * 的意思，与"我全勾上了"在界面上要区分开，所以单独用这个数组表示勾选。
   */
  let selectedDicts = $state<string[]>([]);

  /** 数据文件夹里的词典清单（进页面拉一次；`dict_list` 会逐个读文件，别反复调） */
  let dictItems = $state<DictItem[]>([]);
  let dictsLoading = $state(true);

  /** 数据文件夹里有没有可用词典：false 时禁止开始统计（空词典会跑出一张只有单字的废表） */
  let libReady = $state(true);
  let libReadyChecked = $state(false);

  /**
   * 后端当前打开的那张表（`active_dataset()`）。
   *
   * 本页不靠它渲染数据，但「现在到底有没有表」要在页面上说清楚 —— 否则用户切到
   * 划句分析页看到「还没有可用的词频表」会以为是这里弄坏了。统计完成后自动刷新。
   */
  let activeMeta = $state<Meta | null>(null);

  /** 只统计部分表组（空数组 = 全部） */
  let onlyDomains = $state<string[]>([]);

  let plan = $state<CorpusPlan | null>(null);
  let planning = $state(false);
  let planError = $state('');

  let scanning = $state(false);
  let scanError = $state('');

  /** 进度事件累积 */
  let progress = $state({
    phase: '',
    domain: '',
    bytesDone: 0,
    bytesTotal: 0,
    unitsDone: 0,
    unitsTotal: 0,
    percent: 0,
  });
  let planEvent = $state<{ files: number; bytes: number; domains: { name: string; files: number; bytes: number }[]; warnings: string[] } | null>(null);
  let logs = $state<{ level: string; message: string }[]>([]);
  let tableEvents = $state<{ table: string; entries: number; total_tokens: number; tier_stats: TierStat[] }[]>([]);

  /** scan:done 的产物 */
  let result = $state<Meta | null>(null);
  let savedHint = $state('');

  let pickerHint = $state('');
  let loading = $state(true);

  // ---------------------------------------------------------------- 派生

  const canStart = $derived(
    form.corpus.trim().length > 0 && form.out.trim().length > 0 && !scanning && libReady
  );

  /** 可用的词典（坏文件不给勾：勾了后端也会当作不存在，静默跳过反而更迷惑） */
  const usableDicts = $derived(dictItems.filter((item) => item.error === null));

  /** 勾选的词典按**勾选顺序**给出的名字（第一个是主词典） */
  const dictChain = $derived(
    selectedDicts
      .map((file) => dictItems.find((item) => item.file_name === file))
      .filter((item): item is DictItem => item !== undefined)
  );

  /** 没勾任何词典时实际会用到的（后端按文件名排序取全部 `.dict`） */
  const effectiveChain = $derived(
    [...(selectedDicts.length > 0 ? dictChain : usableDicts)].sort((a, b) =>
      a.file_name.localeCompare(b.file_name)
    )
  );

  /** 进度条百分比：优先用字节口径，退回落 percent 字段 */
  const progressPercent = $derived.by(() => {
    const { bytesDone, bytesTotal, percent } = progress;
    const value = bytesTotal > 0 ? (bytesDone / bytesTotal) * 100 : percent;
    return Math.min(100, Math.max(0, Number.isFinite(value) ? value : 0));
  });

  const resultTables = $derived(result?.tables ?? []);

  /**
   * 产物记录的词典链（`dicts[0]` 是主词典）。
   *
   * `dict` / `user_dict` 是 v1 老字段、只读不写，所以统一走 `resolvedDicts()`
   * （它会在 `dicts` 为空时回退到那两个老字段），不要在模板里直接用它们。
   */
  const resultDicts = $derived(resolvedDicts(result?.tokenizer));

  const stats = $derived.by(() => {
    // 概览数字（来自 meta.totals），避免模板里反复写 result.totals.x
    const totals = result?.totals;
    return totals
      ? [
          { label: t('wordfreq.stat.files'), value: formatInt(totals.files) },
          { label: t('wordfreq.stat.bytes'), value: formatBytes(totals.bytes) },
          { label: t('wordfreq.stat.lines'), value: formatInt(totals.lines) },
          { label: t('wordfreq.stat.paras'), value: formatInt(totals.paras) },
          // 「token」是技术单位，不翻译
          { label: 'token', value: formatInt(totals.tokens) },
          { label: t('wordfreq.stat.badLines'), value: formatInt(totals.bad_lines) },
        ]
      : [];
  });

  const steps = $derived([
    { index: 1, label: t('wordfreq.step.corpus'), done: plan !== null },
    { index: 2, label: t('wordfreq.step.params'), done: plan !== null },
    { index: 3, label: t('wordfreq.step.run'), done: result !== null },
  ]);

  const activeStep = $derived(result !== null || scanning ? 3 : plan !== null ? 2 : 1);

  // ---------------------------------------------------------------- 事件订阅 / 初始化

  // 订阅扫描事件。effect 里只读取稳定的函数引用，所以只会跑一次。
  $effect(() => {
    const offProgress = onScanProgress((event: ScanProgress) => {
      if (event.event === 'log') {
        logs = [...logs, { level: event.level, message: event.message }];
        return;
      }
      if (event.event === 'plan') {
        planEvent = {
          files: event.files,
          bytes: event.bytes,
          domains: event.domains,
          warnings: event.warnings,
        };
        return;
      }
      if (event.event === 'phase') {
        progress = {
          phase: event.phase,
          domain: event.domain,
          bytesDone: event.bytes_done,
          bytesTotal: event.bytes_total,
          unitsDone: event.units_done,
          unitsTotal: event.units_total,
          percent: event.percent,
        };
        return;
      }
      tableEvents = [...tableEvents, event];
    });

    const offDone = onScanDone((meta: Meta) => {
      scanning = false;
      result = meta;
      planEvent = null;
      logs = [...logs, { level: 'info', message: t('wordfreq.logDone') }];
      // 让后端把新产物装入缓存，其它页面立刻可用
      void (async () => {
        const target = form.out.trim();
        if (!target) return;
        const opened = await openDataset(target);
        if (!opened.ok) {
          logs = [...logs, { level: 'warn', message: t('wordfreq.logOpenFailed', { error: opened.error }) }];
        }
        // 后端现在已经打开这张新表了 → 刷新页首那句「当前表」
        const active = await activeDataset();
        if (active.ok) activeMeta = active.data;
        await persistSettings();
      })();
    });

    const offError = onScanError((message: string) => {
      scanning = false;
      scanError = message;
      logs = [...logs, { level: 'error', message }];
    });

    return () => {
      offProgress();
      offDone();
      offError();
    };
  });

  // 载入上次的设置并自动探测（只在挂载时跑一次）
  $effect(() => {
    void bootstrap();
  });

  async function bootstrap() {
    await loadLibrary();

    const res = await getSettings();
    if (res.ok) applySettings(res.data);

    // 「重新统计」带来的交接单：它比设置更具体，所以最后覆盖一次
    const prefill = takeScanPrefill();
    if (prefill) {
      if (prefill.corpus) form.corpus = prefill.corpus;
      if (prefill.tableName) form.tableName = prefill.tableName;
      if (prefill.out) {
        form.out = prefill.out;
        outTouched = true;
      }
      if (prefill.dictFiles) selectedDicts = [...prefill.dictFiles];
      if (prefill.threads !== undefined) form.threads = prefill.threads;
      if (prefill.hmm !== undefined) form.hmm = prefill.hmm;
      if (prefill.minCount !== undefined) form.minCount = prefill.minCount;
      if (prefill.keepDigit !== undefined) form.keepDigit = prefill.keepDigit;
      if (prefill.keepLatin !== undefined) form.keepLatin = prefill.keepLatin;
      if (prefill.skipSingleChar !== undefined) form.skipSingleChar = prefill.skipSingleChar;
      if (prefill.skipDomainTables !== undefined) form.skipDomainTables = prefill.skipDomainTables;
      if (prefill.writeTsv !== undefined) form.writeTsv = prefill.writeTsv;
      if (prefill.onlyDomains) onlyDomains = [...prefill.onlyDomains];
    }

    if (form.corpus.trim()) await probe(form.corpus.trim());
    loading = false;
  }

  /** 拉一次「当前有没有打开表」+ 词典清单，并让输出目录跟着表名走 */
  async function loadLibrary() {
    dictsLoading = true;
    const [dictRes, readyRes, activeRes] = await Promise.all([
      dictList(),
      libraryReady(),
      activeDataset(),
    ]);
    if (dictRes.ok) dictItems = dictRes.data;
    if (readyRes.ok) libReady = readyRes.data;
    if (activeRes.ok) activeMeta = activeRes.data;
    libReadyChecked = true;
    dictsLoading = false;
    await syncSuggestedOut();
  }

  /** 当前输出目录是不是「系统按表名算出来的」那个（是的话就允许自动跟随表名改） */
  function isSuggestedOut(value: string): boolean {
    const trimmed = value.trim();
    if (!trimmed) return true;
    return effectiveChain.length > 0 && suggestedPaths.some((path) => path === trimmed);
  }

  function applySettings(settings: Settings) {
    const previousName = form.tableName;
    form = {
      corpus: settings.corpusDir ?? form.corpus,
      tableName: previousName || defaultTableName(settings.corpusDir),
      // 输出目录不再取 dataDir：那个字段现在是「数据文件夹」，不是产物目录
      out: form.out,
      threads: settings.threads ?? 0,
      hmm: settings.hmm,
      keepDigit: settings.keepDigit,
      keepLatin: settings.keepLatin,
      skipSingleChar: settings.skipSingleChar,
      minCount: settings.minCount ?? 1,
      skipDomainTables: settings.skipDomainTables,
      writeTsv: form.writeTsv,
    };
    if (settings.scanDicts) selectedDicts = [...settings.scanDicts];
  }

  /** 用户没填表名时的默认值：用「新的词频表」（后端会洗成合法目录名） */
  function defaultTableName(corpusDir: string | null | undefined): string {
    const base = (corpusDir ?? '').split(/[\\/]/).filter(Boolean).pop();
    return base ? t('wordfreq.step3.defaultTableName', { corpus: base }) : t('wordfreq.step3.newTable');
  }

  /** 把当前表单参数写回全局设置，让其它页面 / 下次启动沿用 */
  async function persistSettings() {
    const current = await getSettings();
    if (!current.ok) return;
    const merged: Settings = {
      ...current.data,
      corpusDir: form.corpus.trim() || null,
      threads: form.threads,
      hmm: form.hmm,
      keepDigit: form.keepDigit,
      keepLatin: form.keepLatin,
      skipSingleChar: form.skipSingleChar,
      minCount: form.minCount,
      skipDomainTables: form.skipDomainTables,
      // 词典链：空数组与 null 在后端是同一个意思，但显式给 null 更好读
      scanDicts: selectedDicts.length > 0 ? [...selectedDicts] : null,
    };
    const saved = await setSettings(merged);
    savedHint = saved.ok ? t('wordfreq.settingsSaved') : t('wordfreq.settingsSaveFailed', { error: saved.error });
  }

  // ---------------------------------------------------------------- 输出目录预填

  /**
   * 『表名 → 建议输出目录』的记录。
   *
   * 需要在两处判断"当前输出目录还是不是系统预填的那个"：
   *   1. 表名输入框每次变化时（见下面的 `$effect`）；
   *   2. 词典清单 / 数据文件夹就绪之前，`suggest_table_dir` 还调不了。
   * 记下来就不必反复问后端（它是同步命令，但每次输入都问一遍没必要）。
   */
  let suggestedPaths = $state<string[]>([]);
  /** 用户是否手动改过输出目录（改过就不再被表名覆盖） */
  let outTouched = $state(false);

  /** 用表名问一次 `suggest_table_dir`，把输出目录预填上 */
  async function syncSuggestedOut(force = false) {
    const name = form.tableName.trim();
    if (!name) return;
    const res = await suggestTableDir(name);
    if (!res.ok || !res.data) return;
    if (!suggestedPaths.includes(res.data)) suggestedPaths = [...suggestedPaths, res.data];
    if (force || (!outTouched && isSuggestedOut(form.out))) form.out = res.data;
  }

  // 表名变化 → 重新预填输出目录（用户自己改过输出目录就不动它）
  $effect(() => {
    const name = form.tableName;
    if (!name.trim() || loading) return;
    void syncSuggestedOut();
  });

  // ---------------------------------------------------------------- 动作

  async function chooseDirectory(target: 'corpus' | 'out') {
    pickerHint = '';
    const res = await pickDirectory(
      target === 'corpus' ? t('wordfreq.pickCorpusDir') : t('wordfreq.pickOutDir')
    );
    if (!res.ok) {
      pickerHint = res.error;
      return;
    }
    if (res.data === null) return; // 用户取消
    if (target === 'corpus') {
      form.corpus = res.data;
      await probe(res.data);
    } else {
      form.out = res.data;
    }
  }

  async function probe(corpus?: string) {
    const target = (corpus ?? form.corpus).trim();
    if (!target) {
      planError = t('wordfreq.needCorpus');
      return;
    }
    planning = true;
    planError = '';
    result = null;
    const res: Result<CorpusPlan> = await planCorpus(target);
    planning = false;
    if (!res.ok) {
      plan = null;
      planError = res.error;
      return;
    }
    plan = res.data;
    form.corpus = res.data.corpus || target;
    // 表组列表变了，把已选但已不存在的表组清掉
    const names = new Set(res.data.domains.map((d) => d.name));
    onlyDomains = onlyDomains.filter((name) => names.has(name));
  }

  async function runScan() {
    scanError = '';
    savedHint = '';
    result = null;
    logs = [];
    tableEvents = [];
    planEvent = null;
    progress = {
      phase: '',
      domain: '',
      bytesDone: 0,
      bytesTotal: 0,
      unitsDone: 0,
      unitsTotal: 0,
      percent: 0,
    };

    const params: ScanParams = {
      corpus: form.corpus.trim(),
      out: form.out.trim(),
      // 词典链（`dicts\` 下的文件名，第一个是主词典）；空数组 = 用全部 `.dict`
      dictFiles: [...selectedDicts],
      threads: Number.isFinite(form.threads) ? Math.max(0, Math.trunc(form.threads)) : 0,
      hmm: form.hmm,
      minCount: Number.isFinite(form.minCount) ? Math.max(1, Math.trunc(form.minCount)) : 1,
      keepDigit: form.keepDigit,
      keepLatin: form.keepLatin,
      skipSingleChar: form.skipSingleChar,
      onlyDomains: [...onlyDomains],
      skipDomainTables: form.skipDomainTables,
      writeTsv: form.writeTsv,
    };

    const res = await startScan(params);
    if (!res.ok) {
      scanError = res.error;
      return;
    }
    scanning = true;
    logs = [{ level: 'info', message: t('wordfreq.logSubmitted', { corpus: params.corpus, out: params.out }) }];
  }

  async function stopScan() {
    const res = await cancelScan();
    if (!res.ok) {
      scanError = res.error;
      return;
    }
    scanning = false;
    logs = [...logs, { level: 'warn', message: t('wordfreq.logCancelled') }];
  }

  function toggleDomain(name: string, checked: boolean) {
    onlyDomains = checked ? [...onlyDomains, name] : onlyDomains.filter((item) => item !== name);
  }

  /**
   * 勾选 / 取消一份词典。
   *
   * 新勾的**追加到末尾**（而不是按文件名插回原位）：词典链的顺序会影响同名条目的
   * 覆盖结果，用户按自己想要的优先级依次勾选是最自然的表达方式。想调顺序就取消
   * 再重勾，界面上"第 2 份"这类徽标会跟着变。
   */
  function toggleDict(fileName: string, checked: boolean) {
    if (checked) {
      if (!selectedDicts.includes(fileName)) selectedDicts = [...selectedDicts, fileName];
      return;
    }
    selectedDicts = selectedDicts.filter((item) => item !== fileName);
  }

  function goDicts() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'dicts' }));
  }

  function goTables() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'tables' }));
  }

  function ruleList(domain: DomainPlan): string {
    return domain.rules.length > 0 ? domain.rules.join(' · ') : t('wordfreq.plan.noRule');
  }

  async function refreshDataset() {
    const res = await datasetStatus(form.out.trim() || null);
    if (!res.ok) return;
    if (!res.data.exists) {
      savedHint = t('wordfreq.datasetUnavailable', { dir: res.data.dir });
      return;
    }
    // 目录可用时顺手让后端装载产物（并按 meta.tokenizer 重建分词器）
    const opened = await openDataset(res.data.dir);
    savedHint = opened.ok
      ? t('wordfreq.datasetLoaded', { dir: res.data.dir, tables: opened.data.tables.length })
      : t('wordfreq.datasetLoadFailed', { dir: res.data.dir, error: opened.error });
  }

  function logTone(level: string): string {
    if (level === 'error') return 'text-red-600 dark:text-red-400';
    if (level === 'warn' || level === 'warning') return 'text-amber-600 dark:text-amber-400';
    if (level === 'debug') return 'text-muted-foreground/70';
    return 'text-foreground/80';
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      {t('wordfreq.browserPreview')}
    </div>
  {/if}

  <!-- 当前打开的那张表：统计完成后会自动刷新；没有表时给个去词频表管理的入口 -->
  <div
    class="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-surface-muted/40 px-3 py-2 text-xs"
    data-testid="current-table"
  >
    {#if activeMeta}
      <span class="text-muted-foreground">{t('wordfreq.currentTable.label')}</span>
      <span class="font-medium">{activeMeta.corpus_root || '—'}</span>
      <span class="text-[11px] text-muted-foreground">
        {t('wordfreq.currentTable.meta', {
          generated: formatTimestamp(activeMeta.generated_at),
          tables: formatInt(activeMeta.tables.length),
        })}
      </span>
    {:else}
      <span class="text-muted-foreground" data-testid="no-current-table">
        {t('wordfreq.currentTable.none')}
      </span>
    {/if}
    <Button variant="ghost" size="sm" class="ml-auto" onclick={goTables}>
      {t('wordfreq.currentTable.manage')}
    </Button>
  </div>

  <!-- 步骤条 -->
  <div class="flex items-center gap-2">
    {#each steps as step (step.index)}
      <div
        class={cn(
          'flex items-center gap-2 rounded-full border px-3 py-1 text-xs transition-colors',
          step.index === activeStep
            ? 'border-primary/40 bg-primary/10 font-medium text-primary'
            : step.done
              ? 'border-border text-foreground/80'
              : 'border-border text-muted-foreground'
        )}
      >
        <span
          class={cn(
            'flex size-4 items-center justify-center rounded-full text-[10px]',
            step.done ? 'bg-primary text-primary-foreground' : 'bg-surface-muted'
          )}
        >
          {step.done ? '✓' : step.index}
        </span>
        <span>{step.label}</span>
      </div>
      {#if step.index < steps.length}
        <span class="h-px flex-1 bg-border"></span>
      {/if}
    {/each}
  </div>

  <!-- ① 语料库 -->
  <SectionCard section={PAGE_SECTIONS.step1} descriptionKey="wordfreq.step1.description">
    {#snippet titleExtra()}<Badge variant="outline">plan_corpus</Badge>{/snippet}
      <div class="flex flex-wrap items-center gap-2">
        <Input
          bind:value={form.corpus}
          placeholder={t('wordfreq.step1.corpusPlaceholder')}
          class="min-w-64 flex-1 font-mono text-xs"
          aria-label={t('wordfreq.step1.corpusLabel')}
          oninput={() => (plan = null)}
        />
        <Button variant="outline" size="sm" onclick={() => chooseDirectory('corpus')}>
          {t('common.browse')}
        </Button>
        <Button size="sm" disabled={planning || !form.corpus.trim()} onclick={() => probe()}>
          {planning ? t('wordfreq.step1.probing') : t('wordfreq.step1.probe')}
        </Button>
      </div>

      {#if pickerHint}
        <p class="text-xs text-muted-foreground">{pickerHint}</p>
      {/if}

      {#if planError}
        <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          {planError}
        </p>
      {/if}

      {#if plan}
        <!-- 句子里有加粗的值：整句留在消息表，按占位符切分后把值插进 <b> 里 -->
        {@const segDomains = splitMessage('wordfreq.plan.domains', ['count'])}
        {@const segFiles = splitMessage('wordfreq.plan.files', ['count'])}
        {@const segBytes = splitMessage('wordfreq.plan.bytes', ['value'])}
        {@const segRuleHint = splitMessage('wordfreq.plan.ruleHint', ['field'])}
        <div class="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-lg bg-surface-muted/50 px-3 py-2 text-xs">
          <span class="text-muted-foreground">
            {segDomains[0]}<b class="text-foreground">{plan.domains.length}</b>{segDomains[1]}
          </span>
          <span class="text-muted-foreground">
            {segFiles[0]}<b class="text-foreground">{formatInt(plan.files)}</b>{segFiles[1]}
          </span>
          <span class="text-muted-foreground">
            {segBytes[0]}<b class="text-foreground">{formatBytes(plan.bytes)}</b>{segBytes[1]}
          </span>
          <span class="selectable truncate font-mono text-[11px] text-muted-foreground">{plan.corpus}</span>
        </div>

        <div class="overflow-x-auto rounded-lg border border-border">
          <table class="w-full border-collapse text-xs">
            <thead class="bg-surface-muted/60 text-muted-foreground">
              <tr>
                <th class="px-3 py-2 text-left font-medium">{t('wordfreq.plan.col.include')}</th>
                <th class="px-3 py-2 text-left font-medium">{t('wordfreq.plan.col.domain')}</th>
                <th class="px-3 py-2 text-right font-medium">{t('wordfreq.stat.files')}</th>
                <th class="px-3 py-2 text-right font-medium">{t('wordfreq.stat.bytes')}</th>
                <th class="px-3 py-2 text-left font-medium">{t('wordfreq.plan.col.rules')}</th>
              </tr>
            </thead>
            <tbody>
              {#each plan.domains as domain (domain.name)}
                <tr class="border-t border-border/70">
                  <td class="px-3 py-2">
                    <input
                      type="checkbox"
                      class="size-3.5 accent-[var(--primary)]"
                      checked={onlyDomains.includes(domain.name)}
                      onchange={(event) => toggleDomain(domain.name, event.currentTarget.checked)}
                      aria-label={t('wordfreq.plan.onlyThisDomain', { domain: domain.name })}
                    />
                  </td>
                  <td class="px-3 py-2 font-medium">{domain.name}</td>
                  <td class="px-3 py-2 text-right tabular-nums">{formatInt(domain.files)}</td>
                  <td class="px-3 py-2 text-right tabular-nums">{formatBytes(domain.bytes)}</td>
                  <td class="px-3 py-2 font-mono text-[11px] text-muted-foreground">{ruleList(domain)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        <p class="text-[11px] text-muted-foreground">
          {segRuleHint[0]}<span class="font-mono">onlyDomains</span>{segRuleHint[1]}
        </p>
      {/if}
  </SectionCard>

  <!-- ② 参数 -->
  <SectionCard section={PAGE_SECTIONS.step2} descriptionKey="wordfreq.step2.description" contentClass="flex flex-col gap-4">
    {#snippet titleExtra()}<Badge variant="outline">ScanParams</Badge>{/snippet}
      <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">{t('wordfreq.step2.threads')}</span>
          <Input type="number" min="0" bind:value={form.threads} class="text-xs" />
        </label>
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">{t('wordfreq.step2.minCount')}</span>
          <Input type="number" min="1" bind:value={form.minCount} class="text-xs" />
        </label>
      </div>

      <Separator />

      <!-- 词典链：多选，**顺序有意义**（第一个是主词典） -->
      <div class="flex flex-col gap-2" data-testid="dict-chain-picker">
        <div class="flex flex-wrap items-center gap-2">
          <span class="text-xs font-medium">{t('wordfreq.step2.dictChain')}</span>
          <Badge variant="outline">{t('wordfreq.step2.dictChainBadge')}</Badge>
          {#if selectedDicts.length > 0}
            <Button variant="ghost" size="sm" class="ml-auto" onclick={() => (selectedDicts = [])}>
              {t('wordfreq.step2.dictUseAll')}
            </Button>
          {/if}
          <Button
            variant="outline"
            size="sm"
            class={selectedDicts.length > 0 ? '' : 'ml-auto'}
            onclick={goDicts}
          >
            {t('wordfreq.step2.goDicts')}
          </Button>
        </div>
        <p class="text-[11px] leading-relaxed text-muted-foreground">
          {t('wordfreq.step2.dictChainHint')}
        </p>

        {#if dictsLoading}
          <p class="text-[11px] text-muted-foreground">{t('wordfreq.step2.dictsLoading')}</p>
        {:else if usableDicts.length === 0}
          <p
            class="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive"
            data-testid="no-usable-dict"
          >
            {t('wordfreq.step2.noUsableDict')}
          </p>
        {:else}
          <!-- 勾选顺序 = 词典链顺序；第一条 = 主词典 -->
          <ol class="flex flex-col gap-1.5">
            {#each dictItems as item (item.file_name)}
              {@const order = selectedDicts.indexOf(item.file_name)}
              <li
                class={cn(
                  'flex flex-wrap items-center gap-2 rounded-lg border px-3 py-2 text-xs',
                  item.error
                    ? 'border-destructive/30 bg-destructive/5'
                    : order >= 0
                      ? 'border-primary/40 bg-primary/5'
                      : 'border-border'
                )}
                data-dict-option={item.file_name}
              >
                <input
                  type="checkbox"
                  class="size-3.5 accent-[var(--primary)]"
                  data-dict-checkbox={item.file_name}
                  checked={order >= 0}
                  disabled={item.error !== null}
                  onchange={(event) => toggleDict(item.file_name, event.currentTarget.checked)}
                  aria-label={t('wordfreq.step2.dictCheckAria', { name: item.file_name })}
                />
                {#if order >= 0}
                  <Badge variant={order === 0 ? 'default' : 'secondary'} data-dict-order={order}>
                    {order === 0
                      ? t('wordfreq.step2.dictPrimary')
                      : t('wordfreq.step2.dictOrder', { index: order + 1 })}
                  </Badge>
                {/if}
                <span class="font-medium">{item.dict.name || item.file_name}</span>
                <span class="text-[11px] text-muted-foreground">
                  {t('dicts.entries', { count: formatInt(item.report.entries || item.dict.entries) })}
                </span>
                {#if item.error}
                  <span class="text-[11px] text-destructive">{t('dicts.brokenBadge')}</span>
                {/if}
                <span class="ml-auto font-mono text-[11px] text-muted-foreground">{item.file_name}</span>
              </li>
            {/each}
          </ol>

          <p class="text-[11px] text-muted-foreground" data-testid="effective-chain">
            {#if selectedDicts.length === 0}
              {t('wordfreq.step2.dictEffectiveAll', {
                names: effectiveChain.map((item) => item.dict.name || item.file_name).join(t('common.listSeparator')) || '—',
              })}
            {:else}
              {t('wordfreq.step2.dictEffective', {
                names: dictChain.map((item) => item.dict.name || item.file_name).join(' → '),
              })}
            {/if}
          </p>
        {/if}
      </div>

      <Separator />

      <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">{t('wordfreq.step2.hmm')}</p>
            <p class="text-[11px] text-muted-foreground">{t('wordfreq.step2.hmmHint')}</p>
          </div>
          <Switch bind:checked={form.hmm} aria-label={t('wordfreq.step2.hmmAria')} />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">{t('wordfreq.step2.keepDigit')}</p>
            <p class="text-[11px] text-muted-foreground">{t('wordfreq.step2.keepDigitHint')}</p>
          </div>
          <Switch bind:checked={form.keepDigit} aria-label={t('wordfreq.step2.keepDigit')} />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">{t('wordfreq.step2.keepLatin')}</p>
            <p class="text-[11px] text-muted-foreground">{t('wordfreq.step2.keepLatinHint')}</p>
          </div>
          <Switch bind:checked={form.keepLatin} aria-label={t('wordfreq.step2.keepLatin')} />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">{t('wordfreq.step2.skipSingleChar')}</p>
            <p class="text-[11px] text-muted-foreground">
              {t('wordfreq.step2.skipSingleCharHint')}
            </p>
          </div>
          <Switch bind:checked={form.skipSingleChar} aria-label={t('wordfreq.step2.skipSingleChar')} />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">{t('wordfreq.step2.domainTables')}</p>
            <p class="text-[11px] text-muted-foreground">{t('wordfreq.step2.domainTablesHint')}</p>
          </div>
          <Switch
            checked={!form.skipDomainTables}
            onCheckedChange={(checked) => (form.skipDomainTables = !checked)}
            aria-label={t('wordfreq.step2.domainTables')}
          />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">{t('wordfreq.step2.writeTsv')}</p>
            <p class="text-[11px] text-muted-foreground">{t('wordfreq.step2.writeTsvHint')}</p>
          </div>
          <Switch bind:checked={form.writeTsv} aria-label={t('wordfreq.step2.writeTsvAria')} />
        </div>
      </div>
  </SectionCard>

  <!-- ③ 输出与执行 -->
  <SectionCard section={PAGE_SECTIONS.step3} descriptionKey="wordfreq.step3.description">
    {#snippet titleExtra()}<Badge variant="outline">start_scan</Badge>{/snippet}
      <!-- 表名：只用来算默认输出目录（`suggest_table_dir`），不进 ScanParams -->
      <div class="flex flex-wrap items-center gap-2">
        <label class="flex min-w-56 flex-1 flex-col gap-1.5">
          <span class="text-xs font-medium">{t('wordfreq.step3.tableName')}</span>
          <Input
            bind:value={form.tableName}
            placeholder={t('wordfreq.step3.tableNamePlaceholder')}
            class="text-xs"
            aria-label={t('wordfreq.step3.tableName')}
            data-testid="table-name-input"
          />
        </label>
        <span class="mt-4 text-[11px] text-muted-foreground sm:max-w-72">
          {t('wordfreq.step3.tableNameHint')}
        </span>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Input
          bind:value={form.out}
          oninput={() => (outTouched = true)}
          placeholder={t('wordfreq.step3.outPlaceholder')}
          class="min-w-64 flex-1 font-mono text-xs"
          aria-label={t('wordfreq.step3.outLabel')}
          data-testid="out-dir-input"
        />
        <Button variant="outline" size="sm" onclick={() => chooseDirectory('out')}>
          {t('common.browse')}
        </Button>
        <Button variant="outline" size="sm" disabled={!form.out.trim()} onclick={refreshDataset}>
          {t('wordfreq.step3.checkDataset')}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={!form.tableName.trim()}
          onclick={() => {
            outTouched = false;
            void syncSuggestedOut(true);
          }}
        >
          {t('wordfreq.step3.resuggestOut')}
        </Button>
      </div>

      <!-- 数据文件夹里没有可用词典时不放行：空词典会跑出一张只有单字的废表 -->
      {#if libReadyChecked && !libReady}
        <div
          class="flex flex-wrap items-center gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive"
          data-testid="library-not-ready"
        >
          <span class="font-medium">{t('wordfreq.libNotReady.title')}</span>
          <span>{t('wordfreq.libNotReady.body')}</span>
          <Button variant="outline" size="sm" onclick={goDicts}>{t('wordfreq.libNotReady.go')}</Button>
        </div>
      {/if}

      <div class="flex flex-wrap items-center gap-2">
        <Button disabled={!canStart} onclick={runScan}>
          {scanning ? t('wordfreq.step3.scanning') : t('wordfreq.step3.start')}
        </Button>
        <Button variant="outline" disabled={!scanning} onclick={stopScan}>
          {t('wordfreq.step3.cancel')}
        </Button>
        {#if isTauri() && form.out.trim()}
          <Button variant="ghost" size="sm" onclick={() => void openExternal(form.out.trim())}>
            {t('wordfreq.step3.openOutDir')}
          </Button>
        {/if}
        {#if savedHint}
          <span class="text-[11px] text-muted-foreground">{savedHint}</span>
        {/if}
      </div>

      {#if scanError}
        <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          {t('wordfreq.scanFailed', { error: scanError })}
        </p>
      {/if}

      {#if scanning || progress.bytesTotal > 0}
        <!-- 「已处理 X / Y」里 X 加粗、Y 不加粗：整句留在消息表，按 {done} 切两段 -->
        {@const segBytesProgress = splitMessage('wordfreq.progress.bytes', ['done'], {
          total: formatBytes(progress.bytesTotal),
        })}
        {@const segUnitsProgress = splitMessage('wordfreq.progress.units', ['done'], {
          total: formatInt(progress.unitsTotal),
        })}
        <div class="flex flex-col gap-2 rounded-lg border border-border p-3">
          <div class="flex items-center justify-between text-xs">
            <span class="font-medium">
              {progress.phase || t('wordfreq.progress.preparing')}
              {#if progress.domain}{t('wordfreq.progress.domain', { domain: progress.domain })}{/if}
            </span>
            <span class="tabular-nums text-muted-foreground">{progressPercent.toFixed(1)}%</span>
          </div>

          <div class="h-2 w-full overflow-hidden rounded-full bg-surface-muted">
            <div
              class="h-full rounded-full bg-primary transition-[width] duration-200"
              style="width: {progressPercent}%"
            ></div>
          </div>

          <div class="flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
            <span>
              {segBytesProgress[0]}<b class="tabular-nums text-foreground"
                >{formatBytes(progress.bytesDone)}</b
              >{segBytesProgress[1]}
            </span>
            <span>
              {segUnitsProgress[0]}<b class="tabular-nums text-foreground"
                >{formatInt(progress.unitsDone)}</b
              >{segUnitsProgress[1]}
            </span>
          </div>
        </div>
      {/if}

      {#if planEvent}
        <div class="rounded-lg border border-border p-3 text-xs">
          <p class="mb-1 font-medium">{t('wordfreq.planEvent.title')}</p>
          <p class="text-muted-foreground">
            {t('wordfreq.planEvent.summary', {
              files: formatInt(planEvent.files),
              bytes: formatBytes(planEvent.bytes),
            })}
            {planEvent.domains
              .map((d) => `${d.name}(${formatInt(d.files)})`)
              .join(t('common.listSeparator')) || '—'}
          </p>
          {#if planEvent.warnings.length > 0}
            <ul class="mt-1.5 flex flex-col gap-0.5">
              {#each planEvent.warnings as warning, index (index)}
                <li class="text-amber-600 dark:text-amber-400">⚠ {warning}</li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}

      {#if tableEvents.length > 0}
        <div class="flex flex-col gap-1">
          <p class="text-xs font-medium">{t('wordfreq.tables.title')}</p>
          <ul class="flex flex-wrap gap-2">
            {#each tableEvents as table, index (table.table + index)}
              <li class="rounded-md border border-border px-2 py-1 text-[11px]">
                <span class="font-mono">{table.table}</span>
                <span class="ml-2 text-muted-foreground">
                  {t('wordfreq.tables.entries', {
                    entries: formatInt(table.entries),
                    tokens: formatInt(table.total_tokens),
                  })}
                </span>
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if logs.length > 0}
        <div class="flex flex-col gap-1">
          <p class="text-xs font-medium">{t('wordfreq.logs.title')}</p>
          <div
            class="scrollbar-thin max-h-52 overflow-y-auto rounded-lg border border-border bg-surface-muted/30 p-2 font-mono text-[11px] leading-relaxed"
          >
            {#each logs as entry, index (index)}
              <p class={cn('whitespace-pre-wrap', logTone(entry.level))}>
                [{entry.level}] {entry.message}
              </p>
            {/each}
          </div>
        </div>
      {/if}
  </SectionCard>

  <!-- 结果 -->
  {#if result}
    <!-- 下面几处句子里有 <b>：整句留在消息表，按占位符切分成段后把值插进 <b> 里 -->
    {@const segDone = splitMessage('wordfreq.result.done', ['sentences', 'leaderboard'])}
    {@const segEngine = splitMessage('wordfreq.result.engine', ['engine'], {
      version: result.tokenizer.version,
    })}
    {@const segHmm = splitMessage('wordfreq.result.hmm', ['value'])}
    {@const segDigits = splitMessage('wordfreq.result.digits', ['value'])}
    {@const segLatin = splitMessage('wordfreq.result.latin', ['value'])}
    {@const segSkip = splitMessage('wordfreq.result.skipSingle', ['value'])}
    <Card class="border-primary/30">
      <CardHeader>
        <div class="flex items-center gap-2">
          <CardTitle>{t('wordfreq.result.title')}</CardTitle>
          <Badge variant="success">scan:done</Badge>
        </div>
        <CardDescription>
          {t('wordfreq.result.meta', {
            generated: formatTimestamp(result.generated_at),
            elapsed: formatDuration(result.elapsed_ms),
            tool: result.tool_version,
            schema: result.schema_version,
          })}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-4">
        <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
          {#each stats as item (item.label)}
            <div class="rounded-lg border border-border px-3 py-2">
              <p class="text-[11px] text-muted-foreground">{item.label}</p>
              <p class="text-sm font-semibold tabular-nums">{item.value}</p>
            </div>
          {/each}
        </div>

        <div class="flex flex-wrap gap-x-4 gap-y-1 rounded-lg bg-surface-muted/50 px-3 py-2 text-[11px] text-muted-foreground">
          <span>
            {segEngine[0]}<b class="text-foreground"
              >{result.tokenizer.engine} {result.tokenizer.version}</b
            >{segEngine[1]}
          </span>
          <span>
            {segHmm[0]}<b class="text-foreground"
              >{result.tokenizer.hmm ? t('common.on') : t('common.off')}</b
            >{segHmm[1]}
          </span>
          <span>
            {segDigits[0]}<b class="text-foreground"
              >{result.tokenizer.keep_digit ? t('common.keep') : t('common.discard')}</b
            >{segDigits[1]}
          </span>
          <span>
            {segLatin[0]}<b class="text-foreground"
              >{result.tokenizer.keep_latin ? t('common.keep') : t('common.discard')}</b
            >{segLatin[1]}
          </span>
          <span>
            {segSkip[0]}<b class="text-foreground"
              >{result.tokenizer.skip_single_char ? t('common.yes') : t('common.no')}</b
            >{segSkip[1]}
          </span>
          <span class="selectable truncate">
            {t('wordfreq.result.dicts', {
              names: resultDicts.map((ref) => ref.name || ref.id).join(t('common.listSeparator')) || '—',
            })}
          </span>
        </div>

        <div class="flex flex-col gap-4">
          {#each resultTables as table (table.path)}
            <div class="flex flex-col gap-2">
              <div class="flex flex-wrap items-center gap-2">
                <p class="font-mono text-xs font-medium">{table.path}</p>
                <Badge variant="secondary">
                  {table.kind === 'char' ? t('table.char') : t('table.word')}
                </Badge>
                <span class="text-[11px] text-muted-foreground">
                  {t('wordfreq.tables.summary', {
                    entries: formatInt(table.entries),
                    tokens: formatInt(table.total_tokens),
                    bytes: formatBytes(table.vfr_bytes),
                  })}
                </span>
              </div>
              <TierStatsTable
                stats={table.tier_stats}
                tiers={table.tiers}
                labels={tierNamesOf(result)}
                keys={tierKeysFrom(result)}
              />
            </div>
          {/each}
        </div>

        <div class="rounded-lg border border-dashed border-primary/40 bg-primary/5 px-3 py-2 text-xs">
          {segDone[0]}<b>{t('nav.sentences.label')}</b>{segDone[1]}<b
            >{t('nav.leaderboard.label')}</b
          >{segDone[2]}
        </div>
      </CardContent>
    </Card>
  {/if}

  {#if loading}
    <p class="text-xs text-muted-foreground">{t('wordfreq.loading')}</p>
  {/if}
</div>
