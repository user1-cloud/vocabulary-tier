/**
 * 分组设置 / 主作用域的共享状态（Svelte 5 runes，模块级 `.svelte.ts` 是官方推荐的写法）。
 *
 * 为什么要有这个模块：
 *   - 分组自定义（tierMethod / tierPct / tierWordBounds / tierCharBounds / tierCoverage）
 *     与**主作用域**（primaryScope）都在设置里，但渲染分组的地方有三个页面 + 两个组件
 *     （划句分析、排行榜、悬浮小窗、TokenChips、TokenDetail、TierLegend）。
 *     每个页面各拉一次 `get_settings` 会持有各自的一份副本，改完一处别处不刷新。
 *   - 所以设置只在这里存一份：App.svelte 启动时 `loadTierSettings()` 一次，
 *     「表管理」页改完直接写这份 state，其它页面因为读了同一份响应式状态会自动重算。
 *
 * 这里只放**状态**，纯计算（effectiveBounds / tierIndexFor / rankForCoverage）都在
 * `format.ts` 里，方便单测与非组件代码复用。
 */

import { defaultSettings, getSettings, setSettings, tierCurve, type Result } from './api/bridge';
import { boundsInfo, effectiveBounds, tableKeyOf, tierIndexFor } from './format';
import { tierLabels } from './i18n.svelte';
import {
  FULL_SCOPE,
  type BoundsWarning,
  type Meta,
  type Settings,
  type TierCurve,
  type TierMethod,
} from './types';

/** 全局设置（只关心里面与分组 / 主作用域有关的那几个字段） */
export const appSettings = $state<{ value: Settings }>({ value: defaultSettings() });

/** 设置是否已经从后端加载过一次 */
export const settingsReady = $state<{ value: boolean }>({ value: false });

/**
 * 覆盖率曲线缓存：`表身份 → TierCurve`。
 *
 * Rust 侧也按产物缓存，这里再缓存一层是为了：
 *   1. 切换方法 / 改数字时不必反复走 IPC；
 *   2. `effectiveBounds(..., curves)` 是同步函数，必须能立刻拿到曲线。
 */
export const tierCurves = $state<Record<string, TierCurve>>({});

const pendingCurves = new Map<string, Promise<void>>();

/** 当前分组方法（非法值一律当 `'top_pct'`，那是默认口径） */
export function tierMethod(): TierMethod {
  const method = appSettings.value.tierMethod;
  return method === 'rank' || method === 'coverage' || method === 'even' ? method : 'top_pct';
}

/**
 * **主作用域名**（不是 `作用域/类型`）。
 *
 * 设置里没填就从产物里挑：`full` 优先，其次第一张有词表的作用域。
 * 划句分析、排行榜、分组预览都必须以它为准 —— 否则同一句话在两个页面会是两种颜色。
 */
export function primaryScope(meta: Meta | null | undefined): string {
  const wanted = appSettings.value.primaryScope;
  const scopes = meta ? [...new Set(meta.tables.map((t) => t.path))] : [];
  if (wanted && (scopes.length === 0 || scopes.includes(wanted))) return wanted;
  if (scopes.includes(FULL_SCOPE)) return FULL_SCOPE;
  return scopes[0] ?? FULL_SCOPE;
}

/**
 * 主作用域里某一类的**表身份**（`作用域/类型`），用于取阈值、曲线与查表。
 *
 * 主作用域缺这一类时（相加只加了词表就会出现）回落到该类的第一张表 —— 宁可换个
 * 来源也不能让整句变成「未收录」。
 */
export function primaryTableKey(meta: Meta | null | undefined, kind: 'word' | 'char'): string {
  if (!meta) return `${FULL_SCOPE}/${kind}`;
  const scope = primaryScope(meta);
  const exact = meta.tables.find((t) => t.path === scope && t.kind === kind);
  if (exact) return tableKeyOf(exact);
  const fallback = meta.tables.find((t) => t.kind === kind);
  return fallback ? tableKeyOf(fallback) : `${scope}/${kind}`;
}

/** 目标覆盖率（用户没填时由调用方按表决定兜底） */
export function coverageTargets(): number[] | null {
  return appSettings.value.tierCoverage;
}

/** 从后端读一次设置（App.svelte 启动时调用） */
export async function loadTierSettings(): Promise<Result<Settings>> {
  const res = await getSettings();
  if (res.ok) {
    appSettings.value = res.data;
    settingsReady.value = true;
  }
  return res;
}

/**
 * 局部更新设置：读当前值 → 合并 → 保存 → 写回共享状态。
 *
 * 「表管理」页的所有操作都走这里，因此不会覆盖掉设置页里的其它字段。
 */
export async function updateSettings(part: Partial<Settings>): Promise<Result<Settings>> {
  const merged: Settings = { ...appSettings.value, ...part };
  const res = await setSettings(merged);
  if (res.ok) appSettings.value = res.data;
  return res;
}

/**
 * 保存一份完整设置，但**分组 / 表开关这几个字段以共享状态为准**。
 *
 * 设置页可以改分组字段（它自己也能改），所以调用方通过 `authoritative` 明确声明
 * 哪些字段是它刚编辑过的；其余字段（例如用户在「表管理」页刚改过的 enabledTables）
 * 用共享状态里的值覆盖，避免两个页面互相把对方的修改冲掉。
 */
export async function saveSettingsRespectingTierState(
  next: Settings,
  authoritative: (keyof Settings)[] = []
): Promise<Result<Settings>> {
  const merged: Settings = { ...next };
  // 这几个字段「表管理」页与设置页都能改，谁刚改过谁说了算（`authoritative`）。
  // `enabledTables` 已废弃，但**仍要同步**：老设置文件里可能还有值，让它在两个
  // 页面之间保持一致，免得一次保存把它翻来覆去地改。
  for (const key of [
    'primaryScope',
    'enabledTables',
    'tierMethod',
    'tierWordBounds',
    'tierCharBounds',
    'tierCoverage',
    'tierPct',
  ] as const) {
    if (authoritative.includes(key)) continue;
    merged[key] = appSettings.value[key] as never;
  }
  const res = await setSettings(merged);
  if (res.ok) appSettings.value = res.data;
  return res;
}

/** 把设置页保存后的结果同步回共享状态 */
export function adoptSettings(next: Settings): void {
  appSettings.value = next;
  settingsReady.value = true;
}

/** 拿某张表的覆盖率曲线（带缓存与去重；失败时抛错给调用方展示） */
export async function ensureCurve(
  path: string,
  dir?: string | null,
  maxPoints = 600
): Promise<Result<TierCurve>> {
  const cached = tierCurves[path];
  if (cached) return { ok: true, data: cached };

  const inflight = pendingCurves.get(path);
  if (inflight) {
    await inflight;
    const after = tierCurves[path];
    return after
      ? { ok: true, data: after }
      : { ok: false, error: '覆盖率曲线加载失败。' };
  }

  const task = tierCurve(path, dir, maxPoints).then((res) => {
    if (res.ok) tierCurves[path] = res.data;
  });
  pendingCurves.set(path, task);
  try {
    await task;
  } finally {
    pendingCurves.delete(path);
  }

  const fresh = tierCurves[path];
  if (fresh) return { ok: true, data: fresh };
  return { ok: false, error: '覆盖率曲线加载失败。' };
}

// ---------------------------------------------------------------------------
// 给页面用的只读派生函数（都是纯函数，读上面的 $state）
// ---------------------------------------------------------------------------

/** 生效的 6 个排名上界 */
export function activeBounds(
  kind: 'word' | 'char',
  meta: Meta | null | undefined,
  tableKey?: string
): number[] {
  if (!meta) return [];
  return effectiveBounds(kind, meta, appSettings.value, tierCurves, tableKey ?? primaryTableKey(meta, kind));
}

/**
 * 生效阈值 + 回退警告。
 *
 * `warning` 是**错误码**（`{ key, params }`，见 `types.ts::BoundsWarning`），
 * 展示层用 `t(warning.key, warning.params)` 渲染 —— 计算层不碰界面语言。
 */
export function activeBoundsInfo(
  kind: 'word' | 'char',
  meta: Meta | null | undefined,
  tableKey?: string
): { bounds: number[]; warning: BoundsWarning | null } {
  if (!meta) return { bounds: [], warning: null };
  return boundsInfo(kind, meta, appSettings.value, tierCurves, tableKey ?? primaryTableKey(meta, kind));
}

/** 排名 → 组号（0..6；未收录返回 null） */
export function activeTierIndex(
  kind: 'word' | 'char',
  rank: number | null | undefined,
  meta: Meta | null | undefined,
  tableKey?: string
): number | null {
  if (!meta) return null;
  return tierIndexFor(
    kind,
    rank,
    meta,
    appSettings.value,
    tierCurves,
    tableKey ?? primaryTableKey(meta, kind)
  );
}

/**
 * 组名表（展示用）—— **界面分组标签的唯一接缝**。
 *
 * 实现已挪到 `i18n.svelte.ts::tierLabels()`：那里按「组号 → 产物自带的稳定标识
 * `tier_keys` → `tier.<key>` 本地化文案」逐级解析，查不到才回落到产物自带的
 * `meta.tier_names`（中文）与标准七组。
 *
 * 也就是说：**接入新语言不需要改任何页面** —— 消息表里补上 `tier.*` 七条，
 * 全站的分组标签、图例、排行榜筛选一起变。前提是它们都从这里取标签，
 * 而不是各自去读 `meta.tier_names`。
 *
 * ⚠️ 返回值是**文案**：不要拿它当身份。判断分组、索引、配色一律用组号
 * （`token.tier` / `activeTierIndex()`）或稳定标识（`tier-colors.ts::tierKeyAt()`）。
 */
export function tierNamesOf(meta: Meta | null | undefined): string[] {
  return tierLabels(meta);
}

/** 组号 → 组名（展示用；越界返回 null） */
export function tierNameAt(index: number | null, names: string[]): string | null {
  if (index === null || index < 0 || index >= names.length) return null;
  return names[index];
}
