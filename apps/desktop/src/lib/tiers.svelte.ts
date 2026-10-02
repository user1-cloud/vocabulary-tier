/**
 * 分组设置 / 表开关的共享状态（Svelte 5 runes，模块级 `.svelte.ts` 是官方推荐的写法）。
 *
 * 为什么要有这个模块：
 *   - 分组自定义（tierMethod / tierWordBounds / tierCharBounds / tierCoverage）与
 *     表开关（enabledTables）都在设置里，但**渲染分组的地方有三个页面 + 两个组件**
 *     （划句分析、排行榜、悬浮小窗、TokenChips、TokenTip、TierLegend）。
 *     每个页面各拉一次 `get_settings` 会持有各自的一份副本，改完一处别处不刷新。
 *   - 所以设置只在这里存一份：App.svelte 启动时 `loadTierSettings()` 一次，
 *     「表管理」页改完直接写这份 state，其它页面因为读了同一份响应式状态会自动重算。
 *
 * 这里只放**状态**，纯计算（effectiveBounds / tierIndexFor / rankForCoverage）都在
 * `format.ts` 里，方便单测与非组件代码复用。
 */

import { defaultSettings, getSettings, setSettings, tierCurve, type Result } from './api/bridge';
import { boundsInfo, effectiveBounds, tierIndexFor } from './format';
import type { Meta, Settings, TierCurve, TierMethod } from './types';

/** 全局设置（只关心里面与分组 / 表开关有关的那几个字段） */
export const appSettings = $state<{ value: Settings }>({ value: defaultSettings() });

/** 设置是否已经从后端加载过一次 */
export const settingsReady = $state<{ value: boolean }>({ value: false });

/**
 * 覆盖率曲线缓存：`表路径 → TierCurve`。
 *
 * Rust 侧也按产物缓存，这里再缓存一层是为了：
 *   1. 切换方法 / 改数字时不必反复走 IPC；
 *   2. `effectiveBounds(..., curves)` 是同步函数，必须能立刻拿到曲线。
 */
export const tierCurves = $state<Record<string, TierCurve>>({});

const pendingCurves = new Map<string, Promise<void>>();

/** 当前分组方法（非法值一律当 `'rank'`） */
export function tierMethod(): TierMethod {
  const method = appSettings.value.tierMethod;
  return method === 'coverage' || method === 'even' ? method : 'rank';
}

/** 某张表是否启用（未设置 = 全部启用） */
export function tableEnabled(path: string): boolean {
  const enabled = appSettings.value.enabledTables;
  if (!enabled || enabled.length === 0) return true;
  return enabled.includes(path);
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
  for (const key of ['enabledTables', 'tierMethod', 'tierWordBounds', 'tierCharBounds', 'tierCoverage'] as const) {
    if (authoritative.includes(key)) continue;
    if (key === 'enabledTables') merged.enabledTables = appSettings.value.enabledTables;
    else if (key === 'tierMethod') merged.tierMethod = appSettings.value.tierMethod;
    else if (key === 'tierWordBounds') merged.tierWordBounds = appSettings.value.tierWordBounds;
    else if (key === 'tierCharBounds') merged.tierCharBounds = appSettings.value.tierCharBounds;
    else merged.tierCoverage = appSettings.value.tierCoverage;
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
  tablePath?: string
): number[] {
  if (!meta) return [];
  return effectiveBounds(kind, meta, appSettings.value, tierCurves, tablePath);
}

/** 生效阈值 + 回退警告 */
export function activeBoundsInfo(
  kind: 'word' | 'char',
  meta: Meta | null | undefined,
  tablePath?: string
): { bounds: number[]; warning: string } {
  if (!meta) return { bounds: [], warning: '' };
  return boundsInfo(kind, meta, appSettings.value, tierCurves, tablePath);
}

/** 排名 → 组号（0..6；未收录返回 null） */
export function activeTierIndex(
  kind: 'word' | 'char',
  rank: number | null | undefined,
  meta: Meta | null | undefined,
  tablePath?: string
): number | null {
  if (!meta) return null;
  return tierIndexFor(kind, rank, meta, appSettings.value, tierCurves, tablePath);
}

/** 组名表：优先 meta.tier_names，缺省用标准七组 */
export function tierNamesOf(meta: Meta | null | undefined): string[] {
  const names = meta?.tier_names ?? [];
  if (names.length > 0) return names;
  return ['极多', '很多', '较多', '中等', '较少', '很少', '极少'];
}

/** 组号 → 组名 */
export function tierNameAt(index: number | null, names: string[]): string | null {
  if (index === null || index < 0 || index >= names.length) return null;
  return names[index];
}
