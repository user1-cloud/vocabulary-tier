/**
 * Tauri 运行时桥接层 —— 前端唯一的 Tauri 入口。
 *
 * 规则（工程要求 2）：
 *   - 所有 `invoke` / `listen` / `emit` 只出现在本文件里；
 *     页面组件不得直接 import `@tauri-apps/api`。
 *   - 每个命令都包成 `Result<T>`，UI 层不用到处 try/catch。
 *   - `isTauri()` 为 false（`vite dev` 或直接打开 dist）时走**内存 mock**，
 *     页面能完整走通流程，方便在浏览器里检查 UI。mock 数据只在本文件里。
 *
 * 冻结的命令接口见任务说明；字段名与 Rust 侧 serde 结构保持一致。
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { inDictFromFlags } from './../tier-colors';
import { isCjkChar, isPunctuationOrSpace, normalizeToken } from './../segments';
import type {
  AppInfo,
  CorpusPlan,
  DatasetStatus,
  Meta,
  RankRow,
  ScanParams,
  ScanProgress,
  Settings,
  TableMeta,
  TierCurve,
  TierStat,
  TokenInfo,
  WordHit,
} from './../types';
export type Result<T> = { ok: true; data: T } | { ok: false; error: string };

/** 是否运行在 Tauri WebView 中（浏览器里打开 dist 时为 false） */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** 扫描事件名 */
export const SCAN_PROGRESS_EVENT = 'scan:progress';
export const SCAN_DONE_EVENT = 'scan:done';
export const SCAN_ERROR_EVENT = 'scan:error';
/** 悬浮小窗 → 主窗口的文本回传事件（Rust 侧无需改动：走 Tauri 自带事件总线） */
export const POPUP_TEXT_EVENT = 'voctier:popup-text';

const NO_TAURI_HINT = '当前不在桌面端运行，这个功能需要 VocTier 桌面应用。';

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

function toMessage(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === 'object') {
    try {
      return JSON.stringify(error);
    } catch {
      return String(error);
    }
  }
  return String(error);
}

/** 统一包装：把 invoke 的 reject 转成 Result.false */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<Result<T>> {
  if (!isTauri()) {
    return { ok: false, error: NO_TAURI_HINT };
  }
  try {
    const data = await invoke<T>(command, args);
    return { ok: true, data };
  } catch (error) {
    // Rust 侧的错误是中文 &str，Tauri 会原样传过来
    return { ok: false, error: toMessage(error) };
  }
}

// ---------------------------------------------------------------------------
// 应用信息
// ---------------------------------------------------------------------------

export async function appInfo(): Promise<Result<AppInfo>> {
  if (!isTauri()) {
    return { ok: true, data: { ...MOCK.APP_INFO } };
  }
  const res = await call<Partial<AppInfo>>('app_info');
  if (!res.ok) return res;
  // core_version 是后来加进冻结接口的字段，老后端可能没有 → 兜底成「未知」
  const raw = res.data ?? {};
  return {
    ok: true,
    data: {
      name: raw.name ?? 'VocTier',
      version: raw.version ?? '0.0.0',
      tauri_version: raw.tauri_version ?? '未知',
      core_version: raw.core_version ?? '未知',
    },
  };
}

// ---------------------------------------------------------------------------
// 语料库探测
// ---------------------------------------------------------------------------

export async function planCorpus(corpus: string): Promise<Result<CorpusPlan>> {
  if (!isTauri()) return { ok: true, data: MOCK.planCorpus(corpus) };
  return call<CorpusPlan>('plan_corpus', { corpus });
}

// ---------------------------------------------------------------------------
// 产物状态
// ---------------------------------------------------------------------------

export async function datasetStatus(dir: string | null | undefined): Promise<Result<DatasetStatus>> {
  const target = dir ?? '';
  if (!isTauri()) return { ok: true, data: MOCK.datasetStatus(target) };
  const res = await call<DatasetStatus>('dataset_status', { dir: target });
  if (!res.ok) return res;
  // 后端可能只回 { dir, meta }，用 meta 反推 exists，容错一下
  const data = res.data;
  return {
    ok: true,
    data: {
      dir: data?.dir ?? target,
      exists: typeof data?.exists === 'boolean' ? data.exists : !!data?.meta,
      meta: data?.meta ?? null,
    },
  };
}

// ---------------------------------------------------------------------------
// 划句分析
// ---------------------------------------------------------------------------

export async function analyzeText(
  text: string,
  domains: string[] = [],
  dir?: string | null
): Promise<Result<TokenInfo[]>> {
  if (!isTauri()) return { ok: true, data: MOCK.analyzeText(text, domains) };
  const res = await call<unknown>('analyze_text', { text, domains, dir: dir ?? null });
  if (!res.ok) return res;
  return { ok: true, data: normalizeTokenList(res.data) };
}

/**
 * 后端按冻结接口应返回 `TokenInfo[]`；这里兼容 `{ tokens: [...] }` 的包法，
 * 并逐条补齐可选字段，避免界面因为 undefined 崩掉。
 */
function normalizeTokenList(raw: unknown): TokenInfo[] {
  const list: unknown = Array.isArray(raw)
    ? raw
    : raw && typeof raw === 'object' && Array.isArray((raw as { tokens?: unknown }).tokens)
      ? (raw as { tokens: unknown[] }).tokens
      : [];
  return (list as Partial<TokenInfo>[]).map((item) => normalizeToken(item as Partial<TokenInfo> & { text: string }));
}

/**
 * 把已有产物目录装入后端缓存（同时重建分词器）。
 *
 * 按文档 §8.1：打开产物目录时后端必须按 `meta.tokenizer` 重建分词器，
 * 否则查出来的频次会系统性偏错。所以 dataset_status 说目录可用时，
 * 划句分析 / 排行榜应当先做这一步。
 *
 * 这个命令在冻结清单里标注为「可选」：如果 Rust 侧还没实现，会返回
 * 「command not found」之类的错误，界面把它当成「未装载」展示即可，
 * 不会崩。
 */
export async function openDataset(dir: string): Promise<Result<Meta>> {
  if (!isTauri()) {
    const status = MOCK.datasetStatus(dir);
    if (!status.meta) return { ok: false, error: '该目录不是有效的 VocTier 产物目录。' };
    return { ok: true, data: status.meta };
  }
  const res = await call<Meta>('open_dataset', { dir });
  if (!res.ok) return res;
  if (!res.data) return { ok: false, error: '后端没有返回产物元数据。' };
  return { ok: true, data: res.data };
}

// ---------------------------------------------------------------------------
// 覆盖率曲线（分组自定义用）
// ---------------------------------------------------------------------------

/**
 * 取某张表的累计覆盖率曲线：`points = [[rank, 累计覆盖率 0..1], …]`。
 *
 * Rust 侧按产物缓存，重复调用不会重算。前端拿它把「目标覆盖率」反解成
 * 「排名上界」（`format.ts::rankForCoverage`）。
 */
export async function tierCurve(
  path: string,
  dir?: string | null,
  maxPoints?: number | null
): Promise<Result<TierCurve>> {
  if (!isTauri()) return { ok: true, data: MOCK.tierCurve(path, maxPoints ?? 600) };
  return call<TierCurve>('tier_curve', {
    path,
    dir: dir ?? null,
    maxPoints: maxPoints ?? null,
  });
}

// ---------------------------------------------------------------------------
// 查词 / 排行榜
// ---------------------------------------------------------------------------
export async function lookupWord(
  word: string,
  kind: 'word' | 'char' = 'word',
  dir?: string | null
): Promise<Result<WordHit>> {
  if (!isTauri()) {
    const hit = MOCK.lookupWord(word, kind);
    if (!hit) return { ok: false, error: `语料库中未收录「${word}」` };
    return { ok: true, data: hit };
  }
  // `Option<WordHit>`：未收录时后端可能返回 null（而不是 Err）
  const res = await call<WordHit | null>('lookup_word', { word, kind, dir: dir ?? null });
  if (!res.ok) return res;
  if (!res.data) return { ok: false, error: `语料库中未收录「${word}」` };
  return { ok: true, data: res.data };
}

export async function listRank(
  domain: string | null,
  kind: 'word' | 'char',
  from: number,
  limit: number,
  dir?: string | null
): Promise<Result<RankRow[]>> {
  if (!isTauri()) return { ok: true, data: MOCK.listRank(domain, kind, from, limit) };
  return call<RankRow[]>('list_rank', { domain, kind, from, limit, dir: dir ?? null });
}

export async function searchWords(
  query: string,
  kind: 'word' | 'char',
  limit: number,
  domain?: string | null,
  dir?: string | null
): Promise<Result<RankRow[]>> {
  if (!isTauri()) return { ok: true, data: MOCK.searchWords(query, kind, limit) };
  return call<RankRow[]>('search_words', {
    query,
    kind,
    domain: domain ?? null,
    limit,
    dir: dir ?? null,
  });
}

// ---------------------------------------------------------------------------
// 扫描任务（长任务 + 事件推送）
// ---------------------------------------------------------------------------

export async function startScan(params: ScanParams): Promise<Result<null>> {
  if (!isTauri()) {
    MOCK.startScan(params);
    return { ok: true, data: null };
  }
  return call<null>('start_scan', { params });
}

export async function cancelScan(): Promise<Result<null>> {
  if (!isTauri()) {
    MOCK.cancelScan();
    return { ok: true, data: null };
  }
  return call<null>('cancel_scan');
}

/**
 * 订阅扫描进度。返回取消订阅函数（**同步**返回，组件里直接
 * `$effect(() => onScanProgress(...))` 即可，effect 的返回值会被当作清理函数）。
 */
export function onScanProgress(handler: (payload: ScanProgress) => void): () => void {
  if (!isTauri()) return MOCK.onProgress(handler);
  let disposed = false;
  let unlisten: UnlistenFn | null = null;
  void listen<ScanProgress>(SCAN_PROGRESS_EVENT, (event) => handler(event.payload)).then((off) => {
    if (disposed) off();
    else unlisten = off;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** 监听扫描完成（payload = Meta） */
export function onScanDone(handler: (meta: Meta) => void): () => void {
  if (!isTauri()) return MOCK.onDone(handler);
  return bridgeListen<Meta>(SCAN_DONE_EVENT, handler);
}

/** 监听扫描出错（payload = 中文错误串） */
export function onScanError(handler: (message: string) => void): () => void {
  if (!isTauri()) return MOCK.onError(handler);
  return bridgeListen<string>(SCAN_ERROR_EVENT, handler);
}

function bridgeListen<T>(eventName: string, handler: (payload: T) => void): () => void {
  let disposed = false;
  let unlisten: UnlistenFn | null = null;
  void listen<T>(eventName, (event) => handler(event.payload)).then((off) => {
    if (disposed) off();
    else unlisten = off;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

// ---------------------------------------------------------------------------
// 设置
// ---------------------------------------------------------------------------

export async function getSettings(): Promise<Result<Settings>> {
  if (!isTauri()) return { ok: true, data: { ...MOCK.SETTINGS } };
  const res = await call<Partial<Settings>>('get_settings');
  if (!res.ok) return res;
  return { ok: true, data: { ...defaultSettings(), ...res.data } };
}

export async function setSettings(settings: Settings): Promise<Result<Settings>> {
  if (!isTauri()) {
    MOCK.SETTINGS = { ...settings };
    return { ok: true, data: { ...settings } };
  }
  const res = await call<Settings>('set_settings', { settings });
  if (!res.ok) return res;
  return { ok: true, data: { ...settings, ...res.data } };
}

/** 前端兜底默认值（后端字段缺失时用），与 Rust 侧默认值保持一致 */
export function defaultSettings(): Settings {
  return {
    corpusDir: null,
    dataDir: null,
    hotkey: 'Alt+Q',
    popupWidth: 420,
    popupHeight: 320,
    popupOpacity: 0.96,
    popupAlwaysOnTop: true,
    popupAutoCloseMs: 0,
    theme: 'system',
    threads: 0,
    hmm: true,
    keepDigit: true,
    keepLatin: true,
    skipSingleChar: false,
    userDict: null,
    minCount: 1,
    skipDomainTables: false,
    // 表管理与分组自定义：null / 'rank' 表示「一切照 meta 默认」，行为与旧版本完全一致
    enabledTables: null,
    tierMethod: 'rank',
    tierWordBounds: null,
    tierCharBounds: null,
    tierCoverage: null,
  };
}

/** 局部更新设置：把字段并进当前磁盘上的值，避免只需要改几个字段时覆盖掉别的 */
export async function patchSettings(part: Partial<Settings>): Promise<Result<Settings>> {
  const current = await getSettings();
  if (!current.ok) return current;
  return setSettings({ ...current.data, ...part });
}

// ---------------------------------------------------------------------------
// 全局取词 / 悬浮小窗
// ---------------------------------------------------------------------------

/** 手动触发一次全局取词；空串表示当前无选区 */
export async function captureSelection(): Promise<Result<string>> {
  if (!isTauri()) return { ok: true, data: '' };
  return call<string>('capture_selection');
}

/** 打开 / 显示悬浮小窗并填入文本 */
export async function openPopup(text: string): Promise<Result<null>> {
  if (!isTauri()) return { ok: false, error: '浏览器预览模式下无法打开悬浮小窗。' };
  return call<null>('open_popup', { text });
}

/** 小窗启动时取走待分析文本 */
export async function takePendingSelection(): Promise<Result<string>> {
  if (!isTauri()) return { ok: true, data: MOCK.takePendingSelection() };
  return call<string>('take_pending_selection');
}

/**
 * 小窗 → 主窗口的文本回传。
 *
 * 冻结接口里没有「发回主窗口」的命令，而 `open_popup` 的语义是
 * 「打开小窗并填入文本」，用它会自己给自己发。所以这里用 Tauri 自带的
 * 事件总线广播 `voctier:popup-text`，主窗口 `listen` 同一事件即可，
 * Rust 侧不需要新增任何命令。
 */
export async function emitPopupText(text: string): Promise<Result<null>> {
  if (!isTauri()) return { ok: false, error: '浏览器预览模式下没有主窗口可回传。' };
  try {
    const { emit } = await import('@tauri-apps/api/event');
    await emit(POPUP_TEXT_EVENT, { text });
    return { ok: true, data: null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/** 主窗口订阅小窗回传的文本 */
export function onPopupText(handler: (text: string) => void): () => void {
  if (!isTauri()) return () => {};
  return bridgeListen<{ text: string }>(POPUP_TEXT_EVENT, (payload) => handler(payload?.text ?? ''));
}

// ---------------------------------------------------------------------------
// 剪贴板（复制按钮用；navigator.clipboard 在 WebView 里可用，失败时兜底）
// ---------------------------------------------------------------------------

export async function copyText(text: string): Promise<Result<null>> {
  try {
    if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return { ok: true, data: null };
    }
    return { ok: false, error: '当前环境不支持剪贴板写入。' };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

// ---------------------------------------------------------------------------
// 桌面端选择器（dialog 插件）—— 同样只在本文件里 import 插件
// ---------------------------------------------------------------------------

/** 选目录；用户取消返回 null */
export async function pickDirectory(title = '选择目录'): Promise<Result<string | null>> {
  if (!isTauri()) return { ok: false, error: NO_TAURI_HINT };
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open({ directory: true, multiple: false, title });
    return { ok: true, data: typeof picked === 'string' ? picked : null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/** 选文件；用户取消返回 null */
export async function pickFile(title = '选择文件'): Promise<Result<string | null>> {
  if (!isTauri()) return { ok: false, error: NO_TAURI_HINT };
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open({ directory: false, multiple: false, title });
    return { ok: true, data: typeof picked === 'string' ? picked : null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/** 用系统默认程序打开路径 / URL（opener 插件） */
export async function openExternal(target: string): Promise<Result<null>> {
  if (!isTauri()) return { ok: false, error: NO_TAURI_HINT };
  try {
    const { openUrl, openPath, revealItemInDir } = await import('@tauri-apps/plugin-opener');
    if (/^https?:\/\//i.test(target)) {
      await openUrl(target);
    } else if (await isDirectory(target)) {
      await revealItemInDir(target);
    } else {
      await openPath(target);
    }
    return { ok: true, data: null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/** 目录里带不带扩展名，粗略判断（真实判断交给 Rust 侧） */
async function isDirectory(target: string): Promise<boolean> {
  return !/\.[A-Za-z0-9]{1,6}$/.test(target);
}

// ===========================================================================
// 浏览器 mock —— 只在 isTauri() === false 时使用
// ===========================================================================

type MockEntry = { rank: number; word: string; count: number; tier: number };

/**
 * 演示用的词条表。按档位递减排列，分组与 crates/vocfreq-core/src/rank.rs
 * 的默认词表阈值（500 / 3000 / 10000 / 30000 / 80000 / 200000）对应。
 */
const MOCK_WORDS: MockEntry[] = [
  { rank: 1, word: '的', count: 412306, tier: 0 },
  { rank: 2, word: '了', count: 203871, tier: 0 },
  { rank: 3, word: '我们', count: 141220, tier: 0 },
  { rank: 4, word: '在', count: 138004, tier: 0 },
  { rank: 5, word: '是', count: 132758, tier: 0 },
  { rank: 6, word: '他', count: 101332, tier: 0 },
  { rank: 7, word: '有', count: 98221, tier: 0 },
  { rank: 8, word: '就', count: 87003, tier: 0 },
  { rank: 9, word: '这个', count: 72115, tier: 0 },
  { rank: 10, word: '中国', count: 61240, tier: 0 },
  { rank: 11, word: '不', count: 58820, tier: 0 },
  { rank: 12, word: '人', count: 57110, tier: 0 },
  { rank: 13, word: '都', count: 55230, tier: 0 },
  { rank: 14, word: '一个', count: 51402, tier: 0 },
  { rank: 15, word: '上', count: 49881, tier: 0 },
  { rank: 16, word: '也', count: 48220, tier: 0 },
  { rank: 17, word: '很', count: 46115, tier: 0 },
  { rank: 18, word: '到', count: 44390, tier: 0 },
  { rank: 19, word: '说', count: 42771, tier: 0 },
  { rank: 20, word: '要', count: 41008, tier: 0 },
  { rank: 21, word: '还可以', count: 38210, tier: 1 },
  { rank: 22, word: '工作', count: 36411, tier: 1 },
  { rank: 23, word: '问题', count: 35290, tier: 1 },
  { rank: 24, word: '时间', count: 33412, tier: 1 },
  { rank: 25, word: '没有', count: 32018, tier: 1 },
  { rank: 26, word: '生活', count: 30220, tier: 1 },
  { rank: 27, word: '我们国家', count: 29013, tier: 1 },
  { rank: 28, word: '发展', count: 27715, tier: 1 },
  { rank: 29, word: '经济', count: 26301, tier: 1 },
  { rank: 30, word: '社会', count: 25117, tier: 1 },
  { rank: 31, word: '技术', count: 24102, tier: 2 },
  { rank: 32, word: '语言', count: 23244, tier: 2 },
  { rank: 33, word: '研究', count: 22118, tier: 2 },
  { rank: 34, word: '数据', count: 21330, tier: 2 },
  { rank: 35, word: '统计', count: 20419, tier: 2 },
  { rank: 36, word: '分析', count: 19820, tier: 2 },
  { rank: 37, word: '文本', count: 18711, tier: 2 },
  { rank: 38, word: '词汇', count: 17603, tier: 2 },
  { rank: 39, word: '频率', count: 16422, tier: 2 },
  { rank: 40, word: '语料库', count: 15330, tier: 2 },
  { rank: 41, word: '分词', count: 11720, tier: 3 },
  { rank: 42, word: '覆盖率', count: 10814, tier: 3 },
  { rank: 43, word: '阈值', count: 9912, tier: 3 },
  { rank: 44, word: '排名', count: 9203, tier: 3 },
  { rank: 45, word: '字表', count: 8411, tier: 3 },
  { rank: 46, word: '词表', count: 7712, tier: 3 },
  { rank: 47, word: '未收录', count: 4312, tier: 4 },
  { rank: 48, word: '虚词', count: 3910, tier: 4 },
  { rank: 49, word: '停顿', count: 3521, tier: 4 },
  { rank: 50, word: '歧义', count: 3120, tier: 4 },
  { rank: 51, word: '切分', count: 902, tier: 5 },
  { rank: 52, word: '熵值', count: 741, tier: 5 },
  { rank: 53, word: '低频词', count: 611, tier: 5 },
  { rank: 54, word: '生僻字', count: 302, tier: 6 },
  { rank: 55, word: '陨石', count: 204, tier: 6 },
  { rank: 56, word: '氤氲', count: 88, tier: 6 },
  { rank: 57, word: '饕餮', count: 41, tier: 6 },
];

const MOCK_CHARS: MockEntry[] = [
  { rank: 1, word: '的', count: 402118, tier: 0 },
  { rank: 2, word: '一', count: 188220, tier: 0 },
  { rank: 3, word: '是', count: 171004, tier: 0 },
  { rank: 4, word: '不', count: 162508, tier: 0 },
  { rank: 5, word: '了', count: 155332, tier: 0 },
  { rank: 6, word: '在', count: 143221, tier: 0 },
  { rank: 7, word: '人', count: 131003, tier: 0 },
  { rank: 8, word: '有', count: 122115, tier: 0 },
  { rank: 9, word: '我', count: 112240, tier: 0 },
  { rank: 10, word: '他', count: 101820, tier: 0 },
  { rank: 11, word: '这', count: 99110, tier: 1 },
  { rank: 12, word: '个', count: 95230, tier: 1 },
  { rank: 13, word: '们', count: 91402, tier: 1 },
  { rank: 14, word: '中', count: 89881, tier: 1 },
  { rank: 15, word: '来', count: 88220, tier: 1 },
  { rank: 16, word: '上', count: 86115, tier: 1 },
  { rank: 17, word: '大', count: 84390, tier: 1 },
  { rank: 18, word: '为', count: 82771, tier: 1 },
  { rank: 19, word: '和', count: 81008, tier: 1 },
  { rank: 20, word: '国', count: 78210, tier: 1 },
  { rank: 21, word: '地', count: 66411, tier: 2 },
  { rank: 22, word: '到', count: 65290, tier: 2 },
  { rank: 23, word: '以', count: 63412, tier: 2 },
  { rank: 24, word: '说', count: 62018, tier: 2 },
  { rank: 25, word: '时', count: 60220, tier: 2 },
  { rank: 26, word: '要', count: 59013, tier: 2 },
  { rank: 27, word: '就', count: 57715, tier: 2 },
  { rank: 28, word: '出', count: 56301, tier: 2 },
  { rank: 29, word: '会', count: 55117, tier: 2 },
  { rank: 30, word: '可', count: 54102, tier: 2 },
  { rank: 31, word: '也', count: 44244, tier: 3 },
  { rank: 32, word: '你', count: 43118, tier: 3 },
  { rank: 33, word: '对', count: 42330, tier: 3 },
  { rank: 34, word: '生', count: 41419, tier: 3 },
  { rank: 35, word: '能', count: 40820, tier: 3 },
  { rank: 36, word: '而', count: 39711, tier: 3 },
  { rank: 37, word: '子', count: 38603, tier: 3 },
  { rank: 38, word: '那', count: 37422, tier: 3 },
  { rank: 39, word: '得', count: 36330, tier: 3 },
  { rank: 40, word: '于', count: 35720, tier: 3 },
  { rank: 41, word: '着', count: 30814, tier: 4 },
  { rank: 42, word: '下', count: 29912, tier: 4 },
  { rank: 43, word: '自', count: 29203, tier: 4 },
  { rank: 44, word: '之', count: 28411, tier: 4 },
  { rank: 45, word: '年', count: 27712, tier: 4 },
  { rank: 46, word: '过', count: 26312, tier: 4 },
  { rank: 47, word: '发', count: 25910, tier: 4 },
  { rank: 48, word: '后', count: 25321, tier: 4 },
  { rank: 49, word: '作', count: 24320, tier: 4 },
  { rank: 50, word: '里', count: 23902, tier: 4 },
  { rank: 51, word: '熵', count: 812, tier: 5 },
  { rank: 52, word: '阈', count: 611, tier: 5 },
  { rank: 53, word: '氲', count: 502, tier: 5 },
  { rank: 54, word: '饕', count: 304, tier: 6 },
  { rank: 55, word: '餮', count: 204, tier: 6 },
  { rank: 56, word: '龘', count: 88, tier: 6 },
  { rank: 57, word: '氤', count: 41, tier: 6 },
];

/**
 * 演示用的分域。真实产物是「全库 + 7 个分域」，这里也放 7 个，
 * 这样「表管理」页在浏览器里能完整看到 2 × 8 = 16 张表。
 */
const MOCK_DOMAIN_NAMES = ['blog', 'book', 'forum', 'gov', 'news', 'parallel', 'wiki'];

/** 分域的中文说明（只用于 planCorpus 的展示，不影响表路径） */
const MOCK_DOMAIN_LABELS: Record<string, string> = {
  blog: '博客',
  book: '图书',
  forum: '论坛',
  gov: '政府',
  news: '新闻',
  parallel: '平行语料',
  wiki: '百科',
};

/**
 * 演示数据的分组阈值 —— 与 Rust 侧 `vocfreq_core::rank::default_word_tiers()`
 * / `default_char_tiers()` 保持一致（词表 100/1000/5000/20000/50000/150000，
 * 字表 50/200/600/1500/3000/5000）。
 */
const MOCK_WORD_BOUNDS = [100, 1_000, 5_000, 20_000, 50_000, 150_000];
const MOCK_CHAR_BOUNDS = [50, 200, 600, 1_500, 3_000, 5_000];

function mockTiers(kind: 'word' | 'char' = 'word'): { name: string; max_rank: number }[] {
  const names = ['极多', '很多', '较多', '中等', '较少', '很少', '极少'];
  const bounds = kind === 'char' ? MOCK_CHAR_BOUNDS : MOCK_WORD_BOUNDS;
  return names.map((name, i) => ({
    name,
    max_rank: i < bounds.length ? bounds[i] : Number.MAX_SAFE_INTEGER,
  }));
}

/**
 * 演示数据的覆盖率口径。
 *
 * 真实语料里默认阈值下的七组覆盖率是「头重脚轻」的一条递减序列（词表实测约
 * 45% / 15% / 13% / 8% / 5% / 3% / 1%）。演示数据只有 57 条词，按 token 数硬摊
 * 会让最后一组吃掉 38%（所有低频词都挤在「极少」），「按覆盖率分组」的默认起点
 * 就没意义了。所以这里按几何衰减 `0.5 × 0.8^i` 造一条递减曲线 —— 归一化后是
 * 30% / 24% / 19% / 15% / 12% / 10% / 8%，前几组之和约 87%，与真实形状同类。
 */
function mockCoverageShares(): number[] {
  const raw = Array.from({ length: 7 }, (_, i) => 0.5 * Math.pow(0.8, i));
  const total = raw.reduce((sum, value) => sum + value, 0);
  return raw.map((value) => value / total);
}

function mockTierStats(entries: MockEntry[], kind: 'word' | 'char'): TierStat[] {
  const tiers = mockTiers(kind);
  const shares = mockCoverageShares();

  const stats: TierStat[] = tiers.map((tier, i) => ({
    name: tier.name,
    max_rank: tier.max_rank,
    entries: 0,
    tokens: 0,
    coverage: shares[i],
    cumulative: shares.slice(0, i + 1).reduce((sum, value) => sum + value, 0),
  }));

  for (const entry of entries) {
    stats[entry.tier].entries += 1;
    stats[entry.tier].tokens += entry.count;
  }
  return stats;
}

/** 给分域造一份「顺序不同、部分缺档」的排名，模拟真实语料的分域差异 */
function domainEntries(base: MockEntry[], domain: string, kind: 'word' | 'char'): MockEntry[] {
  const seed = domain.length * 31 + (kind === 'word' ? 7 : 13) + base.length;
  const rotated = base.map((_, i) => base[(i * 7 + seed) % base.length]);
  const unique: MockEntry[] = [];
  const seen = new Set<string>();
  for (const entry of rotated) {
    if (seen.has(entry.word)) continue;
    seen.add(entry.word);
    unique.push(entry);
  }
  // 让每个域都缺掉几个词，制造「该域未收录」的展示效果
  const drop = domain === 'book' ? 3 : domain === 'forum' ? 5 : domain === 'wiki' ? 7 : 0;
  const kept = unique.slice(drop);
  return kept.map((entry, i) => ({ ...entry, rank: i + 1 }));
}

function mockTableMeta(
  path: string,
  kind: string,
  entries: MockEntry[],
  stats: TierStat[]
): TableMeta {
  const totalTokens = entries.reduce((sum, e) => sum + e.count, 0);
  return {
    path,
    kind,
    entries: entries.length,
    total_tokens: totalTokens,
    vfr_bytes: entries.length * 34 + 4096,
    tiers: mockTiers(kind === 'char' ? 'char' : 'word'),
    tier_stats: stats,
  };
}

function buildMockMeta(corpusRoot: string): Meta {
  return {
    schema_version: 1,
    generated_at: '2024-05-01T09:30:00Z',
    tool_version: '0.1.0',
    corpus_root: corpusRoot,
    elapsed_ms: 184_230,
    tokenizer: {
      engine: 'jieba-rs',
      version: '0.7.4',
      hmm: true,
      dict: 'jieba 内置词典',
      user_dict: null,
      min_len: 1,
      max_len: 20,
      keep_latin: true,
      keep_digit: true,
      skip_single_char: false,
    },
    totals: {
      files: 1_284,
      bytes: 1_733_000_000,
      lines: 8_120_431,
      paras: 2_104_772,
      tokens: 191_404_112,
      bad_lines: 37,
    },
    domains: MOCK_DOMAIN_NAMES.map((name, i) => ({
      name,
      files: [812, 296, 176, 88, 412, 154, 233][i] ?? 100,
      bytes: [1_062_000_000, 402_000_000, 269_000_000, 61_000_000, 588_000_000, 130_000_000, 322_000_000][i] ?? 1_000_000,
    })),
    tables: [
      mockTableMeta('full/word', 'word', MOCK_WORDS, mockTierStats(MOCK_WORDS, 'word')),
      mockTableMeta('full/char', 'char', MOCK_CHARS, mockTierStats(MOCK_CHARS, 'char')),
      ...MOCK_DOMAIN_NAMES.flatMap((name) => {
        const words = domainEntries(MOCK_WORDS, name, 'word');
        const chars = domainEntries(MOCK_CHARS, name, 'char');
        return [
          mockTableMeta(`domains/${name}/word`, 'word', words, mockTierStats(words, 'word')),
          mockTableMeta(`domains/${name}/char`, 'char', chars, mockTierStats(chars, 'char')),
        ];
      }),
    ],
    tier_names: ['极多', '很多', '较多', '中等', '较少', '很少', '极少'],
  };
}

const MOCK_DIR = '(浏览器预览) 演示数据集';

/** 用小窗 / 划句页共用的「首次取词」模拟 */
let mockPendingTaken = false;

type ProgressHandler = (payload: ScanProgress) => void;
type DoneHandler = (meta: Meta) => void;
type ErrorHandler = (message: string) => void;

const mockProgressHandlers = new Set<ProgressHandler>();
const mockDoneHandlers = new Set<DoneHandler>();
const mockErrorHandlers = new Set<ErrorHandler>();
let mockTimers: ReturnType<typeof setTimeout>[] = [];

/** 演示用分词：内置若干常见词，贪心最长匹配，未命中的连续汉字合成 2 字块 */
const MOCK_SEGMENT_WORDS = [
  // 常用词（演示文本里出现的，尽量让 mock 分词结果接近真实 jieba）
  '可以',
  '帮助',
  '理解',
  '说明',
  '难度',
  '方面',
  '持续',
  '投入',
  '人工智能',
  '智能',
  '很快',
  '知识',
  '需要',
  '记录',
  '数据',
  '系统',
  '内容',
  '学习',
  '计算',
  '自然',
  '处理',
  ...'我们国家还可以语言统计数据文本词汇频率语料库分词覆盖率阈值排名字表词表未收录'.split(''),
  ...MOCK_WORDS.filter((w) => w.word.length > 1).map((w) => w.word),
];

function segmentForMock(text: string): string[] {
  const out: string[] = [];
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (/\s/.test(ch)) {
      let j = i;
      while (j < text.length && /\s/.test(text[j])) j += 1;
      out.push(text.slice(i, j));
      i = j;
      continue;
    }
    if (!isCjkChar(ch)) {
      // 数字 / 拉丁字母 / 标点：数字与字母连续成块，标点逐个
      let j = i;
      const isWordish = /[A-Za-z0-9]/.test(ch);
      while (j < text.length && (isWordish ? /[A-Za-z0-9]/.test(text[j]) : !isCjkChar(text[j]) && !/\s/.test(text[j]))) {
        j += 1;
      }
      out.push(text.slice(i, j));
      i = j;
      continue;
    }
    // 汉字：贪心最长匹配（最多 4 字）
    let matched = '';
    for (let len = 4; len >= 2; len -= 1) {
      const candidate = text.slice(i, i + len);
      if (candidate.length === len && MOCK_SEGMENT_WORDS.includes(candidate)) {
        matched = candidate;
        break;
      }
    }
    if (matched) {
      out.push(matched);
      i += matched.length;
    } else {
      out.push(ch);
      i += 1;
    }
  }
  return out;
}

function mockTokenize(text: string, domains: string[]): TokenInfo[] {
  const pieces = segmentForMock(text);
  const wordTiers = mockTiers('word');
  const charTiers = mockTiers('char');
  const domainFilter = domains.length > 0 ? domains : MOCK_DOMAIN_NAMES;
  const tokens: TokenInfo[] = [];
  let offset = 0;

  for (const piece of pieces) {
    const accepted = !isPunctuationOrSpace(piece);
    const singleCjk = [...piece].length === 1 && isCjkChar(piece);
    const entry = accepted
      ? (singleCjk ? MOCK_CHARS : MOCK_WORDS).find((w) => w.word === piece)
      : undefined;
    const totalTokens = singleCjk
      ? MOCK_CHARS.reduce((sum, e) => sum + e.count, 0)
      : MOCK_WORDS.reduce((sum, e) => sum + e.count, 0);

    const domainRanks: [string, number | null][] = [];
    if (accepted) {
      for (const name of domainFilter) {
        const source = singleCjk
          ? domainEntries(MOCK_CHARS, name, 'char')
          : domainEntries(MOCK_WORDS, name, 'word');
        const hit = source.find((w) => w.word === piece);
        domainRanks.push([name, hit ? hit.rank : null]);
      }
    }

    tokens.push({
      text: piece,
      byte_start: offset,
      byte_end: offset + new TextEncoder().encode(piece).length,
      accepted,
      single_cjk: singleCjk,
      table: accepted ? (singleCjk ? 'char' : 'word') : '',
      count: entry ? entry.count : null,
      rank: entry ? entry.rank : null,
      pct: entry ? (entry.count * 100) / totalTokens : null,
      tier: entry ? entry.tier : null,
      tier_name: entry ? (singleCjk ? charTiers : wordTiers)[entry.tier].name : null,
      in_dict: accepted ? true : null,
      from_user: accepted ? false : null,
      domain_ranks: domainRanks,
    });
    offset += new TextEncoder().encode(piece).length;
  }
  return tokens;
}

const MOCK = {
  APP_INFO: {
    name: 'VocTier',
    version: '0.1.0',
    tauri_version: '2.x',
    core_version: '0.1.0',
  } as AppInfo,

  SETTINGS: {
    ...({
      corpusDir: 'D:\\corpus',
      dataDir: MOCK_DIR,
      hotkey: 'Alt+Q',
      popupWidth: 420,
      popupHeight: 320,
      popupOpacity: 0.96,
      popupAlwaysOnTop: true,
      popupAutoCloseMs: 0,
      theme: 'system',
      threads: 0,
      hmm: true,
      keepDigit: true,
      keepLatin: true,
      skipSingleChar: false,
      userDict: null,
      minCount: 1,
      skipDomainTables: false,
      enabledTables: null,
      tierMethod: 'rank',
      tierWordBounds: null,
      tierCharBounds: null,
      tierCoverage: null,
    } satisfies Settings),
  } as Settings,

  planCorpus(corpus: string): CorpusPlan {
    const files = [812, 296, 176, 88, 412, 154, 233];
    const bytes = [1_062_000_000, 402_000_000, 269_000_000, 61_000_000, 588_000_000, 130_000_000, 322_000_000];
    const domains = MOCK_DOMAIN_NAMES.map((name, i) => ({
      name,
      files: files[i] ?? 100,
      bytes: bytes[i] ?? 1_000_000,
      rules: [`**/${name}/**/*.txt（${MOCK_DOMAIN_LABELS[name] ?? name}）`, `*.${name}.jsonl`],
    }));
    return {
      corpus,
      files: domains.reduce((sum, d) => sum + d.files, 0),
      bytes: domains.reduce((sum, d) => sum + d.bytes, 0),
      domains,
    };
  },

  datasetStatus(dir: string): DatasetStatus {
    return { dir: dir || MOCK_DIR, exists: true, meta: buildMockMeta(dir || 'D:\\corpus') };
  },

  analyzeText(text: string, domains: string[]): TokenInfo[] {
    if (!text.trim()) return [];
    return mockTokenize(text, domains);
  },

  lookupWord(word: string, kind: 'word' | 'char' = 'word'): WordHit | null {
    const tiers = mockTiers(kind);
    const base = kind === 'char' ? MOCK_CHARS : MOCK_WORDS;
    const entry = base.find((w) => w.word === word);
    if (!entry) return null;
    const total = base.reduce((sum, e) => sum + e.count, 0);
    return {
      word: entry.word,
      count: entry.count,
      rank: entry.rank,
      flags: 1,
      tier: entry.tier,
      tier_name: tiers[entry.tier].name,
      pct: (entry.count * 100) / total,
      in_dict: true,
    };
  },

  listRank(domain: string | null, kind: 'word' | 'char', from: number, limit: number): RankRow[] {
    const base = kind === 'char' ? MOCK_CHARS : MOCK_WORDS;
    const source = domain ? domainEntries(base, domain, kind) : base;
    return source
      .slice(Math.max(0, from - 1), Math.max(0, from - 1) + limit)
      .map((entry) => ({ rank: entry.rank, word: entry.word, count: entry.count, flags: 1 }));
  },

  /**
   * 演示用的覆盖率曲线：与 Rust 侧一样按对数间隔采样 `(rank, 累计覆盖率)`，
   * 且最后一点一定落在 `entries` 上（覆盖率 = 1），这样前端按点累加就能得到
   * 精确的累计覆盖率。
   */
  tierCurve(path: string, maxPoints: number): TierCurve {
    const meta = buildMockMeta('D:\\corpus');
    const table =
      meta.tables.find((entry) => entry.path === path) ??
      meta.tables.find((entry) => entry.path.endsWith(`/${path.split('/').pop()}`)) ??
      meta.tables[0];
    const kind: 'word' | 'char' = table.kind === 'char' ? 'char' : 'word';
    const base = kind === 'char' ? MOCK_CHARS : MOCK_WORDS;
    const domain = table.path.startsWith('domains/') ? table.path.split('/')[1] : null;
    const entries = domain ? domainEntries(base, domain, kind) : base;

    const total = Math.max(1, entries.reduce((sum, entry) => sum + entry.count, 0));
    const cumulative: number[] = [];
    let running = 0;
    for (const entry of entries) {
      running += entry.count;
      cumulative.push(running / total);
    }

    const n = entries.length;
    const points: [number, number][] = [];
    if (n === 0) return { path: table.path, kind, entries: 0, total_tokens: 0, points };
    const cap = Math.max(8, Math.min(4000, maxPoints));
    const decades = Math.max(1, Math.log10(n));
    const perDecade = Math.max(2, cap / decades);
    const targets: number[] = [];
    for (let k = 0; targets.length <= cap; k += 1) {
      const rank = Math.max(1, Math.round(Math.pow(10, k / perDecade)));
      if (rank > n) break;
      if (targets[targets.length - 1] !== rank) targets.push(rank);
    }
    if (targets[targets.length - 1] !== n) targets.push(n);
    for (const rank of targets) points.push([rank, cumulative[rank - 1]]);

    return { path: table.path, kind, entries: n, total_tokens: total, points };
  },

  searchWords(query: string, kind: 'word' | 'char', limit: number): RankRow[] {
    if (!query) return [];
    const base = kind === 'char' ? MOCK_CHARS : MOCK_WORDS;
    return base
      .filter((entry) => entry.word.startsWith(query))
      .slice(0, limit)
      .map((entry) => ({ rank: entry.rank, word: entry.word, count: entry.count, flags: 1 }));
  },

  onProgress(handler: ProgressHandler): () => void {
    mockProgressHandlers.add(handler);
    return () => mockProgressHandlers.delete(handler);
  },

  onDone(handler: DoneHandler): () => void {
    mockDoneHandlers.add(handler);
    return () => mockDoneHandlers.delete(handler);
  },

  onError(handler: ErrorHandler): () => void {
    mockErrorHandlers.add(handler);
    return () => mockErrorHandlers.delete(handler);
  },

  /** 模拟一次完整扫描：日志 → 探测 → 分域进度 → 各表结果 → 完成 */
  startScan(params: ScanParams) {
    MOCK.cancelScan();
    const plan = MOCK.planCorpus(params.corpus || MOCK_DIR);
    const meta = buildMockMeta(params.corpus || 'D:\\corpus');
    let elapsed = 0;
    const at = (ms: number, fn: () => void) => {
      elapsed += ms;
      mockTimers.push(setTimeout(fn, elapsed));
    };

    at(60, () =>
      MOCK.emitProgress({
        event: 'log',
        level: 'info',
        message: `开始扫描：${params.corpus || MOCK_DIR}（浏览器演示数据）`,
      })
    );
    at(120, () =>
      MOCK.emitProgress({
        event: 'log',
        level: 'info',
        message: `输出目录：${params.out || MOCK_DIR}`,
      })
    );
    at(160, () =>
      MOCK.emitProgress({
        event: 'plan',
        files: plan.files,
        bytes: plan.bytes,
        domains: plan.domains.map((d) => ({ name: d.name, files: d.files, bytes: d.bytes })),
        warnings: ['浏览器演示模式：以下进度为模拟数据'],
      })
    );

    for (let step = 1; step <= 8; step += 1) {
      const percent = (step / 8) * 100;
      at(220, () =>
        MOCK.emitProgress({
          event: 'phase',
          phase: step <= 6 ? '分词统计' : '写出产物',
          domain: MOCK_DOMAIN_NAMES[Math.min(MOCK_DOMAIN_NAMES.length - 1, Math.floor(step / 3))],
          units_done: Math.round((plan.files * step) / 8),
          units_total: plan.files,
          bytes_done: Math.round((plan.bytes * percent) / 100),
          bytes_total: plan.bytes,
          percent,
        })
      );
    }

    for (const table of meta.tables.slice(0, 3)) {
      at(180, () =>
        MOCK.emitProgress({
          event: 'table',
          table: table.path,
          entries: table.entries,
          total_tokens: table.total_tokens,
          tier_stats: table.tier_stats,
        })
      );
    }

    at(400, () => {
      MOCK.emitProgress({ event: 'log', level: 'info', message: '全部完成，产物已写出。' });
      for (const handler of mockDoneHandlers) handler(meta);
    });
  },

  cancelScan() {
    for (const timer of mockTimers) clearTimeout(timer);
    mockTimers = [];
  },

  emitProgress(payload: ScanProgress) {
    for (const handler of mockProgressHandlers) handler(payload);
  },

  /** 首次调用返回一段示例文本，之后返回空串（模拟「取走待分析文本」） */
  takePendingSelection(): string {
    if (mockPendingTaken) return '';
    mockPendingTaken = true;
    return '语言统计可以帮助我们理解文本的词汇分布，覆盖率与排名说明了用词难度。';
  },
};

/** 供页面判断行的 flags 语义（避免页面自己解析位运算） */
export function rowInDict(flags: number): boolean {
  return inDictFromFlags(flags);
}
