<script lang="ts">
  /**
   * 表管理 —— 两件事：
   *
   *   A. **表清单与开关**：产物目录里是「2 种类型 × 8 个作用域 = 16 张表」，
   *      全部登记在 `meta.tables` 里（不需要新的后端命令）。每张表一个开关，
   *      写进 `Settings.enabledTables`，Rust 侧据此过滤划句分析里的分域排名。
   *      空数组或 null = 全部启用；全库表始终用于「单个词/字的总体频率」查询，
   *      关掉它只影响分域对比与排行榜。
   *
   *   B. **分组自定义**：三种口径（排名 / 覆盖率 / 词条数等分）各自给六组定阈值，
   *      第 7 组自动是「以上全部」。排名与等分是纯前端能算的；覆盖率要用
   *      `tier_curve` 拿到的对数采样曲线反解成排名（`format.ts::rankForCoverage`，
   *      与 Rust 侧 `vocfreq_core::query::rank_for_coverage` 等价）。
   *
   * 所有保存都走 `$lib/tiers.svelte.ts` 的 `updateSettings`：它把改动并进当前设置，
   * 所以这里不会覆盖掉设置页里的其它字段，设置页也不会覆盖这里改的值。
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
  import { Switch } from '$lib/components/ui/switch';
  import TierLegend from '$lib/components/analysis/TierLegend.svelte';
  import { datasetStatus, isTauri, openDataset } from '$lib/api/bridge';
  import {
    defaultCoverageTargets,
    findTable,
    formatBytes,
    formatInt,
    formatRatio,
    rankForCoverage,
    tierIndexFromBounds,
  } from '$lib/format';
  import { NAVIGATE_EVENT } from '$lib/navigation';
  import {
    activeBoundsInfo,
    appSettings,
    ensureCurve,
    tierNamesOf,
    updateSettings,
  } from '$lib/tiers.svelte';
  import { paletteForTierName, swatchStyle } from '$lib/tier-colors';
  import { isDark } from '$lib/use-dark.svelte';
  import { cn } from '$lib/utils';
  import {
    TIER_BOUND_COUNT,
    TIER_COUNT,
    type Meta,
    type TableMeta,
    type TierCurve,
    type TierMethod,
  } from '$lib/types';

  const METHOD_LABELS: Record<TierMethod, string> = {
    rank: '按排名',
    coverage: '按覆盖率',
    even: '按词条数等分',
  };

  const METHOD_NOTES: Record<TierMethod, { what: string; when: string }> = {
    rank: {
      what: '直接给七组定排名上界：第 1 组 = 排名 1..N₁，第 2 组 = N₁+1..N₂ …… 第 7 组 = 最后一个上界以上全部。',
      when: '最直观，跨语料库也能比较；但看不出每组实际盖住多少正文——同样是「前 100 名」，在不同语料库里覆盖的比例可以差很多。',
    },
    coverage: {
      what: '给定「每组累计覆盖正文的百分比」，程序用覆盖率曲线反解出对应的排名上界（在 log10(rank) 上插值）。',
      when: '跨语料库最可比（不用关心词条总数），适合「前 50% 正文由多少词覆盖」这类问题；代价是各组的大小差异会很大。',
    },
    even: {
      what: '简单粗暴地按词条数七等分：每组词条数几乎相同。',
      when: '想让每组样本量接近（抽样、统计检验）时用；与「常用 / 生僻」的直觉完全无关。',
    },
  };

  type PreviewRow = {
    index: number;
    name: string;
    range: string;
    entries: number;
    coverage: number;
    cumulative: number;
    source: string;
  };

  type Preview = {
    kind: 'word' | 'char';
    rows: PreviewRow[];
    bounds: number[];
    warning: string;
  };

  type DomainRow = { domain: string; word?: TableMeta; char?: TableMeta };

  const dark = $derived(isDark());

  let meta = $state<Meta | null>(null);
  let statusDir = $state<string | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let datasetLoaded = $state(false);
  let datasetLoadError = $state('');

  let notice = $state('');
  let noticeTone = $state<'info' | 'error' | 'success'>('info');

  /** 编辑中的阈值（排名 / 等分模式用），切方法时由 effect 从生效值填入 */
  let wordBounds = $state<number[]>([]);
  let charBounds = $state<number[]>([]);
  /** 编辑中的覆盖率目标（0..1），覆盖率模式用 */
  let coverage = $state<number[]>([]);

  /** 覆盖率曲线缓存（全库词表 / 字表） */
  let curves = $state<{ word: TierCurve | null; char: TierCurve | null }>({ word: null, char: null });
  let curveBusy = $state(false);
  let curveError = $state('');

  let saving = $state(false);

  // ---------------------------------------------------------------- 派生

  const names = $derived(tierNamesOf(meta));

  const domains = $derived.by(() => {
    const fromMeta = (meta?.domains ?? []).map((domain) => domain.name);
    const fromTables = (meta?.tables ?? [])
      .filter((table) => table.path.startsWith('domains/'))
      .map((table) => table.path.split('/')[1]);
    return [...new Set([...fromMeta, ...fromTables])];
  });

  const fullTables = $derived((meta?.tables ?? []).filter((table) => table.path.startsWith('full/')));

  const domainRows = $derived.by(() => {
    if (!meta) return [] as DomainRow[];
    const list: DomainRow[] = [];
    for (const name of domains) {
      const word = meta.tables.find((table) => table.path === `domains/${name}/word`);
      const char = meta.tables.find((table) => table.path === `domains/${name}/char`);
      if (word || char) list.push({ domain: name, word, char });
    }
    return list;
  });

  const tableCount = $derived((meta?.tables ?? []).length);

  const enabledCount = $derived.by(() => {
    if (!meta) return 0;
    const enabled = appSettings.value.enabledTables;
    if (!enabled || enabled.length === 0) return meta.tables.length;
    return meta.tables.filter((table) => enabled.includes(table.path)).length;
  });

  const method = $derived<TierMethod>(
    appSettings.value.tierMethod === 'coverage' || appSettings.value.tierMethod === 'even'
      ? appSettings.value.tierMethod
      : 'rank'
  );

  /** 覆盖率目标：用户没填时用 meta 默认分档的累计覆盖率 */
  const effectiveCoverage = $derived(
    appSettings.value.tierCoverage ??
      (meta ? defaultCoverageTargets(meta, 'word') : [0.35, 0.5, 0.62, 0.72, 0.82, 0.92])
  );

  /** 实时预览：用当前生效阈值把一张表的 token 总量摊到七组里 */
  const previews = $derived.by(() => {
    if (!meta) return [] as Preview[];
    const out: Preview[] = [];
    for (const kind of ['word', 'char'] as const) {
      const table = findTable(meta.tables, kind);
      const info = activeBoundsInfo(kind, meta);
      const bounds = info.bounds;
      const total = table?.total_tokens ?? 0;
      const curve = curves[kind];
      // 曲线最后一点的覆盖率必须是 1，按点累加才等于「累计覆盖率」
      const complete = curve !== null && Math.abs((curve.points[curve.points.length - 1]?.[1] ?? 0) - 1) < 1e-6;
      const grouped = groupTokens(table, bounds, complete ? curve : null, total);

      let cumulative = 0;
      const rows: PreviewRow[] = [];
      // 预览用的「有效上界」：超过表的总词条数的阈值夹到词条总数，否则像演示数据
      // （57 条词、阈值 100/1000/5000…）会把 7 组全压进第 1 组，预览看不出任何信息。
      // 注意：真正的分组判定（token 着色 / 排行榜）只用 bounds，不受这个夹取影响。
      const entries = table?.entries ?? 0;
      const capped = bounds.map((value) => Math.min(value, entries));
      const groupedPreview = complete ? grouped : groupTokens(table, capped, null, total);
      names.slice(0, TIER_COUNT).forEach((name, index) => {
        const group = groupedPreview.bands[index] ?? { entries: 0, tokens: 0 };
        const share = total > 0 ? groupedPreview.bands[index].tokens / total : 0;
        cumulative += share;
        rows.push({
          index,
          name,
          range:
            index >= TIER_BOUND_COUNT
              ? `> ${(bounds[bounds.length - 1] ?? 0).toLocaleString('zh-CN')}`
              : `≤ ${(bounds[index] ?? 0).toLocaleString('zh-CN')}`,
          entries: group.entries,
          coverage: share,
          cumulative: Math.min(1, cumulative),
          source: complete ? '曲线' : table?.tier_stats[index] ? '按分档比例估算' : '—',
        });
      });

      const skipped = bounds.filter((value) => value > entries).length;
      const capNote =
        skipped > 0
          ? `有 ${skipped} 个阈值大于这张表的词条数（${formatInt(entries)} 条），预览里已夹到词条总数；真实分组判定不受影响。`
          : '';
      out.push({
        kind,
        rows,
        bounds,
        warning: [info.warning, capNote].filter(Boolean).join(' '),
      });
    }
    return out;
  });

  /** 反解预览：覆盖率目标 → 排名上界 */
  const solved = $derived.by(() =>
    (['word', 'char'] as const).map((kind) => ({
      kind,
      ranks: coverage.map((target) =>
        curves[kind] ? rankForCoverage(curves[kind]!.points, target) : null
      ),
    }))
  );

  // ---------------------------------------------------------------- 生命周期

  $effect(() => {
    void bootstrap();
  });

  // 切到「按排名 / 等分」时，把编辑框填成当前生效值
  $effect(() => {
    const current = meta;
    if (!current || method === 'coverage') return;
    wordBounds = activeBoundsInfo('word', current).bounds;
    charBounds = activeBoundsInfo('char', current).bounds;
  });

  // 切到「按覆盖率」时，把编辑框填成设置里的目标（没有就用 meta 默认）
  $effect(() => {
    if (method !== 'coverage') return;
    coverage = [...effectiveCoverage];
  });

  // 覆盖率模式需要曲线；进入该模式且已装载产物时拉一次（Rust 侧按产物缓存）
  $effect(() => {
    if (method !== 'coverage') return;
    if (!statusDir) return;
    // 依赖：只有 curves 为空时才真正发请求；已经拿到就直接用
    void autoFetchCurves();
  });

  async function bootstrap() {
    loading = true;
    loadError = '';
    const status = await datasetStatus(appSettings.value.dataDir ?? appSettings.value.corpusDir);
    if (!status.ok) {
      loading = false;
      loadError = status.error;
      return;
    }
    if (!status.data.meta) {
      loading = false;
      meta = null;
      return;
    }
    statusDir = status.data.dir;
    meta = status.data.meta;
    datasetLoaded = false;
    datasetLoadError = '';
    const opened = await openDataset(status.data.dir);
    if (opened.ok) {
      datasetLoaded = true;
      meta = opened.data;
    } else {
      datasetLoadError = opened.error;
    }
    loading = false;
  }

  async function autoFetchCurves() {
    if (curves.word && curves.char) return;
    await fetchCurves();
  }

  async function fetchCurves() {
    const dir = statusDir;
    curveBusy = true;
    curveError = '';
    const [word, char] = await Promise.all([
      ensureCurve('full/word', dir, 600),
      ensureCurve('full/char', dir, 600),
    ]);
    if (word.ok) curves.word = word.data;
    if (char.ok) curves.char = char.data;
    curveBusy = false;
    if (!word.ok) curveError = word.error;
    else if (!char.ok) curveError = char.error;
  }

  // ---------------------------------------------------------------- 表开关

  function showNotice(message: string, tone: 'info' | 'error' | 'success' = 'info') {
    notice = message;
    noticeTone = tone;
    window.setTimeout(() => {
      if (notice === message) notice = '';
    }, 4000);
  }

  function tableEnabled(path: string): boolean {
    const enabled = appSettings.value.enabledTables;
    if (!enabled || enabled.length === 0) return true;
    return enabled.includes(path);
  }

  function toggleTable(path: string, checked: boolean) {
    if (!meta) return;
    const current = appSettings.value.enabledTables;
    const list =
      !current || current.length === 0 ? meta.tables.map((table) => table.path) : [...current];
    const next = checked
      ? [...new Set([...list, path])]
      : list.filter((item) => item !== path);
    void persist(
      { enabledTables: next },
      `已${checked ? '启用' : '停用'}「${path}」。划句分析的分域对比与排行榜会随之变化。`
    );
  }

  function applyPreset(kind: 'all' | 'none' | 'full' | 'word' | 'char') {
    if (!meta) return;
    const all = meta.tables.map((table) => table.path);
    let next: string[];
    let message: string;
    if (kind === 'all') {
      next = all;
      message = `已全选 ${all.length} 张表。`;
    } else if (kind === 'none') {
      next = [];
      message = '已全不选：空列表在后端等同于「全部启用」，相当于重置。';
    } else if (kind === 'full') {
      next = all.filter((path) => path.startsWith('full/'));
      message = `只留全库表（${next.length} 张）。`;
    } else if (kind === 'word') {
      next = all.filter((path) => path.endsWith('/word'));
      message = `只留词表（${next.length} 张）。`;
    } else {
      next = all.filter((path) => path.endsWith('/char'));
      message = `只留字表（${next.length} 张）。`;
    }
    void persist({ enabledTables: next }, `${message}划句分析的分域对比与排行榜会随之变化。`);
  }

  function resetEnabled() {
    void persist({ enabledTables: null }, '已重置为「全部启用」（enabledTables = null）。');
  }

  // ---------------------------------------------------------------- 分组设置

  async function persist(part: Parameters<typeof updateSettings>[0], message: string) {
    saving = true;
    const res = await updateSettings(part);
    saving = false;
    if (!res.ok) {
      showNotice(`保存失败：${res.error}`, 'error');
      return;
    }
    showNotice(message, 'success');
  }

  function switchMethod(next: TierMethod) {
    if (next === method) return;
    void persist({ tierMethod: next }, `分组方法已切换为「${METHOD_LABELS[next]}」。`);
  }

  function updateBound(kind: 'word' | 'char', index: number, raw: string) {
    const value = Math.max(0, Math.round(Number(raw) || 0));
    if (kind === 'word') {
      const next = [...wordBounds];
      next[index] = value;
      wordBounds = next;
      void persist({ tierWordBounds: next }, '词表阈值已保存。');
    } else {
      const next = [...charBounds];
      next[index] = value;
      charBounds = next;
      void persist({ tierCharBounds: next }, '字表阈值已保存。');
    }
  }

  function updateCoverage(index: number, raw: string) {
    const value = Math.min(1, Math.max(0, (Number(raw) || 0) / 100));
    const next = [...coverage];
    next[index] = value;
    coverage = next;
    void persist({ tierCoverage: next }, '覆盖率目标已保存。');
  }

  function resetBounds(kind: 'word' | 'char' | 'coverage') {
    if (kind === 'coverage') {
      void persist({ tierCoverage: null }, '已恢复默认覆盖率目标（取自 meta 的累计覆盖率）。');
      return;
    }
    void persist(
      kind === 'word' ? { tierWordBounds: null } : { tierCharBounds: null },
      `${kind === 'word' ? '词表' : '字表'}阈值已恢复为 meta 默认值。`
    );
  }

  async function solveFromCurve() {
    await fetchCurves();
    const word = curves.word;
    const char = curves.char;
    if (!word || !char) {
      showNotice(`拿不到覆盖率曲线，无法反解${curveError ? `：${curveError}` : '。'}`, 'error');
      return;
    }
    const targets = [...coverage];
    const wordRanks = targets.map((target) => rankForCoverage(word.points, target) ?? 0);
    const charRanks = targets.map((target) => rankForCoverage(char.points, target) ?? 0);
    wordBounds = wordRanks;
    charBounds = charRanks;
    void persist(
      {
        tierWordBounds: wordRanks,
        tierCharBounds: charRanks,
        tierMethod: 'rank',
        tierCoverage: targets,
      },
      `已用覆盖率曲线反解成排名阈值（词表：${wordRanks.map(formatInt).join(' / ')}），并切回「按排名」。`
    );
  }

  // ---------------------------------------------------------------- 辅助

  /**
   * 把一张表的 token 总量按分组上界摊到七组里。
   *
   *   - 有完整曲线（最后一点覆盖率 = 1）时：按曲线点累加，累计覆盖率是精确的；
   *     词条数只能按「排名区间里有多少个采样点」近似。
   *   - 没有曲线时：用 meta 的 `tier_stats`（每组的词条数 / token 数）按上界比例估算，
   *     再按全表 token 总量归一化，保证七组加总 = 100%。
   */
  function groupTokens(
    table: TableMeta | undefined,
    bounds: number[],
    curve: TierCurve | null,
    total: number
  ): { bands: { entries: number; tokens: number }[]; truncated: boolean } {
    const bands = Array.from({ length: TIER_COUNT }, () => ({ entries: 0, tokens: 0 }));
    if (!table) return { bands, truncated: false };

    if (curve && curve.points.length >= 2) {
      for (let i = 0; i < curve.points.length; i += 1) {
        const [rank, coverage] = curve.points[i];
        const prev = i === 0 ? 0 : curve.points[i - 1][1];
        const start = i === 0 ? 1 : curve.points[i - 1][0] + 1;
        const index = tierIndexFromBounds(rank, bounds);
        if (index === null) continue;
        bands[index].entries += Math.max(0, rank - start + 1);
        bands[index].tokens += Math.max(0, coverage - prev) * total;
      }
      return { bands, truncated: false };
    }

    const stats = table.tier_stats;
    const densityAt = (groupIndex: number): number => {
      const stat = stats[groupIndex];
      if (!stat || stat.entries <= 0) return 0;
      return stat.tokens / stat.entries;
    };
    // 上界夹到表的总词条数：像演示数据这种只有几十条词、阈值却是 500/3000 的表，
    // 不夹的话「组内词条数」会算出 500 / 2500 这种不可能的值。
    const upper = [...bounds, table.entries].map((value) => Math.min(value, table.entries));
    let lower = 0;
    let estimated = 0;
    for (let i = 0; i < TIER_COUNT; i += 1) {
      const hi = Math.max(lower, upper[i] ?? table.entries);
      const entries = Math.max(0, hi - lower);
      const avg = (densityAt(i) + densityAt(Math.max(0, i - 1))) / 2;
      const tokens = entries * avg;
      bands[i].entries = entries;
      bands[i].tokens = tokens;
      estimated += tokens;
      lower = hi;
    }
    if (estimated > 0 && total > 0) {
      const scale = total / estimated;
      for (const band of bands) band.tokens *= scale;
    }
    return { bands, truncated: lower < table.entries };
  }

  function rowLabel(path: string): string {
    const kind = path.endsWith('/char') ? '字表' : '词表';
    if (path.startsWith('full/')) return `全库 · ${kind}`;
    return `${path.split('/')[1]} · ${kind}`;
  }

  /** 该表最后一组（最少见的那一档）的覆盖率 */
  function lastTierCoverage(table: TableMeta): number {
    const stat = table.tier_stats[table.tier_stats.length - 1];
    return stat?.coverage ?? 0;
  }

  function goWordFreq() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'wordfreq' }));
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      浏览器预览模式：表清单与开关读写的是内置演示数据（仅内存），分组设置同样只保存在内存里。
    </div>
  {/if}

  {#if notice}
    <div
      class={cn(
        'rounded-lg border px-3 py-2 text-xs',
        noticeTone === 'error'
          ? 'border-destructive/30 bg-destructive/5 text-destructive'
          : noticeTone === 'success'
            ? 'border-emerald-500/30 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400'
            : 'border-primary/30 bg-primary/5 text-primary'
      )}
      data-testid="tables-notice"
    >
      {notice}
    </div>
  {/if}

  {#if loading}
    <Card>
      <CardContent class="py-8 text-center text-xs text-muted-foreground">
        正在读取产物目录与设置…
      </CardContent>
    </Card>
  {:else if loadError}
    <Card class="border-destructive/30">
      <CardContent class="py-6 text-xs text-destructive">读取数据集状态失败：{loadError}</CardContent>
    </Card>
  {:else if !meta}
    <Card class="border-dashed">
      <CardHeader>
        <CardTitle>还没有可管理的表</CardTitle>
        <CardDescription>
          表清单来自 meta.json（`meta.tables`）。请先在「生成词频表」页产出一份结果。
        </CardDescription>
      </CardHeader>
      <CardContent class="flex gap-2">
        <Button onclick={goWordFreq}>去生成词频表</Button>
        <Button variant="outline" onclick={() => void bootstrap()}>重新检查</Button>
      </CardContent>
    </Card>
  {:else}
    {#if !datasetLoaded}
      <div class="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
        <p class="font-medium">产物目录尚未装载到后端</p>
        <p class="mt-0.5">
          已找到 meta.json（{statusDir}），但 `open_dataset` 未成功{datasetLoadError ? `：${datasetLoadError}` : '。'}。
          表清单仍可管理，但 `tier_curve` 可能拿不到曲线。
        </p>
      </div>
    {/if}

    <!-- ============================ A. 表清单与开关 ============================ -->
    <Card data-testid="table-list-card">
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>表清单与开关</CardTitle>
          <Badge variant="secondary" data-testid="table-count">{formatInt(tableCount)} 张表</Badge>
          <Badge variant="outline">已启用 {formatInt(enabledCount)} 张</Badge>
          {#if saving}<Badge variant="outline">保存中…</Badge>{/if}
        </div>
        <CardDescription>
          关掉某张表 = 它不参与「划句分析」的分域对比与「排行榜」。全库表（full/word、full/char）始终用于
          单个词 / 字的总体频率查询——关掉它只会让它不参与分域对比，不会让正文整片变成「未收录」。
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div class="flex flex-wrap items-center gap-1.5">
          <span class="mr-1 text-xs font-medium">快捷选择</span>
          <Button variant="outline" size="sm" onclick={() => applyPreset('all')}>全选</Button>
          <Button variant="outline" size="sm" onclick={() => applyPreset('none')}>全不选</Button>
          <Button variant="outline" size="sm" onclick={() => applyPreset('full')}>只留全库</Button>
          <Button variant="outline" size="sm" onclick={() => applyPreset('word')}>只留词表</Button>
          <Button variant="outline" size="sm" onclick={() => applyPreset('char')}>只留字表</Button>
          <Button variant="ghost" size="sm" onclick={resetEnabled}>
            重置为全部启用
          </Button>
        </div>

        <p class="rounded-md bg-surface-muted/60 px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
          注意：开关变化后，<b>划句分析的分域对比与排行榜会随之变化</b>（后端按 `enabledTables` 过滤分域排名）。
          全库表的总体频率查询不受影响。
        </p>

        <div class="scrollbar-thin max-h-[30rem] overflow-auto rounded-lg border border-border">
          <table class="w-full border-collapse text-xs">
            <thead class="sticky top-0 z-10 bg-surface-muted text-muted-foreground">
              <tr>
                <th class="px-3 py-2 text-left font-medium">表</th>
                <th class="px-3 py-2 text-right font-medium">词条数</th>
                <th class="px-3 py-2 text-right font-medium">总 token</th>
                <th class="px-3 py-2 text-right font-medium">.vfr 体积</th>
                <th class="px-3 py-2 text-right font-medium">最后一组覆盖率</th>
                <th class="px-3 py-2 text-right font-medium">启用</th>
              </tr>
            </thead>
            <tbody>
              {#each fullTables as table (table.path)}
                <tr class="border-t border-border/70" data-table-row={table.path}>
                  <td class="px-3 py-2">
                    <span class="font-medium">{rowLabel(table.path)}</span>
                    <span class="ml-2 font-mono text-[11px] text-muted-foreground">{table.path}</span>
                  </td>
                  <td class="px-3 py-2 text-right tabular-nums">{formatInt(table.entries)}</td>
                  <td class="px-3 py-2 text-right tabular-nums">{formatInt(table.total_tokens)}</td>
                  <td class="px-3 py-2 text-right tabular-nums">{formatBytes(table.vfr_bytes)}</td>
                  <td class="px-3 py-2 text-right tabular-nums">
                    {formatRatio(lastTierCoverage(table), { digits: 3 })}
                  </td>
                  <td class="px-3 py-2">
                    <span class="flex justify-end">
                      <Switch
                        checked={tableEnabled(table.path)}
                        onCheckedChange={(checked) => toggleTable(table.path, checked)}
                        aria-label={`启用 ${table.path}`}
                      />
                    </span>
                  </td>
                </tr>
              {/each}

              <tr class="border-t-2 border-border bg-surface-muted/40">
                <td colspan="6" class="px-3 py-1.5 text-[11px] font-medium text-muted-foreground">
                  各分域（domains/）
                </td>
              </tr>

              {#each domainRows as row (row.domain)}
                {#each [row.word, row.char].filter((item) => item !== undefined) as table (table.path)}
                  <tr class="border-t border-border/70" data-table-row={table.path}>
                    <td class="px-3 py-2">
                      <span class="font-medium">{rowLabel(table.path)}</span>
                      <span class="ml-2 font-mono text-[11px] text-muted-foreground">{table.path}</span>
                    </td>
                    <td class="px-3 py-2 text-right tabular-nums">{formatInt(table.entries)}</td>
                    <td class="px-3 py-2 text-right tabular-nums">{formatInt(table.total_tokens)}</td>
                    <td class="px-3 py-2 text-right tabular-nums">{formatBytes(table.vfr_bytes)}</td>
                    <td class="px-3 py-2 text-right tabular-nums">
                      {formatRatio(lastTierCoverage(table), { digits: 3 })}
                    </td>
                    <td class="px-3 py-2">
                      <span class="flex justify-end">
                        <Switch
                          checked={tableEnabled(table.path)}
                          onCheckedChange={(checked) => toggleTable(table.path, checked)}
                          aria-label={`启用 ${table.path}`}
                        />
                      </span>
                    </td>
                  </tr>
                {/each}
              {/each}
            </tbody>
          </table>
        </div>

        {#if tableCount !== 16}
          <p class="text-[11px] text-muted-foreground">
            提示：完整产物是 2 种类型 × 8 个作用域 = 16 张表（全库 + 7 个分域），当前 meta.json 里登记了
            {tableCount} 张——说明扫描时用了 `--skip-domain-tables` 或只统计了部分分域。
          </p>
        {/if}
      </CardContent>
    </Card>

    <!-- ============================ B. 分组自定义 ============================ -->
    <Card data-testid="tier-config-card">
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>分组自定义</CardTitle>
          <Badge variant="outline">七组</Badge>
          <Badge variant="secondary">{METHOD_LABELS[method]}</Badge>
        </div>
        <CardDescription>
          七组的名字固定（{names.join(' / ')}），这里定的是每组的<b>排名上界</b>：第 1..6 组各有一个上界，
          第 7 组自动是「以上全部」。
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-4">
        <!-- 方法切换 -->
        <div class="flex flex-col gap-2">
          <span class="text-xs font-medium">分组方法</span>
          <div class="flex flex-wrap gap-1.5">
            {#each ['rank', 'coverage', 'even'] as TierMethod[] as item (item)}
              <button
                type="button"
                data-testid={`method-${item}`}
                aria-pressed={method === item}
                class={cn(
                  'rounded-md border px-3 py-1.5 text-xs transition-colors',
                  method === item
                    ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                    : 'border-border hover:bg-accent'
                )}
                onclick={() => switchMethod(item)}
              >
                {METHOD_LABELS[item]}
              </button>
            {/each}
          </div>
          <div class="rounded-lg border border-border bg-surface-muted/40 px-3 py-2 text-[11px] leading-relaxed">
            <p><span class="font-medium">在做什么：</span>{METHOD_NOTES[method].what}</p>
            <p class="mt-1"><span class="font-medium">什么时候用：</span>{METHOD_NOTES[method].when}</p>
          </div>
        </div>

        <Separator />

        <!-- 编辑区 -->
        {#if method === 'coverage'}
          <div class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center gap-2">
              <span class="text-xs font-medium">累计覆盖率目标（%）</span>
              <span class="text-[11px] text-muted-foreground">
                第 7 组固定是「以上全部」，所以只填 6 个，且必须严格递增
              </span>
              <span class="ml-auto flex gap-1">
                <Button variant="outline" size="sm" onclick={() => void solveFromCurve()}>
                  一键用当前覆盖率反解成排名阈值
                </Button>
                <Button variant="ghost" size="sm" onclick={() => resetBounds('coverage')}>恢复默认</Button>
              </span>
            </div>

            <div class="flex flex-wrap gap-2">
              {#each coverage as value, index (index)}
                <label class="flex flex-col gap-1">
                  <span class="text-[11px] text-muted-foreground">第 {index + 1} 组</span>
                  <input
                    type="number"
                    min="0"
                    max="100"
                    step="0.1"
                    class="w-24 rounded border border-input bg-surface px-2 py-1 text-right text-xs tabular-nums"
                    data-testid={`coverage-${index}`}
                    value={(value * 100).toFixed(2)}
                    onchange={(event) => updateCoverage(index, event.currentTarget.value)}
                    aria-label={`第 ${index + 1} 组累计覆盖率`}
                  />
                </label>
              {/each}
            </div>

            {#if curveError}
              <p class="text-[11px] text-destructive">曲线加载失败：{curveError}</p>
            {:else if curveBusy}
              <p class="text-[11px] text-muted-foreground">正在读取覆盖率曲线…</p>
            {:else}
              <p class="text-[11px] text-muted-foreground">
                曲线：{curves.word ? `${formatInt(curves.word.points.length)} 点` : '—'}（词表）·
                {curves.char ? `${formatInt(curves.char.points.length)} 点` : '—'}（字表）·
                Rust 侧按产物缓存，重复进入本页不会重算。
              </p>
            {/if}

            <div class="grid grid-cols-1 gap-3 lg:grid-cols-2">
              {#each solved as item (item.kind)}
                <div class="rounded-lg border border-border p-3">
                  <p class="mb-2 text-xs font-medium">
                    {item.kind === 'word' ? '词表反解结果' : '字表反解结果'}
                  </p>
                  <div class="flex flex-wrap gap-x-4 gap-y-1 text-[11px]">
                    {#each item.ranks as rank, index (index)}
                      <span>
                        <span class="text-muted-foreground">第 {index + 1} 组 ≤</span>
                        <span class="ml-1 tabular-nums font-medium" data-testid={`solved-${item.kind}-${index}`}>
                          {rank === null ? '—' : formatInt(rank)}
                        </span>
                      </span>
                    {/each}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {:else}
          <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
            {#each ['word', 'char'] as const as kind (kind)}
              {@const bounds = kind === 'word' ? wordBounds : charBounds}
              <div class="flex flex-col gap-2 rounded-lg border border-border p-3">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="text-xs font-medium">{kind === 'word' ? '词表阈值' : '字表阈值'}</span>
                  <span class="text-[11px] text-muted-foreground">6 个排名上界</span>
                  <Button variant="ghost" size="sm" class="ml-auto" onclick={() => resetBounds(kind)}>
                    恢复默认
                  </Button>
                </div>
                <div class="flex flex-wrap gap-2">
                  {#each Array.from({ length: TIER_BOUND_COUNT }, (_, i) => i) as index (index)}
                    <label class="flex flex-col gap-1">
                      <span class="text-[11px] text-muted-foreground">第 {index + 1} 组</span>
                      <input
                        type="number"
                        min="0"
                        step="1"
                        class="w-24 rounded border border-input bg-surface px-2 py-1 text-right text-xs tabular-nums"
                        data-testid={`${kind}-bound-${index}`}
                        value={bounds[index] ?? 0}
                        onchange={(event) => updateBound(kind, index, event.currentTarget.value)}
                        aria-label={`${kind === 'word' ? '词表' : '字表'}第 ${index + 1} 组排名上界`}
                      />
                    </label>
                  {/each}
                </div>
                <p class="text-[11px] text-muted-foreground">
                  第 7 组 = 「以上全部」（排在
                  {(bounds[bounds.length - 1] ?? 0).toLocaleString('zh-CN')} 名之后的所有词条）
                </p>
              </div>
            {/each}
          </div>
        {/if}

        <Separator />

        <!-- 实时预览 -->
        <div class="flex flex-col gap-2">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-xs font-medium">实时预览</span>
            <span class="text-[11px] text-muted-foreground">
              改上面的数字会立刻重算；「组内词条数」在无曲线时是估算值，累计覆盖率取自曲线时是精确的
            </span>
          </div>

          {#each previews as preview (preview.kind)}
            <div class="flex flex-col gap-2">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-xs font-medium">{preview.kind === 'word' ? '词表' : '字表'}预览</span>
                {#if preview.warning}
                  <span
                    class="rounded border border-amber-500/40 px-1.5 py-0.5 text-[11px] text-amber-600 dark:text-amber-400"
                    data-testid={`warning-${preview.kind}`}
                  >
                    {preview.warning}
                  </span>
                {/if}
              </div>
              <div class="scrollbar-thin overflow-auto rounded-lg border border-border">
                <table class="w-full border-collapse text-xs" data-testid={`preview-${preview.kind}`}>
                  <thead class="bg-surface-muted/60 text-muted-foreground">
                    <tr>
                      <th class="px-3 py-2 text-left font-medium">组名</th>
                      <th class="px-3 py-2 text-right font-medium">阈值</th>
                      <th class="px-3 py-2 text-right font-medium">组内词条数</th>
                      <th class="px-3 py-2 text-right font-medium">本组覆盖率</th>
                      <th class="px-3 py-2 text-right font-medium">累计覆盖率</th>
                      <th class="px-3 py-2 text-right font-medium">来源</th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each preview.rows as row (row.name)}
                      {@const palette = paletteForTierName(row.name)}
                      <tr class="border-t border-border/70" data-preview-row={row.name}>
                        <td class="px-3 py-2">
                          <span class="inline-flex items-center gap-1.5">
                            <span
                              class="inline-block size-3 shrink-0 rounded-[3px] border"
                              style={swatchStyle(palette, dark)}
                            ></span>
                            <span class="font-medium">{row.name}</span>
                          </span>
                        </td>
                        <td class="px-3 py-2 text-right tabular-nums text-muted-foreground">{row.range}</td>
                        <td class="px-3 py-2 text-right tabular-nums">{formatInt(row.entries)}</td>
                        <td class="px-3 py-2 text-right tabular-nums">
                          {formatRatio(row.coverage, { digits: 3 })}
                        </td>
                        <td class="px-3 py-2 text-right tabular-nums font-medium">
                          {formatRatio(row.cumulative, { digits: 2 })}
                        </td>
                        <td class="px-3 py-2 text-right text-[11px] text-muted-foreground">{row.source}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            </div>
          {/each}
        </div>

        <Separator />

        <div class="flex flex-col gap-2">
          <span class="text-xs font-medium">图例效果（词表生效阈值）</span>
          <TierLegend
            names={names}
            bounds={activeBoundsInfo('word', meta).bounds}
            showUnknown={false}
          />
          <p class="text-[11px] text-muted-foreground">
            设置即时生效并已保存：划句分析页的词条着色、悬停浮层与排行榜的分组列都按这份阈值重算。
            三个分组字段全是 null / 'rank' 时，界面与 meta 默认分组完全一致。
          </p>
        </div>
      </CardContent>
    </Card>
  {/if}
</div>
