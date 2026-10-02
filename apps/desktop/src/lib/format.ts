/**
 * 展示层的格式化小工具（体量、数字、百分比、时间）。
 *
 * 全部是纯函数，页面里直接 import 使用，避免每个页面各写一份 toFixed。
 */

import {
  isTierMethod,
  TIER_BOUND_COUNT,
  TIER_COUNT,
  type Meta,
  type Settings,
  type TierCurve,
  type TierMethod,
} from './types';

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
  return Math.round(n).toLocaleString('zh-CN');
}

/** 千分位小数（用于词条数这类可能是浮点的场景） */
export function formatNumber(n: number | null | undefined, digits = 0): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '—';
  return n.toLocaleString('zh-CN', { minimumFractionDigits: digits, maximumFractionDigits: digits });
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
 * 语料库词表动辄 20 万条，rank 1 的「前 0.0005%」与 rank 200 的「前 0.1%」
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

/** 毫秒 → 「1 分 23 秒」/「820 毫秒」 */
export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return '—';
  if (ms < 1000) return `${Math.round(ms)} 毫秒`;
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60) return `${totalSeconds.toFixed(1)} 秒`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.round(totalSeconds % 60);
  if (minutes < 60) return `${minutes} 分 ${seconds} 秒`;
  const hours = Math.floor(minutes / 60);
  return `${hours} 小时 ${minutes % 60} 分`;
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
  if (!Number.isFinite(maxRank) || maxRank >= 1e18) return '以上全部';
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

/** 从 meta 里找到某张表（默认全库词表） */
export function findTable<T extends { path: string; kind: string }>(
  tables: T[],
  kind: 'word' | 'char' = 'word',
  prefix = 'full/'
): T | undefined {
  return (
    tables.find((table) => table.path === `${prefix}${kind}`) ??
    tables.find((table) => table.path.endsWith(`/${kind}`))
  );
}

/** 取某张表的分组阈值；表不存在时回落到任意一张同 kind 的表 */
export function tiersOfTable(
  tables: { path: string; kind: string; tiers: { name: string; max_rank: number }[] }[],
  kind: 'word' | 'char' = 'word'
): { name: string; max_rank: number }[] {
  const table = findTable(tables, kind);
  if (table) return table.tiers;
  const fallback = tables.find((t) => t.kind === kind);
  return fallback?.tiers ?? [];
}

/** 分组名 → 该组的阈值（找不到返回 null） */
export function tierMaxRank(
  tables: { path: string; kind: string; tiers: { name: string; max_rank: number }[] }[],
  tierName: string | null,
  kind: 'word' | 'char' = 'word'
): number | null {
  if (!tierName) return null;
  const found = tiersOfTable(tables, kind).find((tier) => tier.name === tierName);
  return found ? found.max_rank : null;
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
// 分组阈值（表开关 + 分组自定义的**唯一权威实现**）
//
// 后端返回的 `token.tier` / `word_hit.tier` 一律按 meta 里的默认阈值算，
// 用户自定义阈值只在设置里。所以「这个排名属于第几组」必须由这里统一回答，
// 所有渲染分组的地方（TokenChips / TokenDetail / TierLegend / 排行榜）都走
// `tierIndexFor`，不要再去读 token.tier。
//
// 用户的「启用表」设置只影响分域对比与排行榜（后端 analyze_text 里过滤），
// 与分组阈值无关，因此本区块不读 enabledTables。
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

/** 用户没填 / 填得不合法时的兜底：取 meta 里全库表的默认阈值 */
function defaultBounds(meta: Meta, kind: 'word' | 'char'): number[] {
  const tiers = tiersOfTable(meta.tables, kind);
  const out: number[] = [];
  for (let i = 0; i < TIER_BOUND_COUNT; i += 1) {
    const rank = tiers[i]?.max_rank;
    // 后端缺这一档时用「上一档 ×10」外推，保证严格递增
    out.push(
      Number.isFinite(rank) && rank >= 1
        ? Math.round(rank)
        : Math.max(1, Math.round((out[i - 1] ?? 1) * 10))
    );
  }
  return out;
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
 *   - `rank`：`settings.tierWordBounds ??` meta 里全库表的默认阈值（字表同理）；
 *   - `coverage`：把 `settings.tierCoverage` 的 6 个目标经 `curves` 反解成排名上界
 *     （在 log10(rank) 上插值）；
 *   - `even`：`[1..6].map(k => round(entries * k / 7))`，`entries` 优先取
 *     `tablePath` 指向的那张表，其次取该 kind 的全库表；
 *   - 三种方式都必须得到**严格递增**的 6 个整数，否则回退到默认阈值
 *     （`boundsInfo` 会同时给出警告文案，界面据此提示）。
 */
export function effectiveBounds(
  kind: 'word' | 'char',
  meta: Meta,
  settings: Settings,
  curves?: Record<string, TierCurve>,
  tablePath?: string
): number[] {
  return boundsInfo(kind, meta, settings, curves, tablePath).bounds;
}

export type BoundsInfo = {
  bounds: number[];
  /** 非空 = 用户填的值不合法，已回退到默认阈值 */
  warning: string;
};

/** `effectiveBounds` 的完整形态：顺带告诉界面有没有回退 */
export function boundsInfo(
  kind: 'word' | 'char',
  meta: Meta,
  settings: Settings,
  curves?: Record<string, TierCurve>,
  tablePath?: string
): BoundsInfo {
  const fallback = defaultBounds(meta, kind);
  const method: TierMethod = isTierMethod(settings.tierMethod) ? settings.tierMethod : 'rank';
  const table =
    (tablePath ? meta.tables.find((entry) => entry.path === tablePath) : undefined) ??
    findTable(meta.tables, kind);

  if (method === 'rank') {
    const custom = settings[kind === 'word' ? 'tierWordBounds' : 'tierCharBounds'];
    if (!custom) return { bounds: fallback, warning: '' };
    const ok = ascendingInts(custom);
    if (ok) return { bounds: ok, warning: '' };
    return {
      bounds: fallback,
      warning: `${kind === 'word' ? '词表' : '字表'}阈值必须是非负整数、严格递增、共 ${TIER_BOUND_COUNT} 个，已临时回退到 meta 默认值。`,
    };
  }

  if (method === 'even') {
    const entries = table?.entries ?? 0;
    if (!(entries > 0)) {
      return {
        bounds: fallback,
        warning: '这张表的词条数是 0，无法按词条数等分，已回退到 meta 默认值。',
      };
    }
    const even = Array.from({ length: TIER_BOUND_COUNT }, (_, i) =>
      Math.max(1, Math.round((entries * (i + 1)) / TIER_COUNT))
    );
    const ok = ascendingInts(even);
    if (ok) return { bounds: ok, warning: '' };
    return { bounds: fallback, warning: '按词条数等分的结果不合法（词条数太少），已回退到 meta 默认值。' };
  }

  // coverage
  const targets = ascendingCoverage(settings.tierCoverage) ?? ascendingCoverage(defaultCoverageTargets(meta, kind));
  if (!targets) {
    return {
      bounds: fallback,
      warning: '覆盖率目标必须严格递增、且都在 0~100% 之间，已回退到 meta 默认值。',
    };
  }
  const curve = curves?.[table?.path ?? ''];
  const points = curve?.points ?? [];
  if (points.length < 2) {
    return { bounds: fallback, warning: '还没有拿到覆盖率曲线，正在用 meta 默认阈值显示。' };
  }
  const solved = targets.map((target) => rankForCoverage(points, target) ?? 0);
  const ok = ascendingInts(solved);
  if (ok) return { bounds: ok, warning: '' };
  return {
    bounds: fallback,
    warning: '这组覆盖率目标反解不出严格递增的排名阈值（目标太接近或超出曲线范围），已回退到 meta 默认值。',
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
  tablePath?: string
): number | null {
  if (!meta || !settings) return null;
  return tierIndexFromBounds(rank, effectiveBounds(kind, meta, settings, curves, tablePath));
}

