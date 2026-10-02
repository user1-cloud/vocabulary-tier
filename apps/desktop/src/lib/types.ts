/**
 * 共享领域类型 —— 与 Rust 侧（apps/desktop/src-tauri）的 serde 结构严格一一对应。
 *
 * 约定：
 *   - 字段名保持 Rust 的 snake_case，不做 camelCase 转换。
 *     这样 bridge.ts 里不需要写映射层，Rust 侧调整字段时前端报错位置也最直观。
 *   - 所有属性访问一律用 `obj['字段名']` 或 `obj.字段名`（两种写法在本文件里都合法）。
 *   - 所有命令都返回 `Result<T, String>`，错误信息是中文；前端的包装类型见
 *     `$lib/api/bridge` 里的 `Result<T>`。
 */

// ---------------------------------------------------------------------------
// 应用信息
// ---------------------------------------------------------------------------

/** `invoke('app_info')` */
export type AppInfo = {
  name: string;
  version: string;
  tauri_version: string;
  /** 底层 vocfreq-core 的版本（老版本后端可能没有这个字段，读取时做兜底） */
  core_version: string;
};

// ---------------------------------------------------------------------------
// 语料库探测
// ---------------------------------------------------------------------------

/** 单个分域的探测结果 */
export type DomainPlan = {
  name: string;
  files: number;
  bytes: number;
  /** 该分域命中的解析规则说明（给人看的） */
  rules: string[];
};

/** `invoke('plan_corpus', { corpus })` */
export type CorpusPlan = {
  corpus: string;
  files: number;
  bytes: number;
  domains: DomainPlan[];
  /** 探测过程中的警告（字段可能缺失） */
  warnings?: string[];
};

// ---------------------------------------------------------------------------
// 产物元数据（meta.json）
// ---------------------------------------------------------------------------

/** 一个分组（档位）：排名上界（含），最后一组的 max_rank 是 u64::MAX */
export type Tier = {
  name: string;
  max_rank: number;
};

/** 每组的词条数与累计覆盖率 */
export type TierStat = {
  name: string;
  max_rank: number;
  entries: number;
  tokens: number;
  /** 该组 token 占全部 token 的比例（0..1） */
  coverage: number;
  /** 截至该组的累计覆盖率（0..1） */
  cumulative: number;
};

/** 一张结果表（全库或分域） */
export type TableMeta = {
  /** 相对输出目录的路径，不含扩展名，例如 `full/word` */
  path: string;
  /** `word` | `char` */
  kind: string;
  entries: number;
  total_tokens: number;
  vfr_bytes: number;
  tiers: Tier[];
  tier_stats: TierStat[];
};

/** 扫描累计统计量 */
export type Totals = {
  files: number;
  bytes: number;
  lines: number;
  paras: number;
  tokens: number;
  /** UTF-8 非法或 JSON 解析失败、被跳过的行数 */
  bad_lines: number;
};

/** 分词器元信息 */
export type TokenizerMeta = {
  engine: string;
  version: string;
  hmm: boolean;
  dict: string;
  user_dict: string | null;
  min_len: number;
  max_len: number;
  keep_latin: boolean;
  keep_digit: boolean;
  skip_single_char: boolean;
};

/** `meta.json` 的完整结构 */
export type Meta = {
  schema_version: number;
  generated_at: string;
  tool_version: string;
  corpus_root: string;
  elapsed_ms: number;
  tokenizer: TokenizerMeta;
  totals: Totals;
  domains: { name: string; files: number; bytes: number }[];
  tables: TableMeta[];
  /** ["极多","很多","较多","中等","较少","很少","极少"] */
  tier_names: string[];
};

/** `invoke('dataset_status', { dir })` —— 不抛错，exists=false 时 meta=null */
export type DatasetStatus = {
  dir: string;
  exists: boolean;
  meta: Meta | null;
};

// ---------------------------------------------------------------------------
// 覆盖率曲线（tier_curve）
// ---------------------------------------------------------------------------

/**
 * `invoke('tier_curve', { path, dir, maxPoints })` 的返回类型。
 *
 * `points` 是 `[rank, 累计覆盖率 0..1]`，rank 严格递增、覆盖率单调不减；
 * 采样在头部密、尾部疏（对数间隔），全库词表大约 500 个点。
 * 前端用它把「目标累计覆盖率」反解成「排名上界」（见 format.ts 的
 * `rankForCoverage`，与 Rust 侧 `vocfreq_core::query::rank_for_coverage` 等价）。
 */
export type TierCurve = {
  /** 表路径，与 `meta.tables[].path` 同一套取值：`full/word`、`domains/news/char`… */
  path: string;
  /** `word` | `char` */
  kind: string;
  entries: number;
  total_tokens: number;
  points: [number, number][];
};

// ---------------------------------------------------------------------------
// 划句分析
// ---------------------------------------------------------------------------

/** 一个 token 的完整频率信息 */
export type TokenInfo = {
  text: string;
  byte_start: number;
  byte_end: number;
  /** false = 标点/空白，不参与统计，但仍要按原样渲染 */
  accepted: boolean;
  /** true = 查的是字表 */
  single_cjk: boolean;
  /** "word" | "char" | "" */
  table: string;
  count: number | null;
  rank: number | null;
  /** 占全部 token 的百分比（0..100），与 Rust 侧口径一致 */
  pct: number | null;
  /** 0..6 对应 tier_names；null = 语料库未收录 */
  tier: number | null;
  tier_name: string | null;
  in_dict: boolean | null;
  from_user: boolean | null;
  /** 各分域里的排名 */
  domain_ranks: [string, number | null][];
};

/**
 * `invoke('analyze_text', { text, domains })` 的返回类型。
 *
 * 冻结接口里写的是 `TokenInfo[]`（顶层数组）。为了兼容后端可能包成
 * `{ tokens: [...] }` 的写法，bridge.ts 里做了归一化，但对外统一暴露数组。
 */
export type AnalyzeResult = TokenInfo[];

// ---------------------------------------------------------------------------
// 排行榜
// ---------------------------------------------------------------------------

/** `invoke('lookup_word', { word })` */
export type WordHit = {
  word: string;
  count: number;
  rank: number;
  /** bit0 = 在 jieba 词典内，bit1 = 来自用户词典 */
  flags: number;
  tier: number;
  tier_name: string;
  pct: number;
  in_dict: boolean;
};

/** `invoke('list_rank', { domain, kind, from, limit })` */
export type RankRow = {
  rank: number;
  word: string;
  count: number;
  flags: number;
};

/** `invoke('search_words', { query, kind, domain, limit })` */
export type SearchRow = RankRow;

/** `invoke('open_dataset', { dir })` —— 把已有产物目录装入后端缓存，失败返回中文错误 */
export type DatasetHandle = Meta;

// ---------------------------------------------------------------------------
// 扫描任务
// ---------------------------------------------------------------------------

/** `invoke('start_scan', { params })` 的参数 */
export type ScanParams = {
  corpus: string;
  out: string;
  threads: number;
  hmm: boolean;
  userDict: string | null;
  minCount: number;
  keepDigit: boolean;
  keepLatin: boolean;
  skipSingleChar: boolean;
  onlyDomains: string[];
  skipDomainTables: boolean;
  writeTsv: boolean;
};

/** `scan:progress` 事件的 payload（判别联合） */
export type ScanProgress =
  | { event: 'log'; level: string; message: string }
  | {
      event: 'plan';
      files: number;
      bytes: number;
      domains: { name: string; files: number; bytes: number }[];
      warnings: string[];
    }
  | {
      event: 'phase';
      phase: string;
      domain: string;
      units_done: number;
      units_total: number;
      bytes_done: number;
      bytes_total: number;
      percent: number;
    }
  | { event: 'table'; table: string; entries: number; total_tokens: number; tier_stats: TierStat[] };

/** 日志级别（用于着色，未知级别走默认样式） */
export type LogLevel = 'info' | 'warn' | 'error' | 'debug' | string;

// ---------------------------------------------------------------------------
// 设置
// ---------------------------------------------------------------------------

/**
 * 分组方法：
 *   - `rank`：按绝对排名（默认）
 *   - `coverage`：按累计覆盖率（用 tier_curve 反解成排名）
 *   - `even`：按词条数七等分
 */
export type TierMethod = 'rank' | 'coverage' | 'even';

/** 七组 = 6 个上界 + 「以上全部」 */
export const TIER_COUNT = 7;
/** 需要用户填写的阈值个数（第 7 组自动是无穷） */
export const TIER_BOUND_COUNT = TIER_COUNT - 1;

export function isTierMethod(value: unknown): value is TierMethod {
  return value === 'rank' || value === 'coverage' || value === 'even';
}

/** `invoke('get_settings')` / `invoke('set_settings', { settings })` */
export type Settings = {
  corpusDir: string | null;
  dataDir: string | null;
  hotkey: string;
  popupWidth: number;
  popupHeight: number;
  popupOpacity: number;
  popupAlwaysOnTop: boolean;
  popupAutoCloseMs: number;
  theme: string;
  threads: number;
  hmm: boolean;
  keepDigit: boolean;
  keepLatin: boolean;
  skipSingleChar: boolean;
  userDict: string | null;
  minCount: number;
  skipDomainTables: boolean;
  /**
   * 参与「分域对比与排行榜」的表；空数组或 null = 全部启用。
   *
   * 取值是 `meta.tables[].path`（`full/word`、`domains/news/word`…）。
   * 注意：全库表（`full/word`、`full/char`）始终用于「单个词/字的总体频率」查询，
   * 关掉它只会让它不参与分域对比与排行榜，否则划句分析会整片变成「未收录」。
   */
  enabledTables: string[] | null;
  /** 分组方法，默认 `'rank'` */
  tierMethod: TierMethod;
  /** 自定义词表阈值：6 个排名上界；null = 用 meta 里的默认值 */
  tierWordBounds: number[] | null;
  /** 自定义字表阈值：同上 */
  tierCharBounds: number[] | null;
  /** `tierMethod = 'coverage'` 时的 6 个累计覆盖率目标（0..1，严格递增） */
  tierCoverage: number[] | null;
};

// ---------------------------------------------------------------------------
// 界面辅助类型
// ---------------------------------------------------------------------------

/** 表格列的对齐方式 */
export type ColumnAlign = 'left' | 'right';

/** 排行榜的通用行（RankRow / SearchRow / WordHit 归一化后用同一套渲染） */
export type LeaderboardRow = {
  rank: number;
  word: string;
  count: number;
  flags: number;
  pct: number | null;
  tier: number | null;
  tier_name: string | null;
};
