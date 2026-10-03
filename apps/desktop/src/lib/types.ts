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

import type { MessageKey } from './messages';

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

/** 单个表组的探测结果 */
export type DomainPlan = {
  name: string;
  files: number;
  bytes: number;
  /** 该表组命中的解析规则说明（给人看的） */
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

/** 一张结果表（**一个表组的一种类型**：`full` 的词频表、`news` 的字表…） */
export type TableMeta = {
  /**
   * **表组 id**（不是路径），同时是产物目录下的子目录名：`full`、`news`、
   * `相加：财经`。
   *
   * ⚠ schema v3 之前这里存的是 `full/word`、`domains/news/word` 这种把表组与
   * 类型糊在一起的"路径"。现在类型在 [`kind`](#kind) 里，两者合起来才是表身份
   * —— 拼字符串一律走 [`tableKey`](#tableKey)，别自己拼。
   */
  path: string;
  /** `word` | `char` */
  kind: string;
  entries: number;
  total_tokens: number;
  vfr_bytes: number;
  tiers: Tier[];
  tier_stats: TierStat[];
  /** 建这张表时的低频过滤阈值（相加时要沿用，否则与全量重扫的结果对不上） */
  min_count?: number;
  /**
   * 七组的**前%上界**（0..100，6 个），第 7 组是「以上全部」。
   *
   * 这是稀有度的**默认口径**：`前% = 排名 ÷ 总条目数 × 100`。缺字段时用
   * [`DEFAULT_WORD_TIER_PCT`](#DEFAULT_WORD_TIER_PCT) / 字表那套兜底。
   */
  tier_pct?: number[];
  /** 这张表是把哪些表相加出来的；空 = 扫描出来的原始表 */
  source_tables?: string[];
};

/** 表身份：`表组/类型`。**全前端只有这一处**拼这个字符串。 */
export function tableKey(scope: string, kind: string): string {
  return `${scope}/${kind}`;
}

/** 拆开表身份。表组自身可能含 `/`，所以从**右边**切最后一个。 */
export function splitTableKey(key: string): { scope: string; kind: string } {
  const i = key.lastIndexOf('/');
  if (i < 0) return { scope: key, kind: '' };
  return { scope: key.slice(0, i), kind: key.slice(i + 1) };
}

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
  /**
   * **v2**：词典链，**按装载顺序** —— `dicts[0]` 是主词典，其后都是叠加词典。
   *
   * 每份都带 `sha256`，所以读取方能回答「这张表是不是还配得上当前这份词典」。
   * 词典外置之后这是唯一权威的词典记录；`legacy_dict` / `user_dict` 只是老产物残留。
   *
   * 老产物（`schema_version = 1`）没有这个字段 → 空数组，**不要**据此断定「没有词典」。
   */
  dicts: DictRef[];
  /**
   * **v1 老字段，只读不写**：老产物把主词典记成一句自由文本标签
   * （`"dict": "builtin(jieba dict.txt, 349046 entries)"`）。
   *
   * 新产物永不写它。读取时按 `DictRef` 处理（Rust 侧的反序列化接受纯字符串，
   * 会降级成「有名字但不可校验」）。
   */
  dict?: DictRef | null;
  /** **v1 老字段，只读不写**：老产物那个「叠加用户词典」的单个绝对路径 */
  user_dict?: string | null;
  min_len: number;
  max_len: number;
  keep_latin: boolean;
  keep_digit: boolean;
  skip_single_char: boolean;
};

/**
 * 词典链里实际生效的那几份（优先 `dicts`，为空时回退到两个 v1 老字段）。
 *
 * 与 Rust 侧 `vocfreq_core::artifact::TokenizerMeta::resolved_dicts()` 等价。
 */
export function resolvedDicts(tokenizer: TokenizerMeta | null | undefined): DictRef[] {
  if (!tokenizer) return [];
  if (tokenizer.dicts && tokenizer.dicts.length > 0) return tokenizer.dicts;
  const out: DictRef[] = [];
  if (tokenizer.dict) out.push(tokenizer.dict);
  if (tokenizer.user_dict) {
    out.push({ id: '', name: tokenizer.user_dict, path: '', entries: 0, sha256: '' });
  }
  return out;
}

/** `meta.json` 的完整结构 */
export type Meta = {
  schema_version: number;
  generated_at: string;
  tool_version: string;
  corpus_root: string;
  elapsed_ms: number;
  tokenizer: TokenizerMeta;
  totals: Totals;
  /**
   * 建这张表时语料库里**扫到的切片**（一级子目录）及其文件数、体积。
   *
   * ⚠ 这是「**语料切片**的清单」，不是「表清单」—— 表现在看 `tables`。
   * 两者从前是同一件事，schema v3 起不是了：
   *
   * - 「生成词频表」页的扫描计划显示的是**语料切片**；
   * - 「划句分析」「排行榜」「表管理」的表组则取自 [`Meta.scopes`](#scopes)（表）。
   */
  domains: { name: string; files: number; bytes: number }[];
  tables: TableMeta[];
  /**
   * 七组的展示名（中文）：`["极多","很多","较多","中等","较少","很少","极少"]`。
   *
   * 仅供展示与兜底 —— **不要当身份用**（会随界面语言变化）。判断分组一律用组号
   * （`tokens[].tier`）或稳定标识（`tier_keys`）。
   */
  tier_names: string[];
  /**
   * 七组的稳定标识，与 `tier_names` / `tiers` / `tier_stats` 同序，例如
   * `["very_common","common","fairly_common","medium","fairly_rare","rare","very_rare"]`。
   *
   * 界面据此把组号映射到配色与本地化文案。**老产物没有这个字段**（后端
   * `#[serde(default)]` → 缺失/空数组），此时按位置对齐、用前端常量兜底。
   */
  tier_keys?: string[];
};

/**
 * 全部**表组**（表侧），去重后「`full` 优先，其余按名字」排序。
 *
 * 与 Rust 侧 `Meta::scopes()` 等价。注意别和 [`Meta.domains`](#domains) 混了：
 * 那个是**语料切片**，这个是**表**。
 */
export function metaScopes(meta: Meta | null | undefined): string[] {
  if (!meta) return [];
  const names: string[] = [];
  for (const t of meta.tables) {
    if (!names.includes(t.path)) names.push(t.path);
  }
  return names.sort((a, b) => {
    const rank = (s: string) => (s === FULL_SCOPE ? 0 : 1);
    return rank(a) - rank(b) || a.localeCompare(b);
  });
}

/** 全量语料对应的表组 id。它**不是特权表组**，只是"所有表组加一起"那一份。 */
export const FULL_SCOPE = 'full';

/** 找一张表：表组 + 类型。 */
export function findTableMeta(
  meta: Meta | null | undefined,
  scope: string,
  kind: 'word' | 'char'
): TableMeta | undefined {
  return meta?.tables.find((t) => t.path === scope && t.kind === kind);
}

/** 按表身份（`表组/类型`）找一张表。 */
export function findTableMetaByKey(
  meta: Meta | null | undefined,
  key: string
): TableMeta | undefined {
  const { scope, kind } = splitTableKey(key);
  return meta?.tables.find((t) => t.path === scope && t.kind === kind);
}

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
 * 采样在头部密、尾部疏（对数间隔），全库词频表大约 500 个点。
 * 前端用它把「目标累计覆盖率」反解成「排名上界」（见 format.ts 的
 * `rankForCoverage`，与 Rust 侧 `vocfreq_core::query::rank_for_coverage` 等价）。
 */
export type TierCurve = {
  /** 表身份，与 `meta.tables` 的 `tableKey(path, kind)` 同一套取值 */
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

/** 某个 token 在某张表里的排名（划句分析里的「各表对比」一列） */
export type TableRank = {
  /** 表组：`full`、`news`、`相加：财经`… */
  scope: string;
  /** `word` | `char` */
  kind: string;
  /** 该表里的排名；null = 这张表没收录这个词 */
  rank: number | null;
  /** 该表里的前%（排名 ÷ 条目数 × 100） */
  top_pct: number | null;
  /** 该表的条目数 */
  entries: number;
  /** 该表里的频次 */
  count: number | null;
};

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
  /**
   * **前%**：`排名 ÷ 主表条目数 × 100`，也就是「这个词排在前百分之几」。
   *
   * 这是稀有度的默认口径。与 `pct` 完全是两回事：`pct` 是"它占了多少正文"，
   * `top_pct` 是"它比多少词常见"。
   */
  top_pct: number | null;
  /** 主表的总条目数（前% 的分母） */
  entries: number | null;
  /** 组号 0..6，与 `tier_keys` / `tier_names` 同序；null = 语料库未收录 */
  tier: number | null;
  /** 展示用组名（后端按 meta 默认阈值给的）；判断分组请用 `tier` 或 `tier_keys` */
  tier_name: string | null;
  in_dict: boolean | null;
  from_user: boolean | null;
  /** 各表组（表）里的排名对比 */
  table_ranks: TableRank[];
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
  /** 占全部 token 的百分比 */
  pct: number;
  /** 前%（见 [`TokenInfo.top_pct`](#top_pct)） */
  top_pct: number;
  /** 查的那张表的条目数 */
  entries: number;
  /** 查的是哪个表组的表 */
  scope: string;
  in_dict: boolean;
};

/** `invoke('list_rank', { domain, kind, from, limit })` */
export type RankRow = {
  rank: number;
  word: string;
  count: number;
  flags: number;
  /** 前%（排名 ÷ 条目数 × 100） */
  top_pct: number;
  /** 该词频次占全表 token 的百分比 */
  pct: number;
};

/** `invoke('search_words', { query, kind, domain, limit })` */
export type SearchRow = RankRow;

/** `invoke('open_dataset', { dir })` —— 把已有产物目录装入后端缓存，失败返回中文错误 */
export type DatasetHandle = Meta;

// ---------------------------------------------------------------------------
// 数据文件夹：词典（dicts\）与词频表（tables\）
//
// 后端约定的字段名是**出参 snake_case、入参 camelCase**，所以本区块的类型
// （都是命令的返回值）一律 snake_case，只有 `Origin` 的取值是字面量字符串。
// ---------------------------------------------------------------------------

/**
 * 这个词典 / 词频表是怎么来的。**纯展示用**，不挂任何行为 —— 预置项同样可以删。
 *
 * 取值与 Rust 侧 `library::Origin`（`#[serde(rename_all = "snake_case")]`）一致。
 */
export type Origin = 'seeded' | 'imported' | 'scanned' | 'unknown';

/** 读一份词典时的统计（`dict_list()` 每项的 `report`） */
export type DictLoadReport = {
  /** 有效词条数 */
  entries: number;
  /** 跳过的注释行数 */
  comments: number;
  /** 跳过的空行数 */
  blanks: number;
  /** 省略了词频列、装载时按 `suggest_freq` 折算的条目数 */
  freq_omitted: number;
  /**
   * 显式写了 `0` 的条目数。
   *
   * jieba 格式第二列是**概率权重**，写成 0 的词**永远切不出来**，所以这类条目
   * 要在界面上当成隐患提示，不能只算进 `entries` 里。
   */
  freq_zero: number;
};

/** 一份词典的身份（写进 `meta.json` 的 `tokenizer.dicts[]`，也在 `DictItem.dict` 里） */
export type DictRef = {
  /** 稳定标识：文件名去掉 `.dict` */
  id: string;
  /** 展示名 */
  name: string;
  /** 装载时的绝对路径（换机器就失效，仅供排查） */
  path: string;
  /** 实际装载的有效词条数 */
  entries: number;
  /** 内容 sha256（小写十六进制）；v1 老产物是空串 = 不可校验 */
  sha256: string;
};

/** `invoke('dict_list')` 的一项 */
export type DictItem = {
  dict: DictRef;
  /** 读取失败时的原因；**读不了的文件也会列出来**，界面要显眼提示 */
  error: string | null;
  report: DictLoadReport;
  /** `dicts\` 下的文件名，改名 / 删除都靠它定位 */
  file_name: string;
  origin: Origin;
};

/** 表与它记录的词典链之间的绑定状态（`#[serde(tag = "kind")]` 判别联合） */
export type Binding =
  | { kind: 'ok' }
  /** v1 老产物：没有指纹，**无从判断**（不是错误，但也没法说它一致） */
  | { kind: 'legacy' }
  /** 找到了但内容变了（同一个词典被改过）→ 频次可能不准 */
  | { kind: 'drifted'; changed: string[] }
  /** 找不到了 → 频次不可信 */
  | { kind: 'missing'; missing: string[] };

/** `invoke('table_list')` 的一项 */
export type TableItem = {
  name: string;
  /** 产物目录的绝对路径 */
  path: string;
  /** `meta.json` 解析成功时才有 */
  meta: Meta | null;
  /** 读不了的表也会列出来（目录被删了一半、meta.json 坏了…） */
  error: string | null;
  binding: Binding;
  /** 是不是当前激活的那张 */
  active: boolean;
  /**
   * 是不是住在数据文件夹的 `tables\` 里。
   *
   * `false` = 指向数据文件夹之外的既有产物目录，这类表**不给删除按钮**。
   */
  in_library: boolean;
  origin: Origin;
};

/** `invoke('library_info')` —— 数据文件夹的整体状况 */
export type LibraryInfo = {
  root: string;
  dicts_dir: string;
  tables_dir: string;
  /** 是不是默认位置 */
  is_default: boolean;
  /** 当前激活的表在数据文件夹里时的目录名 */
  active_table: string | null;
  /** 当前激活的表在数据文件夹之外时的绝对路径 */
  active_table_path: string | null;
  /** 当前激活那张表的词典绑定状态 */
  active_binding: Binding | null;
  /** 打开激活表时攒下的告警（v1 老产物、词典缺失后退化…），界面应直接显示 */
  active_warnings: string[];
};

/** 该表的词条数：取 `meta.tables` 里 `kind === 'word'` 的那张（与后端口径一致） */
export function wordEntriesOf(meta: Meta | null | undefined): number | null {
  if (!meta) return null;
  const table = meta.tables.find((entry) => entry.kind === 'word');
  return table ? table.entries : null;
}

// ---------------------------------------------------------------------------
// 扫描任务
// ---------------------------------------------------------------------------

/** `invoke('start_scan', { params })` 的参数 */
export type ScanParams = {
  corpus: string;
  out: string;
  /**
   * 用数据文件夹 `dicts\` 里的哪几个词典，**按顺序、第一个是主词典**。
   *
   * 空数组 = 「目录里全部 `.dict`，按文件名排序」。顺序会影响同名条目的覆盖结果，
   * 所以界面上的顺序是有意义的，不是随便排的。
   */
  dictFiles: string[];
  threads: number;
  hmm: boolean;
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
 *   - `top_pct`：按**前%**（`排名 ÷ 条目数`）—— **默认**
 *   - `rank`：按绝对排名
 *   - `coverage`：按累计覆盖率（用 tier_curve 反解成排名）
 *   - `even`：按词条数七等分
 *
 * 默认是前% 而不是绝对排名：排名绝对值只在**同一张表内**可比。表组只有几万条、
 * 全量表有几百万条，同一个绝对阈值套上去会让表组整片挤进「极多」；而铺平之后
 * 任意表组都能当主表，绝对阈值必然失真。
 */
export type TierMethod = 'top_pct' | 'rank' | 'coverage' | 'even';

/** 七组 = 6 个上界 + 「以上全部」 */
export const TIER_COUNT = 7;
/** 需要用户填写的阈值个数（第 7 组自动是无穷） */
export const TIER_BOUND_COUNT = TIER_COUNT - 1;

/**
 * 词频表默认的 6 个前%上界（0..100）。
 *
 * 与 Rust 侧 `rank::DEFAULT_TIER_PCT` **必须一致**。数值不是新拍的：它们是把旧的
 * 绝对排名默认值（100/1000/5000/20000/50000/150000）放在实测的 380 万条全库词频表上
 * 换算出来的，所以换成前%之后色阶观感与从前基本一致。
 */
export const DEFAULT_WORD_TIER_PCT = [0.0026, 0.026, 0.132, 0.526, 1.32, 3.95];
/** 字表默认的 6 个前%上界（字表只有约 1.9 万字，套词频表那套会全部挤进头两档） */
export const DEFAULT_CHAR_TIER_PCT = [0.26, 1.05, 3.16, 7.89, 15.8, 26.3];

export function defaultTierPct(kind: 'word' | 'char'): number[] {
  return [...(kind === 'char' ? DEFAULT_CHAR_TIER_PCT : DEFAULT_WORD_TIER_PCT)];
}

export function isTierMethod(value: unknown): value is TierMethod {
  return value === 'top_pct' || value === 'rank' || value === 'coverage' || value === 'even';
}

/** `invoke('get_settings')` / `invoke('set_settings', { settings })` */
export type Settings = {
  corpusDir: string | null;
  /**
   * **数据文件夹**：里面是 `dicts\`（词典库）与 `tables\`（词频表库）两个子目录。
   *
   * ⚠ 语义变过一次：从前它直接指向**一个词频表产物目录**（那目录里就有
   * `meta.json`）。老设置会在启动时被后端识别出来、转成一张「已注册的表」。
   */
  dataDir: string | null;
  hotkey: string;
  popupWidth: number;
  popupHeight: number;
  popupOpacity: number;
  popupAlwaysOnTop: boolean;
  popupAutoCloseMs: number;
  /**
   * **预留**：关闭主窗口时收进系统托盘而非退出应用。当前固定为 `true`，
   * 设置页尚未提供开关；此字段仅保持前后端契约一致，暂不改写。
   */
  closeToTray: boolean;
  theme: string;
  /**
   * 界面语言（BCP 47 形式，例如 `zh-CN`）。
   *
   * 与 `theme` 完全同构：前端还会在 localStorage 里镜像一份用于首屏同步读取，
   * 但**权威值是这里**，保证主窗口与悬浮小窗一致。
   */
  locale: string;
  threads: number;
  hmm: boolean;
  keepDigit: boolean;
  keepLatin: boolean;
  skipSingleChar: boolean;
  /**
   * 扫描用哪条词典链：`dicts\` 下的**文件名**，按顺序、**第一个是主词典**。
   *
   * `null` 或空数组 = 用数据文件夹里全部 `.dict`（按文件名排序）。
   * 存文件名而不是绝对路径：数据文件夹是可以搬走的，文件名不会因此失效。
   */
  scanDicts?: string[] | null;
  /**
   * **v1 兼容、已废弃**：从前那个「叠加用户词典」的单个绝对路径。
   *
   * 现在词典是数据文件夹里的条目、在扫描时勾选（见 `scanDicts`）。后端只在读老
   * 设置时才有这个字段，迁移时会把它并进 `scanDicts` 并置空。前端**不再写它**。
   */
  userDict?: string | null;
  /** 当前激活的表在数据文件夹 `tables\` 下的目录名 */
  activeTable?: string | null;
  /** 当前激活的表**不在**数据文件夹里时，存它的绝对路径（与 `activeTable` 互斥） */
  activeTablePath?: string | null;
  minCount: number;
  skipDomainTables: boolean;
  /**
   * **主表组**：决定"这个词/字有多常见"的那一张表。
   *
   * 现在所有表组完全平等（`full` 只是"全部相加"的那一个），这个设置是唯一的
   * 特权：划句分析的着色与分组、排行榜、分组阈值预览都以它为准，其余表组只做
   * 对比。存的是**纯表组名**（`full`、`news`、`相加：财经`）。
   *
   * `null` / 空串 = 用 `full`；产物里没有 `full` 就用排序后的第一个表组。
   */
  primaryScope?: string | null;
  /**
   * **已废弃**：从前用它挑"参与表组对比与排行榜"的表。
   *
   * 铺平之后不需要了（任何表组都能当主表、都能相加，对比列表也一律全给）。
   * 留着只是为了读得进老设置文件，**不再有任何行为**。
   */
  enabledTables: string[] | null;
  /** 分组方法，默认 `'top_pct'`（按前%） */
  tierMethod: TierMethod;
  /** 自定义词频表阈值：6 个排名上界；null = 用 meta 里的默认值 */
  tierWordBounds: number[] | null;
  /** 自定义字表阈值：同上 */
  tierCharBounds: number[] | null;
  /** `tierMethod = 'coverage'` 时的 6 个累计覆盖率目标（0..1，严格递增） */
  tierCoverage: number[] | null;
  /** `tierMethod = 'top_pct'` 时的 6 个**前%上界**（0..100）；null = 用产物里记的默认口径 */
  tierPct: number[] | null;
};

// ---------------------------------------------------------------------------
// 界面辅助类型
// ---------------------------------------------------------------------------

/** 表格列的对齐方式 */
export type ColumnAlign = 'left' | 'right';

/**
 * 生效分组阈值的**回退警告** —— 这是**错误码，不是文案**。
 *
 * `key` 直接就是消息表的 key，`params` 是插值参数；展示层写 `t(key, params)` 即可。
 *
 * 为什么不让 `format.ts::boundsInfo` 直接返回字符串：那个函数是**纯计算**函数
 * （被大量 `$derived` 调用、也被非组件代码复用），一旦调 `t()` 就会依赖界面语言，
 * 返回值随语言漂移，缓存与等价性判断全部失效。所以计算层只回码，文案层去翻。
 */
export type BoundsWarning = {
  key: MessageKey;
  params?: Record<string, string | number>;
};

/** 排行榜的通用行（RankRow / SearchRow / WordHit 归一化后用同一套渲染） */
export type LeaderboardRow = {
  rank: number;
  word: string;
  count: number;
  flags: number;
  pct: number | null;
  /** 前%（排名 ÷ 条目数 × 100） */
  top_pct: number | null;
  tier: number | null;
  tier_name: string | null;
};

// ---------------------------------------------------------------------------
// 相加（词频表加法）
// ---------------------------------------------------------------------------

/** `invoke('compose_tables', { params })` 的参数 */
export type ComposeParams = {
  /** 源表身份（`表组/类型`），至少一个 */
  sources: string[];
  /** 新表组的名字（会变成同一份产物里的另一个表组目录） */
  scope: string;
  /** 只要这一类；不给 = 源表里出现过的每一类都相加 */
  kind?: string | null;
};

/** `compose_tables` 的返回：每一类表一条 */
export type ComposedTable = {
  out: string;
  scope: string;
  kind: string;
  entries: number;
  total_tokens: number;
  bytes: number;
  sources: string[];
  elapsed_ms: number;
};
