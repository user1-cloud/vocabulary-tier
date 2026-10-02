<script lang="ts">
  /**
   * 生成词频表 —— 三步向导。
   *
   *   ① 选语料库目录 → 「探测」调 plan_corpus，展示识别到的域 / 文件数 / 体积 / 解析规则
   *   ② 配置参数（线程、HMM、数字词、单字、最小词频、分域表、TSV、自定义词典）
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
  import { Separator } from '$lib/components/ui/separator';
  import { Switch } from '$lib/components/ui/switch';
  import TierStatsTable from '$lib/components/analysis/TierStatsTable.svelte';
  import {
    cancelScan,
    datasetStatus,
    getSettings,
    isTauri,
    onScanDone,
    onScanError,
    onScanProgress,
    openDataset,
    openExternal,
    pickDirectory,
    pickFile,
    planCorpus,
    setSettings,
    startScan,
    type Result,
  } from '$lib/api/bridge';
  import { formatBytes, formatDuration, formatInt, formatTimestamp } from '$lib/format';
  import { cn } from '$lib/utils';
  import type { CorpusPlan, DomainPlan, Meta, ScanParams, ScanProgress, Settings, TierStat } from '$lib/types';

  // ---------------------------------------------------------------- 状态

  /** 表单状态（②③ 的参数也在里面，保存设置时一并写回） */
  let form = $state({
    corpus: '',
    out: '',
    threads: 0,
    hmm: true,
    keepDigit: true,
    keepLatin: true,
    skipSingleChar: false,
    minCount: 1,
    skipDomainTables: false,
    writeTsv: true,
    userDict: '' as string,
  });

  /** 只统计部分分域（空数组 = 全部） */
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

  const canStart = $derived(form.corpus.trim().length > 0 && form.out.trim().length > 0 && !scanning);

  /** 进度条百分比：优先用字节口径，退回落 percent 字段 */
  const progressPercent = $derived.by(() => {
    const { bytesDone, bytesTotal, percent } = progress;
    const value = bytesTotal > 0 ? (bytesDone / bytesTotal) * 100 : percent;
    return Math.min(100, Math.max(0, Number.isFinite(value) ? value : 0));
  });

  const resultTables = $derived(result?.tables ?? []);

  const stats = $derived.by(() => {
    // 概览数字（来自 meta.totals），避免模板里反复写 result.totals.x
    const totals = result?.totals;
    return totals
      ? [
          { label: '文件', value: formatInt(totals.files) },
          { label: '体积', value: formatBytes(totals.bytes) },
          { label: '行数', value: formatInt(totals.lines) },
          { label: '段落', value: formatInt(totals.paras) },
          { label: 'token', value: formatInt(totals.tokens) },
          { label: '跳过行', value: formatInt(totals.bad_lines) },
        ]
      : [];
  });

  const steps = $derived([
    { index: 1, label: '选语料库', done: plan !== null },
    { index: 2, label: '配置参数', done: plan !== null },
    { index: 3, label: '输出与执行', done: result !== null },
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
      logs = [...logs, { level: 'info', message: '统计完成，结果摘要已生成。' }];
      // 让后端把新产物装入缓存，其它页面立刻可用
      void (async () => {
        const target = form.out.trim();
        if (!target) return;
        const opened = await openDataset(target);
        if (!opened.ok) {
          logs = [...logs, { level: 'warn', message: `产物装载失败（其它页面可能还不识别）：${opened.error}` }];
        }
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
    const res = await getSettings();
    if (res.ok) {
      applySettings(res.data);
      if (form.corpus.trim()) await probe(form.corpus.trim());
    }
    loading = false;
  }

  function applySettings(settings: Settings) {
    form = {
      corpus: settings.corpusDir ?? form.corpus,
      out: settings.dataDir ?? form.out,
      threads: settings.threads ?? 0,
      hmm: settings.hmm,
      keepDigit: settings.keepDigit,
      keepLatin: settings.keepLatin,
      skipSingleChar: settings.skipSingleChar,
      minCount: settings.minCount ?? 1,
      skipDomainTables: settings.skipDomainTables,
      writeTsv: form.writeTsv,
      userDict: settings.userDict ?? '',
    };
  }

  /** 把当前表单参数写回全局设置，让其它页面 / 下次启动沿用 */
  async function persistSettings() {
    const current = await getSettings();
    if (!current.ok) return;
    const merged: Settings = {
      ...current.data,
      corpusDir: form.corpus.trim() || null,
      dataDir: form.out.trim() || null,
      threads: form.threads,
      hmm: form.hmm,
      keepDigit: form.keepDigit,
      keepLatin: form.keepLatin,
      skipSingleChar: form.skipSingleChar,
      minCount: form.minCount,
      skipDomainTables: form.skipDomainTables,
      userDict: form.userDict.trim() || null,
    };
    const saved = await setSettings(merged);
    savedHint = saved.ok ? '参数已保存到设置' : `参数保存失败：${saved.error}`;
  }

  // ---------------------------------------------------------------- 动作

  async function chooseDirectory(target: 'corpus' | 'out') {
    pickerHint = '';
    const res = await pickDirectory(target === 'corpus' ? '选择语料库目录' : '选择输出目录');
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

  async function chooseUserDict() {
    pickerHint = '';
    const res = await pickFile('选择自定义词典（每行「词 频次」）');
    if (!res.ok) {
      pickerHint = res.error;
      return;
    }
    if (res.data) form.userDict = res.data;
  }

  async function probe(corpus?: string) {
    const target = (corpus ?? form.corpus).trim();
    if (!target) {
      planError = '请先填写或选择语料库目录。';
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
    // 域列表变了，把已选但已不存在的域清掉
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
      threads: Number.isFinite(form.threads) ? Math.max(0, Math.trunc(form.threads)) : 0,
      hmm: form.hmm,
      userDict: form.userDict.trim() || null,
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
    logs = [{ level: 'info', message: `已提交统计任务：${params.corpus} → ${params.out}` }];
  }

  async function stopScan() {
    const res = await cancelScan();
    if (!res.ok) {
      scanError = res.error;
      return;
    }
    scanning = false;
    logs = [...logs, { level: 'warn', message: '已请求取消统计任务。' }];
  }

  function toggleDomain(name: string, checked: boolean) {
    onlyDomains = checked ? [...onlyDomains, name] : onlyDomains.filter((item) => item !== name);
  }

  function ruleList(domain: DomainPlan): string {
    return domain.rules.length > 0 ? domain.rules.join(' · ') : '（未匹配到显式规则，按默认扩展名扫描）';
  }

  async function refreshDataset() {
    const res = await datasetStatus(form.out.trim() || null);
    if (!res.ok) return;
    if (!res.data.exists) {
      savedHint = `产物目录暂不可用：${res.data.dir}`;
      return;
    }
    // 目录可用时顺手让后端装载产物（并按 meta.tokenizer 重建分词器）
    const opened = await openDataset(res.data.dir);
    savedHint = opened.ok
      ? `已装载产物目录：${res.data.dir}（${opened.data.tables.length} 张表）`
      : `已识别产物目录：${res.data.dir}，但装载失败：${opened.error}`;
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
      浏览器预览模式：正在使用内置演示数据，完整流程可以点通（目录选择器不可用，可直接编辑路径输入框）。
    </div>
  {/if}

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
  <Card>
    <CardHeader>
      <div class="flex items-center gap-2">
        <CardTitle>① 选择语料库目录</CardTitle>
        <Badge variant="outline">plan_corpus</Badge>
      </div>
      <CardDescription>填入或选择语料根目录，先「探测」识别分域、文件数与解析规则。</CardDescription>
    </CardHeader>
    <CardContent class="flex flex-col gap-3">
      <div class="flex flex-wrap items-center gap-2">
        <Input
          bind:value={form.corpus}
          placeholder="例如 D:\corpus 或 /data/corpus"
          class="min-w-64 flex-1 font-mono text-xs"
          aria-label="语料库目录"
          oninput={() => (plan = null)}
        />
        <Button variant="outline" size="sm" onclick={() => chooseDirectory('corpus')}>浏览…</Button>
        <Button size="sm" disabled={planning || !form.corpus.trim()} onclick={() => probe()}>
          {planning ? '探测中…' : '探测'}
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
        <div class="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-lg bg-surface-muted/50 px-3 py-2 text-xs">
          <span class="text-muted-foreground">识别到分域 <b class="text-foreground">{plan.domains.length}</b> 个</span>
          <span class="text-muted-foreground">文件 <b class="text-foreground">{formatInt(plan.files)}</b></span>
          <span class="text-muted-foreground">体积 <b class="text-foreground">{formatBytes(plan.bytes)}</b></span>
          <span class="selectable truncate font-mono text-[11px] text-muted-foreground">{plan.corpus}</span>
        </div>

        <div class="overflow-x-auto rounded-lg border border-border">
          <table class="w-full border-collapse text-xs">
            <thead class="bg-surface-muted/60 text-muted-foreground">
              <tr>
                <th class="px-3 py-2 text-left font-medium">参与统计</th>
                <th class="px-3 py-2 text-left font-medium">分域</th>
                <th class="px-3 py-2 text-right font-medium">文件</th>
                <th class="px-3 py-2 text-right font-medium">体积</th>
                <th class="px-3 py-2 text-left font-medium">解析规则</th>
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
                      aria-label="只统计 {domain.name}"
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
          勾选分域 = 只统计这几项（对应 <span class="font-mono">onlyDomains</span>）；
          不勾选任何一项表示统计全部分域。
        </p>
      {/if}
    </CardContent>
  </Card>

  <!-- ② 参数 -->
  <Card>
    <CardHeader>
      <div class="flex items-center gap-2">
        <CardTitle>② 配置统计参数</CardTitle>
        <Badge variant="outline">ScanParams</Badge>
      </div>
      <CardDescription>这些参数会写进 meta.json，并作为设置页的默认值保存。</CardDescription>
    </CardHeader>
    <CardContent class="flex flex-col gap-4">
      <div class="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">线程数（0 = 自动）</span>
          <Input type="number" min="0" bind:value={form.threads} class="text-xs" />
        </label>
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">最小词频</span>
          <Input type="number" min="1" bind:value={form.minCount} class="text-xs" />
        </label>
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">自定义词典（可选）</span>
          <span class="flex items-center gap-2">
            <Input
              bind:value={form.userDict}
              placeholder="留空表示不启用"
              class="font-mono text-xs"
            />
            <Button variant="outline" size="sm" onclick={chooseUserDict}>选择…</Button>
            {#if form.userDict}
              <Button variant="ghost" size="sm" onclick={() => (form.userDict = '')}>清除</Button>
            {/if}
          </span>
        </label>
      </div>

      <Separator />

      <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">开启 HMM 新词发现</p>
            <p class="text-[11px] text-muted-foreground">识别词典外的连续汉字组合</p>
          </div>
          <Switch bind:checked={form.hmm} aria-label="开启 HMM" />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">保留数字词</p>
            <p class="text-[11px] text-muted-foreground">如 2024、3.14 作为独立词条</p>
          </div>
          <Switch bind:checked={form.keepDigit} aria-label="保留数字词" />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">保留拉丁词</p>
            <p class="text-[11px] text-muted-foreground">如 API、token 作为独立词条</p>
          </div>
          <Switch bind:checked={form.keepLatin} aria-label="保留拉丁词" />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">跳过单字词</p>
            <p class="text-[11px] text-muted-foreground">只统计词表中的多字词（字表仍单独产出）</p>
          </div>
          <Switch bind:checked={form.skipSingleChar} aria-label="跳过单字词" />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">产出分域表</p>
            <p class="text-[11px] text-muted-foreground">关闭后只产出全库表，体积更小</p>
          </div>
          <Switch
            checked={!form.skipDomainTables}
            onCheckedChange={(checked) => (form.skipDomainTables = !checked)}
            aria-label="产出分域表"
          />
        </div>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">写出可读 TSV</p>
            <p class="text-[11px] text-muted-foreground">方便直接用 Excel / 文本编辑器查看</p>
          </div>
          <Switch bind:checked={form.writeTsv} aria-label="写出 TSV" />
        </div>
      </div>
    </CardContent>
  </Card>

  <!-- ③ 输出与执行 -->
  <Card>
    <CardHeader>
      <div class="flex items-center gap-2">
        <CardTitle>③ 输出目录并开始统计</CardTitle>
        <Badge variant="outline">start_scan</Badge>
      </div>
      <CardDescription>产物（meta.json / *.vfr / *.tsv）会写到输出目录，划句分析页从这里读取。</CardDescription>
    </CardHeader>
    <CardContent class="flex flex-col gap-3">
      <div class="flex flex-wrap items-center gap-2">
        <Input
          bind:value={form.out}
          placeholder="例如 D:\voctier-data"
          class="min-w-64 flex-1 font-mono text-xs"
          aria-label="输出目录"
        />
        <Button variant="outline" size="sm" onclick={() => chooseDirectory('out')}>浏览…</Button>
        <Button variant="outline" size="sm" disabled={!form.out.trim()} onclick={refreshDataset}>
          检查产物
        </Button>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button disabled={!canStart} onclick={runScan}>
          {scanning ? '统计中…' : '开始统计'}
        </Button>
        <Button variant="outline" disabled={!scanning} onclick={stopScan}>取消</Button>
        {#if isTauri() && form.out.trim()}
          <Button variant="ghost" size="sm" onclick={() => void openExternal(form.out.trim())}>
            打开输出目录
          </Button>
        {/if}
        {#if savedHint}
          <span class="text-[11px] text-muted-foreground">{savedHint}</span>
        {/if}
      </div>

      {#if scanError}
        <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          统计失败：{scanError}
        </p>
      {/if}

      {#if scanning || progress.bytesTotal > 0}
        <div class="flex flex-col gap-2 rounded-lg border border-border p-3">
          <div class="flex items-center justify-between text-xs">
            <span class="font-medium">
              {progress.phase || '准备中'}
              {#if progress.domain}· 当前分域 {progress.domain}{/if}
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
              已处理 <b class="tabular-nums text-foreground">{formatBytes(progress.bytesDone)}</b>
              / {formatBytes(progress.bytesTotal)}
            </span>
            <span>
              单元 <b class="tabular-nums text-foreground">{formatInt(progress.unitsDone)}</b>
              / {formatInt(progress.unitsTotal)}
            </span>
          </div>
        </div>
      {/if}

      {#if planEvent}
        <div class="rounded-lg border border-border p-3 text-xs">
          <p class="mb-1 font-medium">扫描计划</p>
          <p class="text-muted-foreground">
            共 {formatInt(planEvent.files)} 个文件 / {formatBytes(planEvent.bytes)}，分域：
            {planEvent.domains.map((d) => `${d.name}(${formatInt(d.files)})`).join('、') || '—'}
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
          <p class="text-xs font-medium">已写出的表</p>
          <ul class="flex flex-wrap gap-2">
            {#each tableEvents as table, index (table.table + index)}
              <li class="rounded-md border border-border px-2 py-1 text-[11px]">
                <span class="font-mono">{table.table}</span>
                <span class="ml-2 text-muted-foreground">
                  {formatInt(table.entries)} 词条 · {formatInt(table.total_tokens)} token
                </span>
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if logs.length > 0}
        <div class="flex flex-col gap-1">
          <p class="text-xs font-medium">日志</p>
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
    </CardContent>
  </Card>

  <!-- 结果 -->
  {#if result}
    <Card class="border-primary/30">
      <CardHeader>
        <div class="flex items-center gap-2">
          <CardTitle>统计完成</CardTitle>
          <Badge variant="success">scan:done</Badge>
        </div>
        <CardDescription>
          生成于 {formatTimestamp(result.generated_at)} · 耗时 {formatDuration(result.elapsed_ms)} ·
          工具版本 {result.tool_version} · schema v{result.schema_version}
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
          <span>分词引擎 <b class="text-foreground">{result.tokenizer.engine} {result.tokenizer.version}</b></span>
          <span>HMM <b class="text-foreground">{result.tokenizer.hmm ? '开' : '关'}</b></span>
          <span>数字词 <b class="text-foreground">{result.tokenizer.keep_digit ? '保留' : '丢弃'}</b></span>
          <span>拉丁词 <b class="text-foreground">{result.tokenizer.keep_latin ? '保留' : '丢弃'}</b></span>
          <span>跳过单字 <b class="text-foreground">{result.tokenizer.skip_single_char ? '是' : '否'}</b></span>
          <span class="selectable truncate">词典 {result.tokenizer.dict}</span>
          {#if result.tokenizer.user_dict}
            <span class="selectable truncate">用户词典 {result.tokenizer.user_dict}</span>
          {/if}
        </div>

        <div class="flex flex-col gap-4">
          {#each resultTables as table (table.path)}
            <div class="flex flex-col gap-2">
              <div class="flex flex-wrap items-center gap-2">
                <p class="font-mono text-xs font-medium">{table.path}</p>
                <Badge variant="secondary">{table.kind === 'char' ? '字表' : '词表'}</Badge>
                <span class="text-[11px] text-muted-foreground">
                  {formatInt(table.entries)} 词条 · {formatInt(table.total_tokens)} token · .vfr {formatBytes(table.vfr_bytes)}
                </span>
              </div>
              <TierStatsTable stats={table.tier_stats} tiers={table.tiers} />
            </div>
          {/each}
        </div>

        <div class="rounded-lg border border-dashed border-primary/40 bg-primary/5 px-3 py-2 text-xs">
          统计完成 ✅ 现在可以去「<b>划句分析</b>」页粘贴文本，查看每个词的频率分组；
          也可以在「<b>排行榜</b>」页浏览完整排行。
        </div>
      </CardContent>
    </Card>
  {/if}

  {#if loading}
    <p class="text-xs text-muted-foreground">正在读取已有设置…</p>
  {/if}
</div>
