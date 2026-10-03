/**
 * 展示层的格式化小工具（体量、数字、百分比、时间）。
 *
 * ## 纯度约定（接入 i18n 后新增，别破坏）
 *
 *   - **计算**类函数保持纯函数：`boundsInfo` / `effectiveBounds` / `rankForCoverage` /
 *     `tierIndexFromBounds` / `tiersOfTable` … 一律**不读界面语言、不调 `t()`**。
 *     它们被大量 `$derived` 调用、也被非组件代码复用；一旦读全局状态，返回值就会随
 *     语言变化，缓存与等价性判断全部失效。需要提示用户时返回**错误码**
 *     （`BoundsInfo.warning` 就是 `{ key, params }`），由展示层去 `t()`。
 *   - **格式化**类函数（`formatDuration` / `formatMaxRank`）属于展示层，可以读 `t()`：
 *     它们是响应式读点，切换语言会自动重算。
 *
 * 千分位与小数点分隔符在 `zh-CN` 与 `en` 下完全一致，所以数字格式化统一走
 * `./number-locale`（唯一一处常量），换语言时要动的地方只有一个。
 */

import {
  defaultTierPct,
  FULL_SCOPE,
  isTierMethod,
  TIER_BOUND_COUNT,
  TIER_COUNT,
  type BoundsWarning,
  type Meta,
  type Settings,
  type TableMeta,
  type TierCurve,
  type TierMethod,
} from './types';
import { t } from './i18n.svelte';
import { formatCount, NUMBER_LOCALE } from './number-locale';

/** 1024 进制体积；自动选单位 */
export function formatBytes(bytes: number | null | undefined, digits = 1): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return '—';
  const n = Math.max(0, bytes);
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB', 'PB'];
  let value = n / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i += 1;
  }
  return `${value.toFixed(value >= 100 ? 0 : digits)} ${units[i]}`;
}

/** 千分位整数 */
export function formatInt(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '—';
  return formatCount(n);
}

/** 千分位小数（用于词条数这类可能是浮点的场景） */
export function formatNumber(n: number | null | undefined, digits = 0): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '—';
  return n.toLocaleString(NUMBER_LOCALE, { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

/**
 * 比例 → 百分比字符串。
 * Rust 侧 `coverage` 是 0..1，`pct` 是 0..100，需要调用方指明。
 */
export function formatRatio(
  value: number | null | undefined,
  opts: { digits?: number; scale?: 'ratio' | 'percent' } = {}
): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—';
  const digits = opts.digits ?? 2;
  const scaled = opts.scale === 'percent' ? value : value * 100;
  return `${scaled.toFixed(digits)}%`;
}

/**
 * pct 的自适应精度：大数少位、小数多位。
 * 排行榜里 0.0001% 和 4.2% 都要能看清楚。
 */
export function formatPct(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—';
  const abs = Math.abs(value);
  if (abs === 0) return '0%';
  if (abs >= 1) return `${value.toFixed(2)}%`;
  if (abs >= 0.01) return `${value.toFixed(3)}%`;
  return `${value.toFixed(5)}%`;
}

/**
 * 「前 N%」的自适应精度：排名越靠前，需要的有效位数越多。
 *
 * 语料库词频表动辄 20 万条，rank 1 的「前 0.0005%」与 rank 200 的「前 0.1%」
 * 都要看得出量级差别，固定两位小数会把前三万名全压成「0.00%」。
 */
export function formatTopPercent(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—';
  if (value <= 0) return '0%';
  // 有效数字 3 位；再粗就分不出头部排名，再细在小窗里放不下。
  // 用 toFixed 而不是 toPrecision：后者对 <1e-6 的值会退化成科学计数法（"5.00e-7%"）。
  const digits =
    value >= 100 ? 0
    : value >= 10 ? 1
    : value >= 1 ? 2
    : value >= 0.01 ? 3
    : value >= 0.001 ? 4
    : value >= 0.0001 ? 5
    : value >= 0.00001 ? 6
    : 7;
  return `${formatNumber(value, digits)}%`;
}

/** 毫秒 → 「1 分 23 秒」/「820 毫秒」（展示层，随界面语言变） */
export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return '—';
  if (ms < 1000) return t('format.durationMs', { value: Math.round(ms) });
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60) return t('format.durationSec', { value: totalSeconds.toFixed(1) });
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.round(totalSeconds % 60);
  if (minutes < 60) return t('format.durationMinSec', { minutes, seconds });
  const hours = Math.floor(minutes / 60);
  return t('format.durationHourMin', { hours, minutes: minutes % 60 });
}

/** "2024-05-01T09:30:00Z" → "2024-05-01 09:30:00"（不做时区换算，避免引入依赖） */
export function formatTimestamp(iso: string | null | undefined): string {
  if (!iso) return '—';
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})/.exec(iso);
  if (!m) return iso;
  return `${m[1]}-${m[2]}-${m[3]} ${m[4]}:${m[5]}:${m[6]} UTC`;
}

/** 把 u64::MAX 之类的哨兵值显示成「∞ / 以上」 */
export function formatMaxRank(maxRank: number): string {
  if (!Number.isFinite(maxRank) || maxRank >= 1e18) return t('format.allAbove');
  return formatInt(maxRank);
}

/** 进度百分比夹取到 0..100 */
export function clampPercent(value: number | null | undefined): number {
  if (value === null || value === undefined || !Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, value));
}

// ---------------------------------------------------------------------------
// 结果表辅助（阈值一律从 meta 里读，绝不硬编码分组边界）
// ---------------------------------------------------------------------------

/** 从 meta 里找到某张表（默认主表组的词频表） */
export function findTable<T extends { path: string; kind: string }>(
  tables: T[],
  kind: 'word' | 'char' = 'word',
  scope = FULL_SCOPE
): T | undefined {
  return (
    tables.find((table) => table.path === scope && table.kind === kind) ??
    tables.find((table) => table.kind === kind)
  );
}

/** 取某张表的分组阈值；表不存在时回落到任意一张同 kind 的表 */
export function tiersOfTable(
  tables: { path: string; kind: string; tiers: { name: string; max_rank: number }[] }[],
  kind: 'word' | 'char' = 'word',
  scope?: string
): { name: string; max_rank: number }[] {
  const table = scope
    ? tables.find((t) => t.path === scope && t.kind === kind)
    : findTable(tables, kind);
  if (table) return table.tiers;
  const fallback = tables.find((t) => t.kind === kind);
  return fallback?.tiers ?? [];
}

/**
 * 前% → 排名上界（含）。与 Rust 侧 `rank::rank_for_pct` **必须等价**（都取 `ceil`）。
 *
 * 取整方向不能改成 `floor`：判前%要拿边界条目自己的前%去比，`ceil` 才不会把边界
 * 那一条推到下一档；`floor` 还会在条目少时把六档压成同一个排名。
 *
 * 空表返回 0 —— 调用方要能看出"这张表没有条目，别拿百分比切它"。
 */
export function rankForPct(pct: number, entries: number): number {
  if (!(entries > 0) || !Number.isFinite(pct) || pct <= 0) return 0;
  return Math.max(1, Math.ceil((pct / 100) * entries));
}

/** 排名 → 前%（0..100）。`entries` 是**条目数**，不是 token 数。 */
export function pctForRank(rank: number, entries: number): number {
  if (!(entries > 0) || !Number.isFinite(rank)) return 0;
  return (rank * 100) / entries;
}

/**
 * 把 6 个前%上界换算成严格递增的 6 个排名上界。
 *
 * 与 Rust 侧 `rank::ranks_from_pct` 等价。空表返回 6 个 1（**不能返回空数组**：
 * 缺了前 6 个上界，七组就只剩"以上全部"一组，组号到配色的映射会整体错位）。
 */
export function ranksFromPct(pcts: readonly number[], entries: number): number[] {
  if (!(entries > 0)) return pcts.map(() => 1);
  const out: number[] = [];
  for (const p of pcts) {
    let r = rankForPct(p, entries);
    const prev = out[out.length - 1];
    if (prev !== undefined && r <= prev) r = prev + 1;
    out.push(r);
  }
  return out;
}

/** 某张表实际生效的 6 个前%上界（产物里没记就用该类型的默认口径） */
export function tierPctOfTable(
  table: { kind: string; tier_pct?: number[] } | undefined,
  kind: 'word' | 'char'
): number[] {
  const pct = table?.tier_pct;
  if (pct && pct.length === TIER_BOUND_COUNT) return [...pct];
  return defaultTierPct(kind);
}

/**
 * 组号 → 该组的排名上界（越界 / 没有这张表时返回 null）。
 *
 * 按**组号**取，不按组名取：组名是文案（将来会随界面语言变化），不能当身份。
 */
export function tierMaxRank(
  tables: { path: string; kind: string; tiers: { name: string; max_rank: number }[] }[],
  tierIndex: number | null,
  kind: 'word' | 'char' = 'word'
): number | null {
  if (tierIndex === null || !Number.isInteger(tierIndex) || tierIndex < 0) return null;
  const tiers = tiersOfTable(tables, kind);
  return tiers[tierIndex]?.max_rank ?? null;
}

/**
 * 把排名换算成分组下标（0..n-1）。
 * 这是 `crates/vocfreq-core/src/rank.rs::tier_of` 的前端等价实现，
 * 只在「客户端按组筛选」等前端场景使用，不作为权威来源。
 */
export function tierIndexOfRank(
  rank: number,
  tiers: { name: string; max_rank: number }[]
): number {
  for (let i = 0; i < tiers.length; i += 1) {
    if (rank <= tiers[i].max_rank) return i;
  }
  return Math.max(0, tiers.length - 1);
}

// ---------------------------------------------------------------------------
// 分组阈值（主表组 + 分组自定义的**唯一权威实现**）
//
// 后端返回的 `token.tier` / `word_hit.tier` 一律按 meta 里的默认阈值算，
// 用户自定义阈值只在设置里。所以「这个排名属于第几组」必须由这里统一回答，
// 所有渲染分组的地方（TokenChips / TokenDetail / TierLegend / 排行榜）都走
// `tierIndexFor`，不要再去读 token.tier。
//
// 阈值取自**主表组**那张表：铺平之后任意表组都能当主表，写死 `full/word`
// 会让颜色与详情面板里的前%对不上。`tableKey` 参数不给时由调用方
// （`tiers.svelte.ts::primaryTableKey`）补上。
// ---------------------------------------------------------------------------

/** 曲线里 log10 插值的浮点误差容限（与 Rust 侧 rank_for_coverage 保持一致） */
const COVERAGE_EPS = 1e-12;

function isFiniteInt(value: number): boolean {
  return Number.isFinite(value) && value >= 0;
}

/** 6 个排名上界必须都是非负整数且严格递增（用户可能填重复值） */
function ascendingInts(values: readonly number[] | null | undefined): number[] | null {
  if (!values || values.length !== TIER_BOUND_COUNT) return null;
  const out: number[] = [];
  for (const value of values) {
    if (!isFiniteInt(value)) return null;
    out.push(Math.round(value));
  }
  for (let i = 1; i < out.length; i += 1) {
    if (out[i] <= out[i - 1]) return null;
  }
  return out;
}

/** 6 个覆盖率目标必须都在 (0,1] 且严格递增 */
function ascendingCoverage(values: readonly number[] | null | undefined): number[] | null {
  if (!values || values.length !== TIER_BOUND_COUNT) return null;
  const out: number[] = [];
  for (const value of values) {
    if (!Number.isFinite(value) || value <= 0 || value > 1) return null;
    out.push(value);
  }
  for (let i = 1; i < out.length; i += 1) {
    if (out[i] <= out[i - 1]) return null;
  }
  return out;
}

/** 6 个前%上界必须都在 (0,100) 且严格递增 */
function ascendingPct(values: readonly number[] | null | undefined): number[] | null {
  if (!values || values.length !== TIER_BOUND_COUNT) return null;
  const out: number[] = [];
  for (const value of values) {
    if (!Number.isFinite(value) || value <= 0 || value >= 100) return null;
    out.push(value);
  }
  for (let i = 1; i < out.length; i += 1) {
    if (out[i] <= out[i - 1]) return null;
  }
  return out;
}

/**
 * 兜底阈值：**该表自己的前%口径**换算出来的排名上界。
 *
 * 不能拿"别的表的绝对阈值"当兜底 —— 那些数字是为另一张表的规模校准的，
 * 套到这张表上会让整片词条挤进同一档。
 */
function defaultBounds(meta: Meta, kind: 'word' | 'char', tableKey?: string): number[] {
  const table = resolveTable(meta, kind, tableKey);
  return tableBounds(table, kind);
}

/** 用一张具体表算兜底阈值（产物里没记的前%上界就用该类型的默认口径） */
export function tableBounds(
  table: { kind: string; entries?: number; tier_pct?: number[] } | undefined,
  kind: 'word' | 'char'
): number[] {
  return ranksFromPct(tierPctOfTable(table, kind), table?.entries ?? 0);
}

/** 把 6 个上界补成 7 个 `{name, max_rank}`（第 7 组 = 以上全部） */
export function tierBandsFromBounds(
  bounds: number[],
  names: string[]
): { name: string; max_rank: number }[] {
  return names.map((name, index) => ({
    name,
    max_rank: index < bounds.length ? bounds[index] : Number.POSITIVE_INFINITY,
  }));
}

/** 排名 → 组号（0..6）；未收录（rank 为空/非法）返回 null */
export function tierIndexFromBounds(rank: number | null | undefined, bounds: number[]): number | null {
  if (rank === null || rank === undefined || !Number.isFinite(rank) || rank < 1) return null;
  for (let i = 0; i < bounds.length; i += 1) {
    if (rank <= bounds[i]) return i;
  }
  return bounds.length; // 落在最后一个上界之后 = 第 7 组（以上全部）
}

/**
 * 覆盖率 → 排名阈值。
 *
 * 在 `log10(rank)` 上线性插值：曲线本身就是对数采样的，按排名线性插值会在头部
 * 产生很大误差（1→2 名与 100000→100001 名的覆盖率跨度完全不同）。
 * 与 Rust 侧 `vocfreq_core::query::rank_for_coverage` 是等价实现。
 */
export function rankForCoverage(points: [number, number][], target: number): number | null {
  if (!points || points.length === 0) return null;
  if (target <= points[0][1]) return points[0][0];
  for (let i = 0; i + 1 < points.length; i += 1) {
    const [r0, c0] = points[i];
    const [r1, c1] = points[i + 1];
    if (target <= c1) {
      if (Math.abs(c1 - c0) < COVERAGE_EPS) return r1;
      const t = (target - c0) / (c1 - c0);
      const lr = Math.log10(r0) + t * (Math.log10(r1) - Math.log10(r0));
      return Math.max(1, Math.round(Math.pow(10, lr)));
    }
  }
  return points[points.length - 1][0];
}

/** 从 meta 的 tier_stats 推出 6 个默认累计覆盖率目标（覆盖率模式的兜底起点） */
export function defaultCoverageTargets(meta: Meta, kind: 'word' | 'char'): number[] {
  const table = findTable(meta.tables, kind);
  const stats = table?.tier_stats ?? [];
  const isBlank = (value: number) => !Number.isFinite(value) || value <= 0;
  if (stats.length >= TIER_COUNT && stats.some((stat) => !isBlank(stat.cumulative))) {
    return stats.slice(0, TIER_BOUND_COUNT).map((stat) => {
      const value = Number.isFinite(stat.cumulative) ? stat.cumulative : 1;
      return Math.min(1, Math.max(1e-6, value));
    });
  }
  // 没有分档统计时给一组常见的 Zipf 覆盖率曲线
  return [0.35, 0.5, 0.62, 0.72, 0.82, 0.92];
}

/**
 * 算出某张表实际生效的 6 个排名上界（第 7 组为无穷）。
 *
 *   - `top_pct`（**默认**）：`settings.tierPct ??` 产物里那张表的 `tier_pct`，
 *     再由**该表的条目数**换算成排名上界。这是唯一与表规模无关的口径；
 *   - `rank`：`settings.tierWordBounds ??` meta 里默认表的绝对阈值（字表同理）；
 *   - `coverage`：把 `settings.tierCoverage` 的 6 个目标经 `curves` 反解成排名上界
 *     （在 log10(rank) 上插值）；
 *   - `even`：`[1..6].map(k => round(entries * k / 7))`；
 *   - 三种方式都必须得到**严格递增**的 6 个整数，否则回退到默认阈值
 *     （`boundsInfo` 会同时给出警告文案，界面据此提示）。
 */
export function effectiveBounds(
  kind: 'word' | 'char',
  meta: Meta,
  settings: Settings,
  curves?: Record<string, TierCurve>,
  tableKey?: string
): number[] {
  return boundsInfo(kind, meta, settings, curves, tableKey).bounds;
}

export type BoundsInfo = {
  bounds: number[];
  /**
   * 非 null = 用户填的值不合法，已回退到默认阈值。
   *
   * 是**错误码**（`{ key, params }`）而不是文案：本函数保持纯计算，不读界面语言。
   * 展示层写 `t(warning.key, warning.params)`。
   */
  warning: BoundsWarning | null;
};

/** 按表身份（`表组/类型`）取那张 `TableMeta`；给不出来就回落该 kind 的第一张 */
function resolveTable(
  meta: Meta,
  kind: 'word' | 'char',
  tableKey?: string
): TableMeta | undefined {
  if (tableKey) {
    const hit = meta.tables.find(
      (entry) => tableKeyOf(entry) === tableKey || entry.path === tableKey
    );
    if (hit) return hit;
  }
  return findTable(meta.tables, kind);
}

/** `TableMeta` → 表身份字符串 */
export function tableKeyOf(table: { path: string; kind: string }): string {
  return `${table.path}/${table.kind}`;
}

/** `effectiveBounds` 的完整形态：顺带告诉界面有没有回退 */
export function boundsInfo(
  kind: 'word' | 'char',
  meta: Meta,
  settings: Settings,
  curves?: Record<string, TierCurve>,
  tableKey?: string
): BoundsInfo {
  const fallback = defaultBounds(meta, kind, tableKey);
  // 老设置里可能存着 `rank`；它仍然可用（绝对排名口径没删掉），只是不再是默认。
  const method: TierMethod = isTierMethod(settings.tierMethod) ? settings.tierMethod : 'top_pct';
  const table = resolveTable(meta, kind, tableKey);

  if (method === 'top_pct') {
    const entries = table?.entries ?? 0;
    const custom = settings.tierPct;
    // 优先级：用户填的 → 产物里这张表记的 → 该类型的默认口径
    const pct = ascendingPct(custom) ?? tierPctOfTable(table, kind);
    const solved = ranksFromPct(pct, entries);
    const ok = ascendingInts(solved);
    if (ok) return { bounds: ok, warning: null };
    if (custom && !ascendingPct(custom)) {
      return { bounds: fallback, warning: { key: 'bounds.invalidPct', params: { count: TIER_BOUND_COUNT } } };
    }
    return { bounds: fallback, warning: null };
  }

  if (method === 'rank') {
    const custom = settings[kind === 'word' ? 'tierWordBounds' : 'tierCharBounds'];
    if (!custom) return { bounds: fallback, warning: null };
    const ok = ascendingInts(custom);
    if (ok) return { bounds: ok, warning: null };
    return {
      bounds: fallback,
      warning: {
        key: kind === 'word' ? 'bounds.invalidWord' : 'bounds.invalidChar',
        params: { count: TIER_BOUND_COUNT },
      },
    };
  }

  if (method === 'even') {
    const entries = table?.entries ?? 0;
    if (!(entries > 0)) {
      return {
        bounds: fallback,
        warning: { key: 'bounds.evenNoEntries' },
      };
    }
    const even = Array.from({ length: TIER_BOUND_COUNT }, (_, i) =>
      Math.max(1, Math.round((entries * (i + 1)) / TIER_COUNT))
    );
    const ok = ascendingInts(even);
    if (ok) return { bounds: ok, warning: null };
    return { bounds: fallback, warning: { key: 'bounds.evenInvalid' } };
  }

  // coverage
  const targets =
    ascendingCoverage(settings.tierCoverage) ?? ascendingCoverage(defaultCoverageTargets(meta, kind));
  if (!targets) {
    return {
      bounds: fallback,
      warning: { key: 'bounds.coverageInvalid' },
    };
  }
  const curve = curves?.[table ? tableKeyOf(table) : ''];
  const points = curve?.points ?? [];
  if (points.length < 2) {
    return { bounds: fallback, warning: { key: 'bounds.coverageNoCurve' } };
  }
  const solved = targets.map((target) => rankForCoverage(points, target) ?? 0);
  const ok = ascendingInts(solved);
  if (ok) return { bounds: ok, warning: null };
  return {
    bounds: fallback,
    warning: { key: 'bounds.coverageUnsolvable' },
  };
}

/**
 * 由 rank 定组号 0..6；rank 为空 / 未收录返回 null。
 *
 * 只依赖字符串与数字，不是一个响应式读点，所以可以放心在 `$derived` 里调用。
 */
export function tierIndexFor(
  kind: 'word' | 'char',
  rank: number | null | undefined,
  meta: Meta | null | undefined,
  settings: Settings | null | undefined,
  curves?: Record<string, TierCurve>,
  tableKey?: string
): number | null {
  if (!meta || !settings) return null;
  return tierIndexFromBounds(rank, effectiveBounds(kind, meta, settings, curves, tableKey));
}

