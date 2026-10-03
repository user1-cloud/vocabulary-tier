<script module lang="ts">
  import type { NavSection } from '$lib/navigation';

  /** 页面内可导航区块：navigation.ts 直接组合它，新增区块只需在这里加一项 + 一个 <SectionCard> */
  export const PAGE_SECTIONS = {
    library: { id: 'library', labelKey: 'tables.library.title' },
    primary: { id: 'primary', labelKey: 'tables.primaryDomain.title' },
    list: { id: 'list', labelKey: 'tables.listTitle' },
    compose: { id: 'compose', labelKey: 'tables.compose.title' },
    tier: { id: 'tier', labelKey: 'tables.tierConfigTitle' },
  } as const satisfies Record<string, NavSection>;
</script>

<script lang="ts">
  /**
   * 表管理 —— 三件事：
   *
   *   A. **频率表清单**：产物里每个**表组**各有一张词频表 + 一张字表，它们**完全平级**
   *      （`full` 只是"所有表组加在一起"的那一个，没有任何特权）。这里只有一张统一清单，
   *      不再分「全库表 / 表组」两个区。
   *   B. **主词频表**：指定哪一张决定"这个词有多常见"。划句分析的着色与分组、
   *      排行榜、分组阈值预览都以它为准，其余表只做对比。粒度是全局的 ——
   *      否则同一句话在两个页面会是两种颜色。
   *   C. **相加**：把若干张表加起来成一张新表。相加在数学上是精确的
   *      （`scan` 本身就是"逐表组扫完再累加"），新表与别的表完全平级：
   *      能当主表、能再被相加、能删掉回收空间。
   *
   * 分组自定义（第四件事）在下面另一张卡片里：四种口径（**前%** / 排名 / 覆盖率 /
   * 词条数等分），默认是**前%**。
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
  import { SectionCard } from '$lib/components/ui/section-card';
  import TierLegend from '$lib/components/analysis/TierLegend.svelte';
  import {
    activeDataset,
    activateLibraryTable,
    activeTableDir,
    composeTables,
    dictList,
    isTauri,
    libraryInfo,
    openExternal,
    setPrimaryScope,
    tableDelete,
    tableList,
  } from '$lib/api/bridge';
  import {
    defaultCoverageTargets,
    findTable,
    formatBytes,
    formatInt,
    formatRatio,
    formatTimestamp,
    formatTopPercent,
    rankForCoverage,
    tierIndexFromBounds,
  } from '$lib/format';  import { NAVIGATE_EVENT } from '$lib/navigation';
  import { formatCount } from '$lib/number-locale';
  import { requestScanPrefill } from '$lib/scan-prefill.svelte';
  import { splitMessage, t } from '$lib/i18n.svelte';
  import type { MessageKey } from '$lib/messages';
  import {
    activeBoundsInfo,
    appSettings,
    ensureCurve,
    primaryScope,
    tierNamesOf,
    updateSettings,
  } from '$lib/tiers.svelte';
  import { paletteForTierKey, swatchStyle, tierKeyAt, tierKeysFrom } from '$lib/tier-colors';
  import { isDark } from '$lib/use-dark.svelte';
  import { cn } from '$lib/utils';
  import {
    defaultTierPct,
    metaScopes,
    tableKey,
    TIER_BOUND_COUNT,
    TIER_COUNT,
    wordEntriesOf,
    type Binding,
    type BoundsWarning,
    type DictItem,
    type LibraryInfo,
    type Meta,
    type Origin,
    type TableItem,
    type TableMeta,
    type TierCurve,
    type TierMethod,
  } from '$lib/types';

  /** 数据文件夹那一块的文案 key（与「词典管理」页共用同一批） */
  const ORIGIN_LABELS: Record<Origin, MessageKey> = {
    seeded: 'dicts.origin.seeded',
    imported: 'dicts.origin.imported',
    scanned: 'dicts.origin.scanned',
    unknown: 'dicts.origin.unknown',
  };

  const METHOD_LABELS: Record<TierMethod, MessageKey> = {
    top_pct: 'tables.method.topPct',
    rank: 'tables.method.rank',
    coverage: 'tables.method.coverage',
    even: 'tables.method.even',
  };

  const METHOD_NOTES: Record<TierMethod, { what: MessageKey; when: MessageKey }> = {
    top_pct: {
      what: 'tables.methodNote.topPct.what',
      when: 'tables.methodNote.topPct.when',
    },
    rank: {
      what: 'tables.methodNote.rank.what',
      when: 'tables.methodNote.rank.when',
    },
    coverage: {
      what: 'tables.methodNote.coverage.what',
      when: 'tables.methodNote.coverage.when',
    },
    even: {
      what: 'tables.methodNote.even.what',
      when: 'tables.methodNote.even.when',
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
    /** 阈值回退警告（**错误码**；渲染在模板里，见 `BoundsWarning`） */
    warning: BoundsWarning | null;
    /** 「阈值被夹到词条数」的说明（本页生成，已是本地化文案） */
    capNote: string;
  };

  const dark = $derived(isDark());

  let meta = $state<Meta | null>(null);
  let loading = $state(true);
  let loadError = $state('');

  // ------------------------------------------------- 数据文件夹（词频表管理）
  /** 数据文件夹的整体状况（`library_info`） */
  let library = $state<LibraryInfo | null>(null);
  /** `table_list()` 的全部表：数据文件夹里的 + 数据文件夹之外那张激活的 */
  let tables = $state<TableItem[]>([]);
  /** `dict_list()`：把记录里的 `dicts[].name` 映射成"现在还在不在"（缺失时提示） */
  let dicts = $state<DictItem[]>([]);
  let tablesError = $state('');
  let tableBusy = $state('');
  /** 等待确认删除的表名（null = 没有待确认的删除） */
  let pendingTableDelete = $state<string | null>(null);

  // ------------------------------------------------- 相加
  /** 勾选要相加的**表组**（表组相加：整表组的 word+char 一起加） */
  let composePicks = $state<string[]>([]);
  /** 新表组的名字 */
  let composeName = $state('');
  /** 用户是否手动改过新表名：一旦手动输入过就不再自动覆盖，直到清空/重置 */
  let composeNameTouched = $state(false);
  let composeBusy = $state(false);
  let composeError = $state('');
  let composeDone = $state('');

  let notice = $state('');
  let noticeTone = $state<'info' | 'error' | 'success'>('info');

  /** 编辑中的阈值（排名 / 前% / 等分模式用），切方法时由 effect 从生效值填入 */
  let wordBounds = $state<number[]>([]);
  let charBounds = $state<number[]>([]);
  /** 编辑中的**前%上界**（0..100），前%模式用 */
  let pctBounds = $state<number[]>([]);
  /** 编辑中的覆盖率目标（0..1），覆盖率模式用 */
  let coverage = $state<number[]>([]);

  /** 覆盖率曲线缓存（主表组的词频表 / 字表） */
  let curves = $state<{ word: TierCurve | null; char: TierCurve | null }>({ word: null, char: null });
  let curveBusy = $state(false);
  let curveError = $state('');

  let saving = $state(false);

  // ---------------------------------------------------------------- 派生

  /** 分组标签（展示用；本地化在 `i18n.svelte.ts::tierLabels()`） */
  const names = $derived(tierNamesOf(meta));

  /** 七组稳定标识：取色与身份都走它，不走组名 */
  const keys = $derived(tierKeysFrom(meta));

  /** 全部表组（表侧），`full` 优先 */
  const scopes = $derived(metaScopes(meta));

  /** 主表组名（用户指定的那张；未指定时 `full` 优先） */
  const primary = $derived(primaryScope(meta));

  /**
   * 当前激活表的产物目录绝对路径（`tier_curve` 这类按目录定位的命令要用它）。
   * 规则见 bridge.ts 的 `activeTableDir()`。
   */
  const currentDir = $derived(activeTableDir(library));

  /** 本页展示的一行：一个表组 × 一种类型 */
  type Row = { scope: string; table: TableMeta; isPrimary: boolean };

  const rows = $derived.by(() => {
    const list: Row[] = [];
    for (const scope of scopes) {
      for (const kind of ['word', 'char'] as const) {
        const table = meta?.tables.find((x) => x.path === scope && x.kind === kind);
        if (table) list.push({ scope, table, isPrimary: scope === primary });
      }
    }
    // 主表组那两张排最前，其余按表组名
    return list.sort((a, b) => {
      if (a.isPrimary !== b.isPrimary) return a.isPrimary ? -1 : 1;
      return a.scope.localeCompare(b.scope) || a.table.kind.localeCompare(b.table.kind);
    });
  });

  /**
   * 数据文件夹里现存的词典名（含文件名去后缀与 `dict.name`）。
   *
   * 表里记录的 `dicts[].name` 对着它查一遍，就能在界面上说"这份词典还在不在"——
   * `binding` 已经从后端拿到了结论，这里只是给同义词/改名的情况兜个底。
   */
  const knownDictNames = $derived.by(() => {
    const out = new Set<string>();
    for (const item of dicts) {
      out.add(item.file_name);
      if (item.file_name.endsWith('.dict')) out.add(item.file_name.slice(0, -'.dict'.length));
      if (item.dict.id) out.add(item.dict.id);
      if (item.dict.name) out.add(item.dict.name);
    }
    return out;
  });

  /** 该表的词条数：取 `meta.tables` 里 `kind === 'word'` 的那张 */
  function tableEntries(item: TableItem): number | null {
    return wordEntriesOf(item.meta);
  }

  /** 该表记录的词典链里，现在已经不在数据文件夹里的那些名字 */
  function absentDicts(item: TableItem): string[] {
    const refs = item.meta?.tokenizer.dicts ?? [];
    return refs
      .map((ref) => ref.name || ref.id)
      .filter((name) => !!name && !knownDictNames.has(name));
  }

  const method = $derived<TierMethod>(
    appSettings.value.tierMethod === 'rank' ||
      appSettings.value.tierMethod === 'coverage' ||
      appSettings.value.tierMethod === 'even'
      ? appSettings.value.tierMethod
      : 'top_pct'
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
      const table = findTable(meta.tables, kind, primary);
      const info = activeBoundsInfo(kind, meta);
      const bounds = info.bounds;
      const total = table?.total_tokens ?? 0;
      const curve = curves[kind];
      // 曲线最后一点的覆盖率必须是 1，按点累加才等于「累计覆盖率」
      const complete = curve !== null && Math.abs((curve.points[curve.points.length - 1]?.[1] ?? 0) - 1) < 1e-6;
      const grouped = groupTokens(table, bounds, complete ? curve : null, total);

      let cumulative = 0;
      const previewRows: PreviewRow[] = [];
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
        previewRows.push({
          index,
          name,
          range:
            index >= TIER_BOUND_COUNT
              ? `> ${formatCount(bounds[bounds.length - 1] ?? 0)}`
              : `≤ ${formatCount(bounds[index] ?? 0)}`,
          entries: group.entries,
          coverage: share,
          cumulative: Math.min(1, cumulative),
          source: complete
            ? t('tables.sourceCurve')
            : table?.tier_stats[index]
              ? t('tables.sourceEstimated')
              : '—',
        });
      });

      const skipped = bounds.filter((value) => value > entries).length;
      const capNote =
        skipped > 0
          ? t('tables.previewCapNote', { skipped, entries: formatInt(entries) })
          : '';
      out.push({
        kind,
        rows: previewRows,
        bounds,
        warning: info.warning,
        capNote,
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

  /** 反解预览（前%口径）：前%上界 → 排名上界 */
  const solvedPct = $derived.by(() =>
    (['word', 'char'] as const).map((kind) => {
      const table = findTable(meta?.tables ?? [], kind, primary);
      const entries = table?.entries ?? 0;
      const ranks = pctBounds.map((p) =>
        entries > 0 ? Math.max(1, Math.ceil((p / 100) * entries)) : 0
      );
      return { kind, ranks };
    })
  );

  // ---------------------------------------------------------------- 生命周期

  $effect(() => {
    void bootstrap();
  });

  // 切到「按排名 / 等分」时，把编辑框填成当前生效值
  $effect(() => {
    const current = meta;
    if (!current || method === 'coverage' || method === 'top_pct') return;
    wordBounds = activeBoundsInfo('word', current).bounds;
    charBounds = activeBoundsInfo('char', current).bounds;
  });

  // 切到「按覆盖率」时，把编辑框填成设置里的目标（没有就用 meta 默认）
  $effect(() => {
    if (method !== 'coverage') return;
    coverage = [...effectiveCoverage];
  });

  // 切到「按前%」时，把编辑框填成设置里的值（没有就用**主表**记的默认口径）
  $effect(() => {
    if (method !== 'top_pct') return;
    const custom = appSettings.value.tierPct;
    const table = findTable(meta?.tables ?? [], 'word', primary);
    pctBounds = custom && custom.length > 0 ? [...custom] : [...(table?.tier_pct ?? defaultTierPct('word'))];
  });

  // 覆盖率 / 前% 模式需要曲线（后者只在用户想看"覆盖了多少正文"时用）
  $effect(() => {
    if (method !== 'coverage') return;
    if (!currentDir) return;
    void autoFetchCurves();
  });

  async function bootstrap() {
    loading = true;
    loadError = '';
    tablesError = '';

    // 数据文件夹状况 + 词频表清单 + 词典清单（词频表清单要用词典清单判断"词典还在不在"）
    const [infoRes, tablesRes, dictsRes] = await Promise.all([
      libraryInfo(),
      tableList(),
      dictList(),
    ]);
    if (infoRes.ok) library = infoRes.data;
    if (tablesRes.ok) {
      tables = tablesRes.data;
      if (pendingTableDelete && !tables.some((table) => table.name === pendingTableDelete)) {
        pendingTableDelete = null;
      }
    } else {
      tablesError = tablesRes.error;
    }
    if (dictsRes.ok) dicts = dictsRes.data;

    // 当前打开的那张表的 meta：**取**后端的，不再自己拼 settings.dataDir 的路径。
    // dataDir 现在是「数据文件夹」（里面是 dicts\ 与 tables\），它下面没有 meta.json，
    // 拼出来必然是 exists: false，于是页面永远显示"尚未打开词频表"。
    const current = await activeDataset();
    if (!current.ok) {
      loadError = current.error;
      loading = false;
      return;
    }
    meta = current.data;
    // 后端在启动 / 激活时已经按 meta.tokenizer 的词典链重建过分词器了，这里只需取 meta
    loading = false;
  }

  // ---------------------------------------------------------------- 词频表管理

  /** 重新拉一次数据文件夹状况与词频表清单 */
  async function refreshTables() {
    const [infoRes, tablesRes, dictsRes] = await Promise.all([
      libraryInfo(),
      tableList(),
      dictList(),
    ]);
    if (infoRes.ok) library = infoRes.data;
    if (tablesRes.ok) {
      tables = tablesRes.data;
      if (pendingTableDelete && !tables.some((table) => table.name === pendingTableDelete)) {
        pendingTableDelete = null;
      }
    }
    if (dictsRes.ok) dicts = dictsRes.data;
  }

  /** 激活某张表：后端会按它记录的词典链重建分词器，并把 meta 换成这张表的 */
  async function activateTable(item: TableItem) {
    tableBusy = item.name;
    const res = await activateLibraryTable(item.name);
    tableBusy = '';
    if (!res.ok) {
      showNotice(t('tables.library.activateFailed', { name: item.name, error: res.error }), 'error');
      return;
    }
    // 曲线是按产物缓存的，换了表要重新拉
    curves = { word: null, char: null };
    curveError = '';
    await refreshTables();
    meta = res.data;
    showNotice(t('tables.library.activated', { name: item.name }), 'success');
  }

  /**
   * 重新统计：把该表的语料库与参数放进「一次性交接单」，再跳到扫描页。
   *
   * 为什么不在本页直接起扫描：扫描表单（线程 / HMM / 表组勾选 …）是扫描页的状态，
   * 在这里复制一份必然漂移。交接单的说明见 `$lib/scan-prefill.svelte.ts`。
   */
  function rescanTable(item: TableItem) {
    const tokenizer = item.meta?.tokenizer;
    requestScanPrefill({
      corpus: item.meta?.corpus_root ?? '',
      tableName: item.name,
      // 用该表记录的词典链预勾选（名字就是 `dicts\` 下的文件名）；空数组 = 用全部
      dictFiles: tokenizer
        ? tokenizer.dicts.map((ref) => ref.name || ref.id).filter(Boolean)
        : [],
    });
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'wordfreq' }));
  }

  /** 删除某张表（只有 `in_library === true` 才给这个入口） */
  async function deleteTable(item: TableItem) {
    tableBusy = item.name;
    const res = await tableDelete(item.name);
    tableBusy = '';
    if (!res.ok) {
      showNotice(t('tables.library.deleteFailed', { name: item.name, error: res.error }), 'error');
      return;
    }
    pendingTableDelete = null;
    await refreshTables();
    // 删掉的可能正是当前激活的那张（后端会把激活项清掉）→ 重新取一次 meta
    const current = await activeDataset();
    meta = current.ok ? current.data : null;
    curves = { word: null, char: null };
    showNotice(t('tables.library.deleted', { name: item.name }), 'success');
  }

  async function autoFetchCurves() {
    if (curves.word && curves.char) return;
    await fetchCurves();
  }

  async function fetchCurves() {
    const dir = currentDir;
    curveBusy = true;
    curveError = '';
    const [word, char] = await Promise.all([
      ensureCurve(tableKey(primary, 'word'), dir, 600),
      ensureCurve(tableKey(primary, 'char'), dir, 600),
    ]);
    if (word.ok) curves.word = word.data;
    if (char.ok) curves.char = char.data;
    curveBusy = false;
    if (!word.ok) curveError = word.error;
    else if (!char.ok) curveError = char.error;
  }

  // ---------------------------------------------------------------- 绑定状态

  /**
   * 绑定状态 → 徽标样式与文案 key。
   *
   * 词典外置之后，**一张表的频次是否可信**取决于它记录的词典链现在还成不成立。
   * 所以四种状态各有颜色与文案，不能只给个图标。
   */
  function bindingTone(binding: Binding): 'ok' | 'warn' | 'error' {
    if (binding.kind === 'ok') return 'ok';
    if (binding.kind === 'missing') return 'error';
    return 'warn';
  }

  function bindingLabelKey(binding: Binding): MessageKey {
    if (binding.kind === 'ok') return 'tables.binding.ok';
    if (binding.kind === 'legacy') return 'tables.binding.legacy';
    if (binding.kind === 'drifted') return 'tables.binding.drifted';
    return 'tables.binding.missing';
  }

  /** 绑定异常的详细一行（列出变了 / 缺了的词典名） */
  function bindingDetail(binding: Binding): string {
    if (binding.kind === 'drifted') {
      return t('tables.binding.driftedDetail', {
        changed: binding.changed.join(t('common.listSeparator')) || '—',
      });
    }
    if (binding.kind === 'missing') {
      return t('tables.binding.missingDetail', {
        missing: binding.missing.join(t('common.listSeparator')) || '—',
      });
    }
    if (binding.kind === 'legacy') return t('tables.binding.legacyDetail');
    return '';
  }

  function showNotice(message: string, tone: 'info' | 'error' | 'success' = 'info') {
    notice = message;
    noticeTone = tone;
    window.setTimeout(() => {
      if (notice === message) notice = '';
    }, 4000);
  }

  // ---------------------------------------------------------------- 主词频表

  /** 把某个表组设为主表（词频表与字表一起换，口径只能有一套） */
  async function makePrimary(scope: string) {
    if (scope === primary) return;
    tableBusy = `primary:${scope}`;
    const res = await setPrimaryScope(scope);
    tableBusy = '';
    if (!res.ok) {
      showNotice(t('tables.primaryFailed', { scope, error: res.error }), 'error');
      return;
    }
    // 后端已把 primary_scope 持久化并重开数据集；这里必须同步前端共享状态，
    // 否则 primaryScope(meta) 读到的还是旧值，会一路回退到 full ——
    // 造成「改了主表组却仍显示/生效为 full」的前后端口径分裂。
    appSettings.value.primaryScope = scope;
    meta = res.data;
    curves = { word: null, char: null };
    showNotice(t('tables.primarySet', { scope }), 'success');
  }

  // ---------------------------------------------------------------- 相加

  /** 按当前勾选自动生成新表名（如 `news + wiki`）；无勾选时为空 */
  function autoComposeName(): string {
    return composePicks.join(' + ');
  }

  /** 勾选 / 取消勾选一个**表组**（表组相加：整表组的 word+char 一起加） */
  function toggleComposePick(scope: string) {
    composePicks = composePicks.includes(scope)
      ? composePicks.filter((s) => s !== scope)
      : [...composePicks, scope];
    // 用户还没手动改过名字时，随勾选自动预填组合名
    if (!composeNameTouched) composeName = autoComposeName();
  }

  /** 所有表组是否已全选 */
  function composeAllPicked(): boolean {
    return scopes.length > 0 && scopes.every((s) => composePicks.includes(s));
  }

  /** 全选 / 取消全选所有表组 */
  function toggleComposeAll() {
    composePicks = composeAllPicked() ? [] : [...scopes];
    if (!composeNameTouched) composeName = autoComposeName();
  }

  /** 某表组下现有的表（word / char） */
  function composeScopeTables(scope: string): TableMeta[] {
    return meta?.tables.filter((t) => t.path === scope) ?? [];
  }

  async function runCompose() {
    if (composePicks.length === 0) {
      composeError = t('tables.compose.needPick');
      return;
    }
    // 表组相加：勾选的是**表组**，展开成该表组存在的每类表，词表+字表一起加。
    // 后端 `kind` 不给 = 源表里出现过的每一类都产出（见 compose.rs），跨类天然合法。
    const sources = composePicks.flatMap((scope) =>
      (['word', 'char'] as const)
        .filter((kind) => meta?.tables.some((t) => t.path === scope && t.kind === kind))
        .map((kind) => tableKey(scope, kind))
    );
    composeBusy = true;
    composeError = '';
    composeDone = '';
    const res = await composeTables({
      sources,
      scope: composeName.trim() || t('tables.compose.defaultName'),
      kind: null,
    });
    composeBusy = false;
    if (!res.ok) {
      composeError = res.error;
      return;
    }
    const scope = res.data[0]?.scope ?? composeName;
    composeDone = t('tables.compose.done', {
      scope,
      kinds: res.data.map((c) => (c.kind === 'char' ? t('table.char') : t('table.word'))).join(' / '),
      entries: formatInt(res.data[0]?.entries ?? 0),
    });
    composePicks = [];
    composeName = '';
    composeNameTouched = false;
    // 新表要立刻出现在清单与表组选择里
    await refreshTables();
    const current = await activeDataset();
    meta = current.ok ? current.data : meta;
    showNotice(t('tables.compose.doneShort', { scope }), 'success');
  }

  // ---------------------------------------------------------------- 分组设置

  async function persist(part: Parameters<typeof updateSettings>[0], message: string) {
    saving = true;
    const res = await updateSettings(part);
    saving = false;
    if (!res.ok) {
      showNotice(t('tables.saveFailed', { error: res.error }), 'error');
      return;
    }
    showNotice(message, 'success');
  }

  function switchMethod(next: TierMethod) {
    if (next === method) return;
    void persist(
      { tierMethod: next },
      t('tables.methodSwitched', { method: t(METHOD_LABELS[next]) })
    );
  }

  function updateBound(kind: 'word' | 'char', index: number, raw: string) {
    const value = Math.max(0, Math.round(Number(raw) || 0));
    if (kind === 'word') {
      const next = [...wordBounds];
      next[index] = value;
      wordBounds = next;
      void persist({ tierWordBounds: next }, t('tables.wordBoundsSaved'));
    } else {
      const next = [...charBounds];
      next[index] = value;
      charBounds = next;
      void persist({ tierCharBounds: next }, t('tables.charBoundsSaved'));
    }
  }

  /** 改一个前%上界（输入是百分数，存的是百分数 0..100） */
  function updatePctBound(index: number, raw: string) {
    const value = Math.min(100, Math.max(0, Number(raw) || 0));
    const next = [...pctBounds];
    next[index] = value;
    pctBounds = next;
    void persist({ tierPct: next }, t('tables.pctBoundsSaved'));
  }

  function updateCoverage(index: number, raw: string) {
    const value = Math.min(1, Math.max(0, (Number(raw) || 0) / 100));
    const next = [...coverage];
    next[index] = value;
    coverage = next;
    void persist({ tierCoverage: next }, t('tables.coverageSaved'));
  }

  function resetBounds(kind: 'word' | 'char' | 'coverage' | 'pct') {
    if (kind === 'coverage') {
      void persist({ tierCoverage: null }, t('tables.coverageReset'));
      return;
    }
    if (kind === 'pct') {
      void persist({ tierPct: null }, t('tables.pctBoundsReset'));
      return;
    }
    void persist(
      kind === 'word' ? { tierWordBounds: null } : { tierCharBounds: null },
      kind === 'word' ? t('tables.wordBoundsReset') : t('tables.charBoundsReset')
    );
  }

  async function solveFromCurve() {
    await fetchCurves();
    const word = curves.word;
    const char = curves.char;
    if (!word || !char) {
      showNotice(
        curveError
          ? t('tables.solveFailedWithError', { error: curveError })
          : t('tables.solveFailed'),
        'error'
      );
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
      t('tables.solved', { ranks: wordRanks.map(formatInt).join(' / ') })
    );
  }

  /** 把「前%」口径反解成绝对排名，存进 rank 口径（两条路都能走，给用户一个退路） */
  async function solveFromPct() {
    const ranks = solvedPct.map((item) => item.ranks);
    const wordRanks = ranks[0] ?? [];
    const charRanks = ranks[1] ?? [];
    void persist(
      {
        tierWordBounds: wordRanks,
        tierCharBounds: charRanks,
        tierMethod: 'rank',
      },
      t('tables.solved', { ranks: wordRanks.map(formatInt).join(' / ') })
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

  function rowLabel(table: TableMeta): string {
    return table.kind === 'char' ? t('table.char') : t('table.word');
  }

  /** 该表最后一组（最少见的那一档）的覆盖率 */
  function lastTierCoverage(table: TableMeta): number {
    const stat = table.tier_stats[table.tier_stats.length - 1];
    return stat?.coverage ?? 0;
  }

  /** 这张表是相加出来的吗（有来源就是） */
  function isComposed(table: TableMeta): boolean {
    return (table.source_tables?.length ?? 0) > 0;
  }

  function goWordFreq() {
    window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: 'wordfreq' }));
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      {t('tables.browserPreview')}
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
        {t('tables.loading')}
      </CardContent>
    </Card>
  {:else}
    <!-- ==================== 词频表库（数据文件夹 tables\） ==================== -->
    <SectionCard
      section={PAGE_SECTIONS.library}
      descriptionKey="tables.library.description"
      data-testid="library-tables-card"
    >
      {#snippet titleExtra()}
        <Badge variant="secondary">{t('tables.library.countBadge', { count: formatInt(tables.length) })}</Badge>
        {#if tables.find((table) => table.active)}
          <Badge variant="outline">
            {t('tables.library.activeBadge', { name: tables.find((table) => table.active)?.name ?? '' })}
          </Badge>
        {/if}
        <Button variant="outline" size="sm" class="ml-auto" onclick={() => void refreshTables()}>
          {t('tables.recheck')}
        </Button>
      {/snippet}
        {#if library}
          <div class="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
            <span>{t('tables.library.dataDir', { dir: library.root })}</span>
            {#if library.is_default}
              <Badge variant="outline">{t('dicts.defaultLocation')}</Badge>
            {/if}
          </div>
        {/if}

        {#if tablesError}
          <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
            {t('tables.library.loadFailed', { error: tablesError })}
          </p>
        {/if}

        <!-- 当前激活表的绑定状态：词典外置之后最关键的一块，必须显眼 -->
        {#if library && library.active_binding}
          {@const tone = bindingTone(library.active_binding)}
          <div
            class={cn(
              'rounded-lg border px-3 py-2 text-xs',
              tone === 'ok'
                ? 'border-emerald-500/30 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400'
                : tone === 'warn'
                  ? 'border-amber-500/30 bg-amber-500/5 text-amber-700 dark:text-amber-300'
                  : 'border-destructive/40 bg-destructive/5 text-destructive'
            )}
            data-testid="active-binding"
          >
            <p class="font-medium">
              {t('tables.library.activeBinding', { label: t(bindingLabelKey(library.active_binding)) })}
            </p>
            {#if bindingDetail(library.active_binding)}
              <p class="mt-0.5">{bindingDetail(library.active_binding)}</p>
            {/if}
          </div>
        {/if}

        <!-- `library_info().active_warnings`：打开表时攒下的告警，直接显示出来 -->
        {#if library && library.active_warnings.length > 0}
          <ul
            class="flex flex-col gap-0.5 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] text-amber-700 dark:text-amber-300"
            data-testid="active-warnings"
          >
            {#each library.active_warnings as warning, index (index)}
              <li>⚠ {warning}</li>
            {/each}
          </ul>
        {/if}

        {#if tables.length === 0}
          <p class="rounded-lg border border-dashed border-border px-3 py-6 text-center text-xs text-muted-foreground">
            {t('tables.library.empty')}
          </p>
        {/if}

        {#each tables as item (item.path)}
          {@const tone = bindingTone(item.binding)}
          {@const missing = absentDicts(item)}
          <div
            class={cn(
              'flex flex-col gap-2 rounded-lg border p-3',
              item.active ? 'border-primary/40 bg-primary/5' : 'border-border'
            )}
            data-library-table={item.name}
          >
            <div class="flex flex-wrap items-center gap-2">
              <span class="font-medium">{item.name}</span>
              {#if item.active}
                <Badge variant="success">{t('tables.library.active')}</Badge>
              {/if}
              <Badge variant="outline">{t(ORIGIN_LABELS[item.origin])}</Badge>
              {#if !item.in_library}
                <Badge variant="outline">{t('tables.library.external')}</Badge>
              {/if}
              <span class="text-[11px] text-muted-foreground" data-testid="library-table-entries">
                {#if tableEntries(item) === null}
                  {t('tables.library.entriesUnavailable')}
                {:else}
                  {t('dicts.entries', { count: formatInt(tableEntries(item) ?? 0) })}
                {/if}
              </span>
              <span class="ml-auto flex flex-wrap items-center gap-1.5">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={item.active || tableBusy === item.name}
                  onclick={() => void activateTable(item)}
                >
                  {t('tables.library.activate')}
                </Button>
                <Button variant="outline" size="sm" onclick={() => rescanTable(item)}>
                  {t('tables.library.rescan')}
                </Button>
                <Button variant="ghost" size="sm" onclick={() => void openExternal(item.path)}>
                  {t('tables.library.openDir')}
                </Button>
                <!-- 数据文件夹之外的表不给删除：那个目录不归我们管 -->
                {#if item.in_library}
                  <Button
                    variant={pendingTableDelete === item.name ? 'destructive' : 'ghost'}
                    size="sm"
                    disabled={tableBusy === item.name}
                    onclick={() => (pendingTableDelete = pendingTableDelete === item.name ? null : item.name)}
                  >
                    {t('tables.library.delete')}
                  </Button>
                {/if}
              </span>
            </div>

            <p class="selectable truncate font-mono text-[11px] text-muted-foreground">{item.path}</p>

            <div class="flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
              <span>{t('tables.library.corpus', { corpus: item.meta?.corpus_root || '—' })}</span>
              <span>
                {t('tables.library.generated', {
                  generated: item.meta ? formatTimestamp(item.meta.generated_at) : '—',
                })}
              </span>
            </div>

            {#if item.error}
              <p class="rounded-md border border-destructive/40 bg-destructive/5 px-2 py-1.5 text-[11px] text-destructive">
                {t('tables.library.tableError', { error: item.error })}
              </p>
            {/if}

            <p
              class={cn(
                'rounded-md border px-2 py-1.5 text-[11px]',
                tone === 'ok'
                  ? 'border-emerald-500/30 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400'
                  : tone === 'warn'
                    ? 'border-amber-500/40 bg-amber-500/5 text-amber-700 dark:text-amber-300'
                    : 'border-destructive/40 bg-destructive/5 text-destructive'
              )}
              data-binding-kind={item.binding.kind}
            >
              <span class="font-medium">{t(bindingLabelKey(item.binding))}</span>
              {#if bindingDetail(item.binding)}
                <span class="ml-1">{bindingDetail(item.binding)}</span>
              {/if}
              {#if tone !== 'ok'}
                <Button variant="ghost" size="sm" class="ml-1.5 h-6 px-2 text-[11px]" onclick={() => rescanTable(item)}>
                  {t('tables.library.rescanSuggestion')}
                </Button>
              {/if}
            </p>

            {#if item.meta && item.meta.tokenizer.dicts.length > 0}
              <p class="text-[11px] text-muted-foreground">
                {t('tables.library.dictChain', {
                  chain: item.meta.tokenizer.dicts
                    .map((ref) => ref.name || ref.id || '—')
                    .join(t('common.listSeparator')),
                })}
                {#if missing.length > 0}
                  <span class="text-destructive">
                    {t('tables.library.absentDicts', { names: missing.join(t('common.listSeparator')) })}
                  </span>
                {/if}
              </p>
            {/if}

            {#if pendingTableDelete === item.name}
              <div
                class="flex flex-col gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-[11px] text-destructive"
                data-testid="table-delete-confirm"
              >
                <p class="font-medium">{t('tables.library.confirmDeleteTitle', { name: item.name })}</p>
                <p>{t('tables.library.confirmDeleteNote')}</p>
                <span class="flex gap-1.5">
                  <Button variant="destructive" size="sm" disabled={tableBusy === item.name} onclick={() => void deleteTable(item)}>
                    {t('tables.library.confirmDeleteYes')}
                  </Button>
                  <Button variant="outline" size="sm" onclick={() => (pendingTableDelete = null)}>
                    {t('common.cancel')}
                  </Button>
                </span>
              </div>
            {/if}
          </div>
        {/each}
  </SectionCard>

    {#if loadError}
      <Card class="border-destructive/30">
        <CardContent class="py-6 text-xs text-destructive">
          {t('tables.loadFailed', { error: loadError })}
        </CardContent>
      </Card>
    {/if}

    {#if !meta}
      <Card class="border-dashed">
        <CardHeader>
          <CardTitle>{t('tables.emptyTitle')}</CardTitle>
          <CardDescription>{t('tables.emptyDescription')}</CardDescription>
        </CardHeader>
        <CardContent class="flex gap-2">
          <Button onclick={goWordFreq}>{t('tables.goWordFreq')}</Button>
          <Button variant="outline" onclick={() => void bootstrap()}>{t('tables.recheck')}</Button>
        </CardContent>
      </Card>
    {:else}
      {@const segPrimary = splitMessage('tables.primaryDescription', ['emphasis'], {
        emphasis: t('tables.primaryEmphasis'),
      })}
      {@const segTierDesc = splitMessage('tables.tierConfigDescription', ['emphasis'], {
        names: names.join(' / '),
        emphasis: t('tables.rankUpperBound'),
      })}

      <!-- ==================== 主表组 ==================== -->
      <SectionCard section={PAGE_SECTIONS.primary} descriptionKey="tables.primaryDomain.description" data-testid="primary-scope-card">
        {#snippet titleExtra()}
          <Badge variant="outline">{t('tables.primaryBadge', { scope: primary })}</Badge>
        {/snippet}
          {#each scopes as scope}
            {@const word = findTable(meta.tables, 'word', scope)}
            {@const char = findTable(meta.tables, 'char', scope)}
            <label
              class={cn(
                'flex cursor-pointer items-center gap-3 rounded-lg border px-3 py-2 text-xs',
                scope === primary ? 'border-primary/40 bg-primary/5' : 'border-border'
              )}
            >
              <input
                type="radio"
                name="primary-scope"
                class="size-3.5 accent-[var(--primary)]"
                checked={scope === primary}
                disabled={tableBusy === `primary:${scope}`}
                onchange={() => void makePrimary(scope)}
                aria-label={t('tables.primaryAria', { scope })}
                data-primary-radio={scope}
              />
              <span class="font-medium">{scope}</span>
              <span class="ml-auto text-muted-foreground">
                {t('tables.primaryDomain.scopeSummary', {
                  word: formatInt(word?.entries ?? 0),
                  char: formatInt(char?.entries ?? 0),
                })}
              </span>
            </label>
          {/each}
  </SectionCard>

      <!-- ==================== A. 频率表（全部表组，平等） ==================== -->
      <SectionCard section={PAGE_SECTIONS.list} data-testid="table-list-card">
        {#snippet titleExtra()}
          <Badge variant="secondary" data-testid="table-count">
            {t('tables.tableCountBadge', { count: formatInt(rows.length) })}
          </Badge>
          <Badge variant="outline">{t('tables.scopeCountBadge', { count: formatInt(scopes.length) })}</Badge>
          <Badge variant="outline">{t('tables.primaryBadge', { scope: primary })}</Badge>
          {#if saving}<Badge variant="outline">{t('tables.saving')}</Badge>{/if}
        {/snippet}
        {#snippet description()}
          {segPrimary[0]}<b>{segPrimary[1]}</b>{segPrimary[2]}
        {/snippet}
          <div class="scrollbar-thin max-h-[30rem] overflow-auto rounded-lg border border-border">
            <table class="w-full border-collapse text-xs">
              <thead class="sticky top-0 z-10 bg-surface-muted text-muted-foreground">
                <tr>
                  <th class="px-3 py-2 text-left font-medium">{t('tables.col.scope')}</th>
                  <th class="px-3 py-2 text-left font-medium">{t('tables.col.kind')}</th>
                  <th class="px-3 py-2 text-right font-medium">{t('tables.col.entries')}</th>
                  <th class="px-3 py-2 text-right font-medium">{t('tables.col.tokens')}</th>
                  <th class="px-3 py-2 text-right font-medium">{t('tables.col.vfr')}</th>
                  <th class="px-3 py-2 text-right font-medium">{t('tables.col.lastCoverage')}</th>
                  <th class="px-3 py-2 text-left font-medium">{t('tables.col.source')}</th>
                </tr>
              </thead>
              <tbody>
                {#each rows as row (tableKey(row.scope, row.table.kind))}
                  <tr class={cn('border-t border-border/70')} data-table-row={tableKey(row.scope, row.table.kind)}>
                    <td class="px-3 py-2">
                      <span class="font-medium">{row.scope}</span>
                      <span class="ml-2 font-mono text-[11px] text-muted-foreground"
                        >{tableKey(row.scope, row.table.kind)}</span
                      >
                    </td>
                    <td class="px-3 py-2">{rowLabel(row.table)}</td>
                    <td class="px-3 py-2 text-right tabular-nums">{formatInt(row.table.entries)}</td>
                    <td class="px-3 py-2 text-right tabular-nums">{formatInt(row.table.total_tokens)}</td>
                    <td class="px-3 py-2 text-right tabular-nums">{formatBytes(row.table.vfr_bytes)}</td>
                    <td class="px-3 py-2 text-right tabular-nums">
                      {formatRatio(lastTierCoverage(row.table), { digits: 3 })}
                    </td>
                    <td class="px-3 py-2">
                      {#if isComposed(row.table)}
                        <span
                          class="rounded border border-primary/40 px-1.5 py-0.5 text-[11px] text-primary"
                          title={(row.table.source_tables ?? []).join(' + ')}
                        >
                          {t('tables.sourceComposed', { count: row.table.source_tables?.length ?? 0 })}
                        </span>
                      {:else}
                        <span class="text-[11px] text-muted-foreground">{t('tables.sourceScanned')}</span>
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>

          <p class="rounded-md bg-surface-muted/60 px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
            {t('tables.equalNote')}
          </p>
  </SectionCard>

      <!-- ==================== B. 相加 ==================== -->
      <SectionCard section={PAGE_SECTIONS.compose} descriptionKey="tables.compose.description" data-testid="compose-card">
        {#snippet titleExtra()}
          {#if composePicks.length > 0}
            <Badge variant="secondary">
              {t('tables.compose.pickedBadge', { count: formatInt(composePicks.length) })}
            </Badge>
          {/if}
          {#if composeBusy}<Badge variant="outline">{t('tables.compose.running')}</Badge>{/if}
        {/snippet}
          <div class="flex flex-col gap-2">
            <div class="flex items-center justify-between gap-2">
              <span class="text-xs font-medium">{t('tables.compose.pickLabel')}</span>
              <button
                type="button"
                class="text-[11px] text-primary hover:underline"
                onclick={() => toggleComposeAll()}
                data-compose-select-all
              >
                {composeAllPicked() ? t('common.clear') : t('tables.compose.selectAll')}
              </button>
            </div>
            <div class="max-h-64 overflow-auto rounded-lg border border-border">
              {#each scopes as scope (scope)}
                {@const tables = composeScopeTables(scope)}
                {@const wordTable = tables.find((t) => t.kind === 'word')}
                {@const charTable = tables.find((t) => t.kind === 'char')}
                <label
                  class="flex cursor-pointer items-center gap-2 px-3 py-1.5 text-xs transition-colors hover:bg-accent"
                  data-compose-row={scope}
                >
                  <input
                    type="checkbox"
                    class="size-3.5 accent-[var(--primary)]"
                    checked={composePicks.includes(scope)}
                    onchange={() => toggleComposePick(scope)}
                    data-compose-pick={scope}
                  />
                  <span class="min-w-0 flex-1 truncate font-medium">{scope}</span>
                  <span class="shrink-0 text-[10px] text-muted-foreground"
                    >{wordTable ? t('table.word') : ''}{wordTable && charTable ? ' + ' : ''}{charTable ? t('table.char') : ''}</span
                  >
                  <span class="shrink-0 text-[10px] tabular-nums text-muted-foreground"
                    >{wordTable ? formatInt(wordTable.entries) : ''}</span
                  >
                  <span class="shrink-0 text-[10px] tabular-nums text-muted-foreground"
                    >{wordTable ? formatRatio(lastTierCoverage(wordTable), { digits: 3 }) : ''}</span
                  >
                </label>
              {/each}
            </div>
          </div>

          <div class="flex flex-wrap items-end gap-2">
            <label class="flex flex-col gap-1">
              <span class="text-[11px] text-muted-foreground">{t('tables.compose.nameLabel')}</span>
              <input
                type="text"
                class="w-56 rounded border border-input bg-surface px-2 py-1 text-xs"
                placeholder={t('tables.compose.namePlaceholder')}
                value={composeName}
                oninput={(e) => {
                  composeName = e.currentTarget.value;
                  // 用户手动输入过 → 之后的勾选不再自动覆盖，直到清空/重置
                  composeNameTouched = true;
                }}
                data-testid="compose-name"
              />
            </label>
            <Button
              size="sm"
              disabled={composeBusy || composePicks.length === 0}
              onclick={() => void runCompose()}
            >
              {composeBusy ? t('tables.compose.running') : t('tables.compose.run')}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={composeBusy || composePicks.length === 0}
              onclick={() => {
                composePicks = [];
                composeName = '';
                composeNameTouched = false;
                composeError = '';
              }}
            >
              {t('common.clear')}
            </Button>
          </div>

          {#if composeError}
            <p class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive" data-testid="compose-error">
              {composeError}
            </p>
          {/if}
          {#if composeDone}
            <p class="rounded-md border border-emerald-500/30 bg-emerald-500/5 px-3 py-2 text-xs text-emerald-600 dark:text-emerald-400" data-testid="compose-done">
              {composeDone}
            </p>
          {/if}

          <p class="text-[11px] leading-relaxed text-muted-foreground">
            {t('tables.compose.note')}
          </p>
  </SectionCard>

      <!-- ==================== C. 分组自定义 ==================== -->
      <SectionCard section={PAGE_SECTIONS.tier} contentClass="flex flex-col gap-4" data-testid="tier-config-card">
        {#snippet titleExtra()}
          <Badge variant="outline">{t('tables.tierCountBadge')}</Badge>
          <Badge variant="secondary">{t(METHOD_LABELS[method])}</Badge>
          <Badge variant="outline">{t('tables.primaryBadge', { scope: primary })}</Badge>
        {/snippet}
        {#snippet description()}
          {segTierDesc[0]}<b>{segTierDesc[1]}</b>{segTierDesc[2]}
        {/snippet}
          <!-- 方法切换 -->
          <div class="flex flex-col gap-2">
            <span class="text-xs font-medium">{t('tables.methodLabel')}</span>
            <div class="flex flex-wrap gap-1.5">
              {#each ['top_pct', 'rank', 'coverage', 'even'] as TierMethod[] as item (item)}
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
                  {t(METHOD_LABELS[item])}
                </button>
              {/each}
            </div>
            <div class="rounded-lg border border-border bg-surface-muted/40 px-3 py-2 text-[11px] leading-relaxed">
              <p>
                <span class="font-medium">{t('tables.whatLabel')}</span>{t(METHOD_NOTES[method].what)}
              </p>
              <p class="mt-1">
                <span class="font-medium">{t('tables.whenLabel')}</span>{t(METHOD_NOTES[method].when)}
              </p>
            </div>
          </div>

          <Separator />

          <!-- 编辑区 -->
          {#if method === 'top_pct'}
            <div class="flex flex-col gap-3">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-xs font-medium">{t('tables.pctBoundsTitle')}</span>
                <span class="text-[11px] text-muted-foreground">{t('tables.pctBoundsHint')}</span>
                <span class="ml-auto flex gap-1">
                  <Button variant="ghost" size="sm" onclick={() => resetBounds('pct')}>
                    {t('tables.resetDefault')}
                  </Button>
                </span>
              </div>

              <div class="flex flex-wrap gap-2">
                {#each Array.from({ length: TIER_BOUND_COUNT }, (_, i) => i) as index (index)}
                  <label class="flex flex-col gap-1">
                    <span class="text-[11px] text-muted-foreground"
                      >{t('tables.groupN', { index: index + 1 })}</span
                    >
                    <input
                      type="number"
                      min="0"
                      max="100"
                      step="0.001"
                      class="w-28 rounded border border-input bg-surface px-2 py-1 text-right text-xs tabular-nums"
                      data-testid={`pct-bound-${index}`}
                      value={pctBounds[index] ?? 0}
                      onchange={(event) => updatePctBound(index, event.currentTarget.value)}
                      aria-label={t('tables.pctBoundAria', { index: index + 1 })}
                    />
                  </label>
                {/each}
              </div>

              <div class="grid grid-cols-1 gap-3 lg:grid-cols-2">
                {#each solvedPct as item (item.kind)}
                  <div class="rounded-lg border border-border p-3">
                    <p class="mb-2 text-xs font-medium">
                      {item.kind === 'word'
                        ? t('tables.solvedWordTitle')
                        : t('tables.solvedCharTitle')}
                    </p>
                    <div class="flex flex-wrap gap-x-4 gap-y-1 text-[11px]">
                      {#each item.ranks as rank, index (index)}
                        <span>
                          <span class="text-muted-foreground"
                            >{t('tables.groupUpperLe', { index: index + 1 })}</span
                          >
                          <span class="ml-1 tabular-nums font-medium" data-testid={`solved-pct-${item.kind}-${index}`}>
                            {rank === 0 ? '—' : formatInt(rank)}
                          </span>
                        </span>
                      {/each}
                    </div>
                  </div>
                {/each}
              </div>

              <p class="text-[11px] text-muted-foreground">
                {t('tables.pctNote', { entries: formatInt(findTable(meta.tables, 'word', primary)?.entries ?? 0) })}
              </p>
            </div>
          {:else if method === 'coverage'}
            <div class="flex flex-col gap-3">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-xs font-medium">{t('tables.coverageTargetLabel')}</span>
                <span class="text-[11px] text-muted-foreground">
                  {t('tables.coverageTargetHint')}
                </span>
                <span class="ml-auto flex gap-1">
                  <Button variant="outline" size="sm" onclick={() => void solveFromCurve()}>
                    {t('tables.solveButton')}
                  </Button>
                  <Button variant="ghost" size="sm" onclick={() => resetBounds('coverage')}
                    >{t('tables.resetDefault')}</Button
                  >
                </span>
              </div>

              <div class="flex flex-wrap gap-2">
                {#each coverage as value, index (index)}
                  <label class="flex flex-col gap-1">
                    <span class="text-[11px] text-muted-foreground"
                      >{t('tables.groupN', { index: index + 1 })}</span
                    >
                    <input
                      type="number"
                      min="0"
                      max="100"
                      step="0.1"
                      class="w-24 rounded border border-input bg-surface px-2 py-1 text-right text-xs tabular-nums"
                      data-testid={`coverage-${index}`}
                      value={(value * 100).toFixed(2)}
                      onchange={(event) => updateCoverage(index, event.currentTarget.value)}
                      aria-label={t('tables.groupCoverageAria', { index: index + 1 })}
                    />
                  </label>
                {/each}
              </div>

              {#if curveError}
                <p class="text-[11px] text-destructive">
                  {t('tables.curveLoadFailed', { error: curveError })}
                </p>
              {:else if curveBusy}
                <p class="text-[11px] text-muted-foreground">{t('tables.curveLoading')}</p>
              {:else}
                <p class="text-[11px] text-muted-foreground">
                  {t('tables.curveSummary', {
                    word: curves.word
                      ? t('tables.curvePoints', { count: formatInt(curves.word.points.length) })
                      : '—',
                    char: curves.char
                      ? t('tables.curvePoints', { count: formatInt(curves.char.points.length) })
                      : '—',
                  })}
                </p>
              {/if}

              <div class="grid grid-cols-1 gap-3 lg:grid-cols-2">
                {#each solved as item (item.kind)}
                  <div class="rounded-lg border border-border p-3">
                    <p class="mb-2 text-xs font-medium">
                      {item.kind === 'word' ? t('tables.solvedWordTitle') : t('tables.solvedCharTitle')}
                    </p>
                    <div class="flex flex-wrap gap-x-4 gap-y-1 text-[11px]">
                      {#each item.ranks as rank, index (index)}
                        <span>
                          <span class="text-muted-foreground"
                            >{t('tables.groupUpperLe', { index: index + 1 })}</span
                          >
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
                    <span class="text-xs font-medium"
                      >{kind === 'word' ? t('tables.wordBoundsTitle') : t('tables.charBoundsTitle')}</span
                    >
                    <span class="text-[11px] text-muted-foreground">{t('tables.sixBoundsHint')}</span>
                    <Button variant="ghost" size="sm" class="ml-auto" onclick={() => resetBounds(kind)}>
                      {t('tables.resetDefault')}
                    </Button>
                  </div>
                  <div class="flex flex-wrap gap-2">
                    {#each Array.from({ length: TIER_BOUND_COUNT }, (_, i) => i) as index (index)}
                      <label class="flex flex-col gap-1">
                        <span class="text-[11px] text-muted-foreground"
                          >{t('tables.groupN', { index: index + 1 })}</span
                        >
                        <input
                          type="number"
                          min="0"
                          step="1"
                          class="w-24 rounded border border-input bg-surface px-2 py-1 text-right text-xs tabular-nums"
                          data-testid={`${kind}-bound-${index}`}
                          value={bounds[index] ?? 0}
                          onchange={(event) => updateBound(kind, index, event.currentTarget.value)}
                          aria-label={t('tables.boundAria', {
                            kind: kind === 'word' ? t('table.word') : t('table.char'),
                            index: index + 1,
                          })}
                        />
                      </label>
                    {/each}
                  </div>
                  <p class="text-[11px] text-muted-foreground">
                    {t('tables.group7Note', { count: formatCount(bounds[bounds.length - 1] ?? 0) })}
                  </p>
                  <Button variant="outline" size="sm" class="self-start" onclick={() => void solveFromPct()}>
                    {t('tables.solveFromPct')}
                  </Button>
                </div>
              {/each}
            </div>
          {/if}

          <Separator />

          <!-- 实时预览 -->
          <div class="flex flex-col gap-2">
            <div class="flex flex-wrap items-center gap-2">
              <span class="text-xs font-medium">{t('tables.previewTitle')}</span>
              <span class="text-[11px] text-muted-foreground">{t('tables.previewHint')}</span>
            </div>

            {#each previews as preview (preview.kind)}
              <div class="flex flex-col gap-2">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="text-xs font-medium"
                    >{preview.kind === 'word' ? t('tables.wordPreview') : t('tables.charPreview')}</span
                  >
                  {#if preview.warning || preview.capNote}
                    <span
                      class="rounded border border-amber-500/40 px-1.5 py-0.5 text-[11px] text-amber-600 dark:text-amber-400"
                      data-testid={`warning-${preview.kind}`}
                    >
                      {[
                        preview.warning ? t(preview.warning.key, preview.warning.params) : '',
                        preview.capNote,
                      ]
                        .filter(Boolean)
                        .join(' ')}
                    </span>
                  {/if}
                </div>
                <div class="scrollbar-thin overflow-auto rounded-lg border border-border">
                  <table class="w-full border-collapse text-xs" data-testid={`preview-${preview.kind}`}>
                    <thead class="bg-surface-muted/60 text-muted-foreground">
                      <tr>
                        <th class="px-3 py-2 text-left font-medium">{t('tables.preview.col.name')}</th>
                        <th class="px-3 py-2 text-right font-medium">{t('tables.preview.col.range')}</th>
                        <th class="px-3 py-2 text-right font-medium">{t('tables.preview.col.entries')}</th>
                        <th class="px-3 py-2 text-right font-medium">{t('tables.preview.col.coverage')}</th>
                        <th class="px-3 py-2 text-right font-medium">{t('tables.preview.col.cumulative')}</th>
                        <th class="px-3 py-2 text-right font-medium">{t('tables.preview.col.source')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {#each preview.rows as row (row.index)}
                        {@const palette = paletteForTierKey(tierKeyAt(row.index, keys))}
                        <tr class="border-t border-border/70" data-preview-row={row.name}>
                          <td class="px-3 py-2">
                            <span class="inline-flex items-center gap-1.5">
                              <span
                                class="inline-block size-3 shrink-0 rounded-[3px] border"
                                style={swatchStyle(palette, dark)}
                              ></span>
                              <span class="font-medium">{row.name}</span>
                              {#if method === 'top_pct'}
                                <span class="text-[11px] text-muted-foreground">
                                  {formatTopPercent(pctBounds[row.index] ?? 0)}
                                </span>
                              {/if}
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
            <span class="text-xs font-medium">{t('tables.legendTitle')}</span>
            <TierLegend
              names={names}
              keys={keys}
              bounds={activeBoundsInfo('word', meta).bounds}
              showUnknown={false}
            />
            <p class="text-[11px] text-muted-foreground">{t('tables.applyNote')}</p>
          </div>
  </SectionCard>
    {/if}
  {/if}
</div>
