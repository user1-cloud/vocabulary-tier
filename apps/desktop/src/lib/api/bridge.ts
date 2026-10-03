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
import { inDictFromFlags, TIER_KEYS } from './../tier-colors';
import { isCjkChar, isPunctuationOrSpace, normalizeToken } from './../segments';
import { t } from './../i18n.svelte';
import {
  DEFAULT_CHAR_TIER_PCT,
  DEFAULT_WORD_TIER_PCT,
  FULL_SCOPE,
  splitTableKey,
  tableKey,
} from './../types';
import { pctForRank } from './../format';
import type {
  AppInfo,
  Binding,
  ComposeParams,
  ComposedTable,
  CorpusPlan,
  DatasetStatus,
  DictItem,
  DictRef,
  LibraryInfo,
  Meta,
  Origin,
  RankRow,
  ScanParams,
  ScanProgress,
  Settings,
  TableItem,
  TableMeta,
  TableRank,
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
/**
 * 后端 → 悬浮小窗 的「待分析文本」推送事件（payload = 纯字符串，已冻结）。
 *
 * 小窗关闭时只是隐藏而不是销毁，JS 只在第一次挂载时跑一次；所以每次显示小窗
 * 后由 Rust 侧（`show_popup()` 里的 `emit_to(POPUP_LABEL, "popup:text", text)`）
 * 主动把文本推给小窗，不能只靠小窗自己 `take_pending_selection`。
 * 空串 = 当前没有选中内容，小窗应显示空输入框等用户手输。
 */
export const POPUP_TEXT_EVENT = 'popup:text';
/** 取词失败原因，与 `POPUP_TEXT_EVENT` 同时推送（见 `onPopupNote`） */
export const POPUP_NOTE_EVENT = 'popup:note';

/**
 * 悬浮小窗 → 主窗口 的文本回传事件（小窗里的「发回主窗口」按钮）。
 *
 * 与 `POPUP_TEXT_EVENT` 方向相反、事件名也不同，别混淆：冻结接口里没有
 * 「发回主窗口」的命令，而 `open_popup` 的语义是「打开小窗并填入文本」，
 * 用它等于自己给自己发，所以这一路走 Tauri 自带的事件总线广播。
 */
export const POPUP_REPLY_EVENT = 'voctier:popup-text';

/** 非 Tauri 环境下的统一提示（调用时取当前界面语言） */
function noTauriHint(): string {
  return t('bridge.noTauri');
}

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
    return { ok: false, error: noTauriHint() };
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
      tauri_version: raw.tauri_version ?? t('bridge.unknownVersion'),
      core_version: raw.core_version ?? t('bridge.unknownVersion'),
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
// 数据文件夹：词典管理（dicts\）+ 词频表管理（tables\）
//
// 契约提醒（**入参 camelCase、出参 snake_case**）：
//   - `dict_delete` 的 Rust 参数名是 `file_name`，Tauri 会转成 camelCase，
//     所以 JS 侧传 `{ fileName }`（与 `analyzeText` 的 `byte_start` → `byteStart`
//     是同一套规则，见 lib.rs 末尾的契约测试）。
//   - 返回值一律 snake_case（`dicts_dir` / `file_name` / `in_library` …），
//     与 `$lib/types.ts::LibraryInfo` 等一一对应，不做映射。
// ---------------------------------------------------------------------------

/** 数据文件夹的整体状况（词典列表 + 词频表列表都由它给的信息定位） */
export async function libraryInfo(): Promise<Result<LibraryInfo>> {
  if (!isTauri()) return { ok: true, data: MOCK.libraryInfo() };
  return call<LibraryInfo>('library_info');
}

/**
 * 换数据文件夹。
 *
 * 后端只建目录、**不搬运**已有内容，并且会把「当前激活的表」清掉（换了文件夹
 * 就是换了一整套词典与词频表）。所以调用方拿到新 `LibraryInfo` 后应当重新拉
 * `dict_list()` 与 `table_list()`。
 */
export async function setDataDir(dir: string): Promise<Result<LibraryInfo>> {
  if (!isTauri()) return { ok: true, data: MOCK.setDataDir(dir) };
  return call<LibraryInfo>('set_data_dir', { dir });
}

/** 建出数据文件夹的 `dicts\` 与 `tables\`，返回数据文件夹的绝对路径 */
export async function ensureDataDirs(): Promise<Result<string>> {
  if (!isTauri()) return { ok: true, data: MOCK_LIB_ROOT };
  return call<string>('ensure_data_dirs');
}

/**
 * 数据文件夹里有没有**可用**的词典。
 *
 * 扫描页用它决定要不要挡住「开始统计」：词典外置之后没有内置兜底，
 * 空词典跑出来的是一张只有单字的废表，还不如不让用户点。
 */
export async function libraryReady(): Promise<Result<boolean>> {
  if (!isTauri()) return { ok: true, data: MOCK.libraryReady() };
  return call<boolean>('library_ready');
}

/**
 * 列出数据文件夹里的全部词典。
 *
 * ⚠ 后端会**完整读取并解析**每个 `.dict`（一份 jieba 词典约 50–100 ms，会算
 * sha256）。界面按需调用并缓存结果，**别在每次重渲染时都调**。
 */
export async function dictList(): Promise<Result<DictItem[]>> {
  if (!isTauri()) return { ok: true, data: MOCK.dictList() };
  const res = await call<DictItem[]>('dict_list');
  if (!res.ok) return res;
  return { ok: true, data: res.data ?? [] };
}

/** 把一份 `.dict` 复制进数据文件夹；返回落地后的文件名（重名自动加后缀，绝不覆盖） */
export async function dictImport(path: string): Promise<Result<string>> {
  if (!isTauri()) return { ok: true, data: MOCK.dictImport(path) };
  return call<string>('dict_import', { path });
}

/**
 * 删掉一份词典（只删数据文件夹里那一个文件）。
 *
 * **没有任何权限等级**：后端没有 builtin 之类的权限位，「预置」只是个展示徽标。
 * 但删掉被引用的词典会让那些表变成「词典缺失」，所以调用方必须先确认。
 */
export async function dictDelete(fileName: string): Promise<Result<null>> {
  if (!isTauri()) {
    MOCK.dictDelete(fileName);
    return { ok: true, data: null };
  }
  return call<null>('dict_delete', { fileName });
}

/** 列出词频表：数据文件夹里的全部 + 数据文件夹之外那张激活的（如果有） */
export async function tableList(): Promise<Result<TableItem[]>> {
  if (!isTauri()) return { ok: true, data: MOCK.tableList() };
  const res = await call<TableItem[]>('table_list');
  if (!res.ok) return res;
  return { ok: true, data: res.data ?? [] };
}

/** 激活数据文件夹里的一张表；返回它的 meta（同时后端按记录的词典链重建分词器） */
export async function activateLibraryTable(name: string): Promise<Result<Meta>> {
  if (!isTauri()) return MOCK.activateLibraryTable(name);
  const res = await call<Meta>('activate_library_table', { name });
  if (!res.ok) return res;
  if (!res.data) return { ok: false, error: t('bridge.noMeta') };
  return { ok: true, data: res.data };
}

/**
 * 删掉数据文件夹里的一张表（整个产物目录）。
 *
 * 只能删数据文件夹内的 —— 数据文件夹之外的目录不归我们管，界面对那些表
 * （`TableItem.in_library === false`）也不给删除按钮。
 */
export async function tableDelete(name: string): Promise<Result<null>> {
  if (!isTauri()) {
    MOCK.tableDelete(name);
    return { ok: true, data: null };
  }
  return call<null>('table_delete', { name });
}

/**
 * 换**主词频表**：指定哪个表组回答"这个词有多常见"。
 *
 * 粒度是全局的：划句分析、排行榜、分组阈值都必须跟着换，否则同一句话在两个页面
 * 会显示成两种颜色。后端会重开一次数据集并把新的 meta 返回回来。
 */
export async function setPrimaryScope(scope: string): Promise<Result<Meta>> {
  if (!isTauri()) {
    const meta = MOCK.activeDataset();
    if (!meta) return { ok: false, error: t('bridge.noMeta') };
    if (!meta.tables.some((entry) => entry.path === scope)) {
      return { ok: false, error: `演示数据里没有表组「${scope}」` };
    }
    return { ok: true, data: meta };
  }
  const res = await call<Meta>('set_primary_scope', { scope });
  if (!res.ok) return res;
  if (!res.data) return { ok: false, error: t('bridge.noMeta') };
  return { ok: true, data: res.data };
}

/**
 * 把若干张表**相加**成一张新表（同一份产物目录里的另一个表组）。
 *
 * 相加在数学上是精确的：`scan` 本身就是"逐表组扫完再累加"，所以各表组表相加
 * 逐条等于全量扫描出来的那张表。新表与别的表完全平级 —— 能当主表、能再被相加。
 */
export async function composeTables(
  params: ComposeParams
): Promise<Result<ComposedTable[]>> {
  if (!isTauri()) return { ok: true, data: MOCK.composeTables(params) };
  return call<ComposedTable[]>('compose_tables', { params });
}

/**
 * 给一张新表算默认的输出目录：`<数据文件夹>\tables\<洗过的名字>`。
 *
 * 扫描页用它预填输出路径，用户仍可改成任意位置（落到数据文件夹之外的产物目录
 * 会以 `in_library: false` 出现在词频表列表里，同样能用）。
 */
export async function suggestTableDir(name: string): Promise<Result<string>> {
  if (!isTauri()) return { ok: true, data: MOCK.suggestTableDir(name) };
  return call<string>('suggest_table_dir', { name });
}

/**
 * 当前**已经打开**的那张表的 `meta`；没打开就是 null。
 *
 * 各页面开机都拿它来渲染，**不要再**自己拼 `settings.dataDir` 的路径：
 * `dataDir` 现在是「数据文件夹」（里面是 `dicts\` 与 `tables\`），
 * 它下面没有 `meta.json`，拼出来必然是 `exists: false`，于是每个页面都会显示
 * 「尚未打开词频表」。后端在启动时已经按 `activeTable` / `activeTablePath`
 * 打开过表了，这里直接取即可。
 *
 * `datasetStatus(dir)` / `openDataset(dir)` 仍然保留，它们的用途是
 * 「用户手动指向数据文件夹之外的某份已有产物目录」。
 */
export async function activeDataset(): Promise<Result<Meta | null>> {
  if (!isTauri()) return { ok: true, data: MOCK.activeDataset() };
  const res = await call<Meta | null>('active_dataset');
  if (!res.ok) return res;
  return { ok: true, data: res.data ?? null };
}

/**
 * 当前打开那张表的**产物目录绝对路径**。
 *
 * 后端没有一条命令直接给这个值，但 `library_info()` 的两个字段互斥地覆盖了两种情况：
 * `active_table_path`（数据文件夹之外）优先，其次 `tables_dir\active_table`。
 * 只用于展示（页面里显示"当前表在哪儿"）与按目录定位的命令（`tier_curve` 等）。
 */
export function activeTableDir(info: LibraryInfo | null | undefined): string | null {
  if (!info) return null;
  if (info.active_table_path) return info.active_table_path;
  if (!info.active_table) return null;
  return `${info.tables_dir.replace(/[\\/]+$/, '')}\\${info.active_table}`;
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
    if (!status.meta) return { ok: false, error: t('bridge.invalidDatasetDir') };
    return { ok: true, data: status.meta };
  }
  const res = await call<Meta>('open_dataset', { dir });
  if (!res.ok) return res;
  if (!res.data) return { ok: false, error: t('bridge.noMeta') };
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
    if (!hit) return { ok: false, error: t('bridge.wordNotInCorpus', { word }) };
    return { ok: true, data: hit };
  }
  // `Option<WordHit>`：未收录时后端可能返回 null（而不是 Err）
  const res = await call<WordHit | null>('lookup_word', { word, kind, dir: dir ?? null });
  if (!res.ok) return res;
  if (!res.data) return { ok: false, error: t('bridge.wordNotInCorpus', { word }) };
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
    popupWidth: 520,
    popupHeight: 560,
    popupOpacity: 0.96,
    popupAlwaysOnTop: true,
    popupAutoCloseMs: 0,
    // 预留：关主窗口收进托盘（界面未提供开关，恒为 true）
    closeToTray: true,
    theme: 'system',
    locale: 'zh-CN',
    threads: 0,
    hmm: true,
    keepDigit: true,
    keepLatin: true,
    skipSingleChar: false,
    // 词典链：null / 空数组 = 用数据文件夹里全部 `.dict`（按文件名排序）
    scanDicts: null,
    activeTable: null,
    activeTablePath: null,
    minCount: 1,
    skipDomainTables: false,
    // 全表组平等：谁当"主表"由 primaryScope 决定，默认 full（没有 full 就用第一个）
    primaryScope: null,
    // `enabledTables` 已废弃（留着只为读老设置），新代码不再写它
    enabledTables: null,
    // 分组默认按**前%**：排名绝对值跨表不可比，而任意表组都能当主表
    tierMethod: 'top_pct',
    tierWordBounds: null,
    tierCharBounds: null,
    tierCoverage: null,
    tierPct: null,
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
  if (!isTauri()) return { ok: false, error: t('bridge.popupUnavailable') };
  return call<null>('open_popup', { text });
}

/** 小窗启动时取走待分析文本 */
export async function takePendingSelection(): Promise<Result<string>> {
  if (!isTauri()) return { ok: true, data: MOCK.takePendingSelection() };
  return call<string>('take_pending_selection');
}

/**
 * 订阅后端在**每次显示小窗后**推送的待分析文本（`popup:text`）。
 *
 * 返回值是 `Promise<UnlistenFn>`：`listen` 本身是异步的，组件必须在卸载时调用
 * 拿到的 unlisten 才会真正解绑。组件里的标准写法（注意 `disposed` 竞态）：
 *
 *   $effect(() => {
 *     let disposed = false;
 *     let unlisten: (() => void) | null = null;
 *     void onPopupText(apply).then((off) => (disposed ? off() : (unlisten = off)));
 *     return () => { disposed = true; unlisten?.(); };
 *   });
 *
 * 浏览器预览（`isTauri() === false`）下返回一个空操作的 unlisten，保证 UI 能跑。
 */
export async function onPopupText(cb: (text: string) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return listen<string>(POPUP_TEXT_EVENT, (event) =>
    cb(typeof event.payload === 'string' ? event.payload : '')
  );
}

/**
 * 取词失败原因（`popup:note`）。
 *
 * 取词失败的原因全在环境里（焦点被抢、权限级别不一致、剪贴板被占用、
 * 目标程序不响应 Ctrl+C），而日志文件用户未必找得到。后端在每次显示小窗后
 * 会把「这次为什么没取到」一并推过来，界面直接显示出来，用户当场就知道
 * 该换目标程序、该以管理员身份运行、还是干脆手输。
 *
 * 取到内容时 payload 是空串。
 */
export async function onPopupNote(cb: (note: string) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return listen<string>(POPUP_NOTE_EVENT, (event) =>
    cb(typeof event.payload === 'string' ? event.payload : '')
  );
}

/**
 * 小窗挂载时主动拉一次取词失败原因（与 `onPopupNote` 的关系同 `takePendingSelection`：
 * 首次显示时事件可能早于监听注册）。
 */
export async function fetchCaptureNote(): Promise<string> {
  if (!isTauri()) return MOCK.captureNote();
  const res = await call<string>('capture_note');
  return res.ok ? res.data : '';
}

/**
 * 隐藏悬浮小窗（后端会同时把焦点还给用户原来在用的窗口）。
 *
 * 热键流程里这个很重要：小窗看完要能一键关掉，且关掉后焦点回到你刚才划词的
 * 那个程序，方便接着选下一句。
 */
export async function hidePopup(): Promise<void> {
  if (!isTauri()) return;
  await call<void>('hide_popup');
}

/**
 * 小窗 → 主窗口的文本回传（见 `POPUP_REPLY_EVENT`）。
 */
export async function emitPopupReply(text: string): Promise<Result<null>> {
  if (!isTauri()) return { ok: false, error: t('bridge.noMainWindow') };
  try {
    const { emit } = await import('@tauri-apps/api/event');
    await emit(POPUP_REPLY_EVENT, { text });
    return { ok: true, data: null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/** 主窗口订阅小窗回传的文本 */
export function onPopupReply(handler: (text: string) => void): () => void {
  if (!isTauri()) return () => {};
  return bridgeListen<{ text: string }>(POPUP_REPLY_EVENT, (payload) => handler(payload?.text ?? ''));
}

// ---------------------------------------------------------------------------
// 主题跨窗口同步（见 $lib/theme-sync.ts）
// ---------------------------------------------------------------------------

/** 主题广播事件名（主窗口 ↔ 悬浮小窗） */
export const THEME_EVENT = 'voctier:theme';

/**
 * 把主题变化广播给**所有**窗口。
 *
 * 用全局 `emit`（不是 `getCurrentWindow().emit()`）：后者只发给本窗口自己，
 * 跨窗口同步就失效了。payload 用对象包一层，和 `popup:text` 的裸字符串区分开。
 */
export async function emitTheme(mode: string): Promise<Result<null>> {
  if (!isTauri()) return { ok: true, data: null };
  try {
    const { emit } = await import('@tauri-apps/api/event');
    await emit(THEME_EVENT, { theme: mode });
    return { ok: true, data: null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/**
 * 订阅别的窗口广播来的主题。返回 Promise<UnlistenFn>，与 `onPopupText` 同一套用法。
 *
 * 注意：`emit` 是广播，本窗口也会收到自己的那条，调用方必须自己比对当前 mode
 * （`theme-sync.ts` 里做了）。
 */
export async function onTheme(cb: (payload: { theme?: string } | null) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return listen<{ theme?: string }>(THEME_EVENT, (event) => cb(event.payload ?? null));
}

// ---------------------------------------------------------------------------
// 界面语言跨窗口同步（见 $lib/locale-sync.ts）
// ---------------------------------------------------------------------------

/** 界面语言广播事件名（主窗口 ↔ 悬浮小窗） */
export const LOCALE_EVENT = 'voctier:locale';

/**
 * 把界面语言变化广播给**所有**窗口。与 `emitTheme` 同一套理由：
 * 必须用全局 `emit`，`getCurrentWindow().emit()` 只发给自己。
 */
export async function emitLocale(locale: string): Promise<Result<null>> {
  if (!isTauri()) return { ok: true, data: null };
  try {
    const { emit } = await import('@tauri-apps/api/event');
    await emit(LOCALE_EVENT, { locale });
    return { ok: true, data: null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/**
 * 订阅别的窗口广播来的界面语言。返回 Promise<UnlistenFn>，与 `onTheme` 同一套用法。
 *
 * 同样是广播，本窗口会收到自己的那条，调用方要自己比对当前值。
 */
export async function onLocale(
  cb: (payload: { locale?: string } | null) => void
): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return listen<{ locale?: string }>(LOCALE_EVENT, (event) => cb(event.payload ?? null));
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
    return { ok: false, error: t('bridge.clipboardUnsupported') };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

// ---------------------------------------------------------------------------
// 桌面端选择器（dialog 插件）—— 同样只在本文件里 import 插件
// ---------------------------------------------------------------------------

/** 选目录；用户取消返回 null */
export async function pickDirectory(title = t('bridge.pickDirectory')): Promise<Result<string | null>> {
  if (!isTauri()) return { ok: false, error: noTauriHint() };
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open({ directory: true, multiple: false, title });
    return { ok: true, data: typeof picked === 'string' ? picked : null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/** 选文件；用户取消返回 null。`filters` 用于限制扩展名（例如词典只认 `*.dict`） */
export async function pickFile(
  title = t('bridge.pickFile'),
  filters?: { name: string; extensions: string[] }[]
): Promise<Result<string | null>> {
  if (!isTauri()) return { ok: false, error: noTauriHint() };
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open({
      directory: false,
      multiple: false,
      title,
      ...(filters && filters.length > 0 ? { filters } : {}),
    });
    return { ok: true, data: typeof picked === 'string' ? picked : null };
  } catch (error) {
    return { ok: false, error: toMessage(error) };
  }
}

/** 用系统默认程序打开路径 / URL（opener 插件） */
export async function openExternal(target: string): Promise<Result<null>> {
  if (!isTauri()) return { ok: false, error: noTauriHint() };
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
 * 的默认词频表阈值（500 / 3000 / 10000 / 30000 / 80000 / 200000）对应。
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
  { rank: 46, word: '词频表', count: 7712, tier: 3 },
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
 * 演示用的表组。真实产物是「全库 + 7 个表组」，这里也放 7 个，
 * 这样「表管理」页在浏览器里能完整看到 2 × 8 = 16 张表。
 */
const MOCK_DOMAIN_NAMES = ['blog', 'book', 'forum', 'gov', 'news', 'parallel', 'wiki'];

/** 表组的中文说明（只用于 planCorpus 的展示，不影响表路径） */
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
 * / `default_char_tiers()` 保持一致（词频表 100/1000/5000/20000/50000/150000，
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
 * 真实语料里默认阈值下的七组覆盖率是「头重脚轻」的一条递减序列（词频表实测约
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

/** 给表组造一份「顺序不同、部分缺档」的排名，模拟真实语料的表组差异 */
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
  // 让每个表组都缺掉几个词，制造「该表组未收录」的展示效果
  const drop = domain === 'book' ? 3 : domain === 'forum' ? 5 : domain === 'wiki' ? 7 : 0;
  const kept = unique.slice(drop);
  return kept.map((entry, i) => ({ ...entry, rank: i + 1 }));
}

function mockTableMeta(
  scope: string,
  kind: string,
  entries: MockEntry[],
  stats: TierStat[]
): TableMeta {
  const totalTokens = entries.reduce((sum, e) => sum + e.count, 0);
  return {
    // `path` 是**表组名**（schema v3 起），不再是 `full/word` 那种路径
    path: scope,
    kind,
    entries: entries.length,
    total_tokens: totalTokens,
    vfr_bytes: entries.length * 34 + 4096,
    tiers: mockTiers(kind === 'char' ? 'char' : 'word'),
    tier_stats: stats,
    min_count: 1,
    tier_pct: [...(kind === 'char' ? DEFAULT_CHAR_TIER_PCT : DEFAULT_WORD_TIER_PCT)],
    source_tables: [],
  };
}

function buildMockMeta(corpusRoot: string): Meta {
  return {
    schema_version: 3,
    generated_at: '2024-05-01T09:30:00Z',
    tool_version: '0.1.0',
    corpus_root: corpusRoot,
    elapsed_ms: 184_230,
    tokenizer: {
      engine: 'jieba-rs',
      version: '0.7.4',
      hmm: true,
      // v2：词典链，dicts[0] 是主词典
      dicts: [mockDictRef('jieba 主词典', MOCK_MAIN_DICT_ENTRIES, 'a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90')],
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
      mockTableMeta(FULL_SCOPE, 'word', MOCK_WORDS, mockTierStats(MOCK_WORDS, 'word')),
      mockTableMeta(FULL_SCOPE, 'char', MOCK_CHARS, mockTierStats(MOCK_CHARS, 'char')),
      ...MOCK_DOMAIN_NAMES.flatMap((name) => {
        const words = domainEntries(MOCK_WORDS, name, 'word');
        const chars = domainEntries(MOCK_CHARS, name, 'char');
        // 铺平之后每个表组就是一个平级的目录名（不再有 domains/ 这一层）
        return [
          mockTableMeta(name, 'word', words, mockTierStats(words, 'word')),
          mockTableMeta(name, 'char', chars, mockTierStats(chars, 'char')),
        ];
      }),
    ],
    tier_names: ['极多', '很多', '较多', '中等', '较少', '很少', '极少'],
    // 与后端一致：产物自带稳定标识，界面据此取色（不按组名取色）
    tier_keys: [...TIER_KEYS],
  };
}

const MOCK_DIR = '(浏览器预览) 演示数据集';

// ===========================================================================
// 数据文件夹（词典库 / 词频表库）的演示数据
//
// 目标：让浏览器预览里**所有状态都看得到**，而不是只走顺利路径：
//   - 词典：一份正常的、一份 `error` 非空的坏文件、一份 `freq_zero > 0` 的隐患文件；
//   - 词频表：`binding.kind` 分别是 ok / legacy / drifted / missing，外加一张
//     `in_library: false` 的外部表（那种表界面不给删除按钮）。
//
// 这些数据是**可变的**：导入 / 删除词典、激活 / 删除词频表都要能在浏览器里点出来。
// ===========================================================================

/** 演示用的数据文件夹（和 Rust 侧默认位置同构：里面是 dicts\ 与 tables\） */
const MOCK_LIB_ROOT = 'C:\\Users\\demo\\AppData\\Local\\com.voctier.desktop\\data';

/** 主词典的有效词条数：给 mock meta 的 `dicts[]` 与 `report.entries` 共用 */
const MOCK_MAIN_DICT_ENTRIES = 349_046;

/** 造一条 `DictRef`；`file_name` + `root` 拼出 `path`，`sha256` 给固定值 */
function mockDictRef(
  name: string,
  entries: number,
  sha256: string,
  file = '',
  root = MOCK_LIB_ROOT
): DictRef {
  return {
    id: name,
    name,
    path: `${root}\\dicts\\${file || `${name}.dict`}`,
    entries,
    sha256,
  };
}

/** 演示用词典清单（`file_name` 就是 `dicts\` 下的文件名，也是 `dictFiles` 的取值） */
let mockDicts: DictItem[] = [
  {
    dict: mockDictRef('jieba 主词典', MOCK_MAIN_DICT_ENTRIES, 'a1b2'.repeat(16), 'jieba 主词典.dict'),
    error: null,
    report: {
      entries: MOCK_MAIN_DICT_ENTRIES,
      comments: 42,
      blanks: 3,
      freq_omitted: 0,
      freq_zero: 0,
    },
    file_name: 'jieba 主词典.dict',
    origin: 'seeded',
  },
  {
    // 隐患样本：显式写了 0 的词频 → 这些词永远切不出来
    dict: mockDictRef('领域补充词', 1_284, 'b2c3'.repeat(16), '领域补充词.dict'),
    error: null,
    report: {
      entries: 1_284,
      comments: 12,
      blanks: 1,
      freq_omitted: 37,
      freq_zero: 9,
    },
    file_name: '领域补充词.dict',
    origin: 'imported',
  },
  {
    // 坏文件样本：读不了，但**必须列出来**，否则用户只知道"我放进去了怎么没有"
    dict: mockDictRef('手工整理.dict', 0, '', '手工整理.dict'),
    error: '第 128 行不是合法的 jieba 词条（缺少词频列）',
    report: { entries: 0, comments: 0, blanks: 0, freq_omitted: 0, freq_zero: 0 },
    file_name: '手工整理.dict',
    origin: 'unknown',
  },
];

/** 演示用词频表清单；`path` 是绝对路径，外部表落在数据文件夹之外 */
const MOCK_TABLES_DIR = `${MOCK_LIB_ROOT}\\tables`;
/** 数据文件夹之外那张演示表（`in_library: false`） */
export const MOCK_EXTERNAL_TABLE_DIR = 'E:\\corpora\\voctier-out\\2024 汇总';

let mockActiveTable: string | null = '演示全库表';

/** 造一份词频表列表项：`meta` 用 `buildMockMeta` 那份假产物 */
function mockTableItem(
  name: string,
  opts: {
    path?: string;
    corpusRoot?: string;
    generatedAt?: string;
    binding?: Binding;
    inLibrary?: boolean;
    origin?: Origin;
    error?: string | null;
    active?: boolean;
  } = {}
): TableItem {
  const inLibrary = opts.inLibrary ?? true;
  const path = opts.path ?? (inLibrary ? `${MOCK_TABLES_DIR}\\${name}` : MOCK_EXTERNAL_TABLE_DIR);
  const meta = buildMockMeta(opts.corpusRoot ?? 'D:\\corpus');
  meta.generated_at = opts.generatedAt ?? meta.generated_at;
  return {
    name,
    path,
    meta,
    error: opts.error ?? null,
    binding: opts.binding ?? { kind: 'ok' },
    active: opts.active ?? mockActiveTable === name,
    in_library: inLibrary,
    origin: opts.origin ?? (inLibrary ? 'scanned' : 'unknown'),
  };
}

/** 演示词频表：覆盖 ok / legacy / drifted / missing 四种绑定 + 一张外部表 */
function buildMockTableList(): TableItem[] {
  return [
    mockTableItem('演示全库表', {
      generatedAt: '2024-05-01T09:30:00Z',
      binding: { kind: 'ok' },
      origin: 'scanned',
    }),
    mockTableItem('2023 老产物', {
      generatedAt: '2023-11-08T14:02:00Z',
      binding: { kind: 'legacy' },
      origin: 'unknown',
    }),
    mockTableItem('新闻语料 v2', {
      corpusRoot: 'D:\\corpus\\news',
      generatedAt: '2024-06-11T18:45:00Z',
      binding: { kind: 'drifted', changed: ['领域补充词'] },
      origin: 'scanned',
    }),
    mockTableItem('论坛语料', {
      corpusRoot: 'D:\\corpus\\forum',
      generatedAt: '2024-04-02T07:12:00Z',
      binding: { kind: 'missing', missing: ['旧版主词典'] },
      origin: 'imported',
    }),
    mockTableItem('外部汇总表', {
      corpusRoot: 'E:\\corpora\\raw',
      generatedAt: '2024-02-20T11:05:00Z',
      binding: { kind: 'ok' },
      inLibrary: false,
      origin: 'unknown',
    }),
  ];
}

let mockTables: TableItem[] = buildMockTableList();

/** 把用户给的表名洗成安全的目录名（`sanitize_table_name` 的前端等价） */
function sanitizeMockTableName(raw: string): string {
  let s = raw
    .trim()
    .replace(/[<>:"/\\|?*]/g, '_')
    // eslint-disable-next-line no-control-regex
    .replace(/[\u0000-\u001f]/g, '_');
  while (s.endsWith('.') || s.endsWith(' ')) s = s.slice(0, -1);
  return s || '未命名';
}

/** 用小窗 / 划句页共用的「首次取词」模拟 */
let mockPendingTaken = false;

/**
 * 浏览器预览开关：`?capture=failed` 让 mock 直接呈现「取词失败」那一屏。
 *
 * 这一屏是小窗里最容易出布局问题的（长中文说明 + 460px 窄窗），
 * 之前就是因为只能在真实 Tauri 里才看得到，横向溢出一直没被发现。
 */
const MOCK_CAPTURE_FAILED =
  typeof location !== 'undefined' &&
  new URLSearchParams(location.search).get('capture') === 'failed';

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
  ...'我们国家还可以语言统计数据文本词汇频率语料库分词覆盖率阈值排名字表词频表未收录'.split(''),
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

    const domainRanks: TableRank[] = [];
    if (accepted) {
      for (const name of domainFilter) {
        const source = singleCjk
          ? domainEntries(MOCK_CHARS, name, 'char')
          : domainEntries(MOCK_WORDS, name, 'word');
        const hit = source.find((w) => w.word === piece);
        const kind = singleCjk ? 'char' : 'word';
        domainRanks.push({
          scope: name,
          kind,
          rank: hit ? hit.rank : null,
          top_pct: hit ? pctForRank(hit.rank, source.length) : null,
          entries: source.length,
          count: hit ? hit.count : null,
        });
      }
    }

    const entries = singleCjk ? MOCK_CHARS.length : MOCK_WORDS.length;
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
      top_pct: entry ? pctForRank(entry.rank, entries) : null,
      entries: entry ? entries : null,
      tier: entry ? entry.tier : null,
      tier_name: entry ? (singleCjk ? charTiers : wordTiers)[entry.tier].name : null,
      in_dict: accepted ? true : null,
      from_user: accepted ? false : null,
      table_ranks: domainRanks,
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
      popupWidth: 520,
      popupHeight: 560,
      popupOpacity: 0.96,
      popupAlwaysOnTop: true,
      popupAutoCloseMs: 0,
      closeToTray: true,
      theme: 'system',
      locale: 'zh-CN',
      threads: 0,
      hmm: true,
      keepDigit: true,
      keepLatin: true,
      skipSingleChar: false,
      scanDicts: ['jieba 主词典.dict', '领域补充词.dict'],
      activeTable: '演示全库表',
      activeTablePath: null,
      minCount: 1,
      skipDomainTables: false,
      primaryScope: FULL_SCOPE,
      enabledTables: null,
      tierMethod: 'top_pct',
      tierWordBounds: null,
      tierCharBounds: null,
      tierCoverage: null,
      tierPct: null,
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

  /** 当前"已打开"的表：演示里就是 `mockActiveTable` 那张 */
  activeDataset(): Meta | null {
    const item = mockTables.find((table) => table.name === mockActiveTable);
    return item?.meta ?? null;
  },

  // ------------------------------------------------------------ 数据文件夹

  libraryInfo(): LibraryInfo {
    const active = mockTables.find((table) => table.active) ?? null;
    return {
      root: MOCK_LIB_ROOT,
      dicts_dir: `${MOCK_LIB_ROOT}\\dicts`,
      tables_dir: MOCK_TABLES_DIR,
      is_default: true,
      active_table: mockActiveTable,
      active_table_path: active && !active.in_library ? active.path : null,
      active_binding: active?.binding ?? null,
      active_warnings:
        active && active.binding.kind === 'legacy'
          ? ['这是 v1 老产物：没有词典指纹，词典一致性无从校验。']
          : [],
    };
  },

  setDataDir(dir: string): LibraryInfo {
    // 真实后端只建目录、不搬内容，并且会清掉「当前激活的表」
    mockActiveTable = null;
    for (const table of mockTables) table.active = false;
    const info = MOCK.libraryInfo();
    return { ...info, root: dir, dicts_dir: `${dir}\\dicts`, tables_dir: `${dir}\\tables` };
  },

  libraryReady(): boolean {
    return mockDicts.some((item) => item.error === null);
  },

  dictList(): DictItem[] {
    return mockDicts.map((item) => ({ ...item }));
  },

  dictImport(path: string): string {
    const base = path.split(/[\\/]/).pop() ?? 'imported.dict';
    const stem = base.replace(/\.dict$/i, '') || 'imported';
    let name = `${stem}.dict`;
    let n = 2;
    while (mockDicts.some((item) => item.file_name === name)) {
      name = `${stem} (${n}).dict`;
      n += 1;
    }
    // 导入的演示词典一律是"正常"的，带一点点隐患让列表有内容可看
    mockDicts = [
      ...mockDicts,
      {
        dict: mockDictRef(stem, 512, 'c3d4'.repeat(16), name),
        error: null,
        report: { entries: 512, comments: 4, blanks: 0, freq_omitted: 6, freq_zero: 0 },
        file_name: name,
        origin: 'imported',
      },
    ];
    return name;
  },

  dictDelete(fileName: string) {
    mockDicts = mockDicts.filter((item) => item.file_name !== fileName);
  },

  tableList(): TableItem[] {
    return mockTables.map((table) => ({ ...table, active: table.name === mockActiveTable }));
  },

  activateLibraryTable(name: string): Result<Meta> {
    const item = mockTables.find((table) => table.name === name);
    if (!item) return { ok: false, error: `数据文件夹里没有表「${name}」` };
    if (!item.meta) return { ok: false, error: item.error ?? t('bridge.noMeta') };
    mockActiveTable = name;
    for (const table of mockTables) table.active = table.name === name;
    return { ok: true, data: item.meta };
  },

  tableDelete(name: string) {
    mockTables = mockTables.filter((table) => table.name !== name);
    if (mockActiveTable === name) mockActiveTable = null;
  },

  suggestTableDir(name: string): string {
    return `${MOCK_TABLES_DIR}\\${sanitizeMockTableName(name)}`;
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
      top_pct: pctForRank(entry.rank, base.length),
      entries: base.length,
      scope: FULL_SCOPE,
      in_dict: true,
    };
  },

  listRank(domain: string | null, kind: 'word' | 'char', from: number, limit: number): RankRow[] {
    const base = kind === 'char' ? MOCK_CHARS : MOCK_WORDS;
    const source = domain ? domainEntries(base, domain, kind) : base;
    const total = Math.max(1, source.reduce((sum, e) => sum + e.count, 0));
    return source
      .slice(Math.max(0, from - 1), Math.max(0, from - 1) + limit)
      .map((entry) => ({
        rank: entry.rank,
        word: entry.word,
        count: entry.count,
        flags: 1,
        top_pct: pctForRank(entry.rank, source.length),
        pct: (entry.count * 100) / total,
      }));
  },

  /**
   * 演示用的覆盖率曲线：与 Rust 侧一样按对数间隔采样 `(rank, 累计覆盖率)`，
   * 且最后一点一定落在 `entries` 上（覆盖率 = 1），这样前端按点累加就能得到
   * 精确的累计覆盖率。
   */
  tierCurve(path: string, maxPoints: number): TierCurve {
    const meta = buildMockMeta('D:\\corpus');
    const { scope, kind: wantKind } = splitTableKey(path);
    const table =
      meta.tables.find((entry) => entry.path === scope && entry.kind === wantKind) ??
      meta.tables.find((entry) => entry.kind === wantKind) ??
      meta.tables[0];
    const kind: 'word' | 'char' = table.kind === 'char' ? 'char' : 'word';
    const base = kind === 'char' ? MOCK_CHARS : MOCK_WORDS;
    // 非 full 的表组 = 表组演示数据（铺平之后表组名就是子目录名）
    const domain = table.path === FULL_SCOPE ? null : table.path;
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
    if (n === 0) {
      return { path: tableKey(table.path, table.kind), kind, entries: 0, total_tokens: 0, points };
    }
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

    return {
      path: tableKey(table.path, table.kind),
      kind,
      entries: n,
      total_tokens: total,
      points,
    };
  },

  searchWords(query: string, kind: 'word' | 'char', limit: number): RankRow[] {
    if (!query) return [];
    const base = kind === 'char' ? MOCK_CHARS : MOCK_WORDS;
    const total = Math.max(1, base.reduce((sum, e) => sum + e.count, 0));
    return base
      .filter((entry) => entry.word.startsWith(query))
      .slice(0, limit)
      .map((entry) => ({
        rank: entry.rank,
        word: entry.word,
        count: entry.count,
        flags: 1,
        top_pct: pctForRank(entry.rank, base.length),
        pct: (entry.count * 100) / total,
      }));
  },

  /**
   * 演示「相加」：不写任何文件，只往当前演示产物的 `tables` 里追加一条记录，
   * 让界面上的新表组、可设为主表、来源标记这些都能点出来。
   */
  composeTables(params: ComposeParams): ComposedTable[] {
    const meta = MOCK.activeDataset();
    if (!meta) return [];
    const kind: 'word' | 'char' = params.kind === 'char' ? 'char' : 'word';
    const sources = params.sources.filter((s) => splitTableKey(s).kind === kind);
    if (sources.length === 0) return [];
    const scope = params.scope.trim() || '相加';
    const base = kind === 'char' ? MOCK_CHARS : MOCK_WORDS;
    const total = base.reduce((sum, e) => sum + e.count, 0) * sources.length;
    const already = meta.tables.some((t) => t.path === scope && t.kind === kind);
    if (already) return [];
    meta.tables.push(mockTableMeta(scope, kind, base, mockTierStats(base, kind)));
    const added = meta.tables[meta.tables.length - 1];
    added.total_tokens = total;
    added.source_tables = [...sources];
    return [
      {
        out: MOCK_DIR,
        scope,
        kind,
        entries: base.length,
        total_tokens: total,
        bytes: added.vfr_bytes,
        sources,
        elapsed_ms: 1200,
      },
    ];
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

  /** 模拟一次完整扫描：日志 → 探测 → 表组进度 → 各表结果 → 完成 */
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
    if (MOCK_CAPTURE_FAILED) return ''; // 预览「取词失败」那一屏
    if (mockPendingTaken) return '';
    mockPendingTaken = true;
    return '语言统计可以帮助我们理解文本的词汇分布，覆盖率与排名说明了用词难度。';
  },

  /**
   * 取词失败原因的示例。
   *
   * 浏览器预览里没有真实取词，但**必须**能预览到这个错误态：它是小窗最容易出
   * 布局问题的一屏（长中文串 + 窄窗口），不能只在真实 Tauri 里才看得到。
   * 用 `?window=popup&capture=failed` 打开即可看到（见 MOCK_CAPTURE_FAILED）。
   */
  captureNote(): string {
    return MOCK_CAPTURE_FAILED
      ? '没有取到选中内容：Ctrl+C 没有被目标程序响应。常见原因是两者权限级别不一致，或目标程序不支持复制。可以点下面的输入框直接手输要查的词。'
      : '';
  },
};

/** 供页面判断行的 flags 语义（避免页面自己解析位运算） */
export function rowInDict(flags: number): boolean {
  return inDictFromFlags(flags);
}
