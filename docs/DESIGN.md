# VocTier 设计文档

> 字词频率分析工具。两个交付物：一个纯 Rust 的语料库词频统计 CLI，一个 Tauri 桌面应用。

## 0. 实测基线（本机 Ryzen 9 8945HX 16C/32T + 31GB RAM + Samsung PM9A1 1TB NVMe）

这些数字是实测得来，不是估计，用来约束设计：

| 环节 | 实测结果 |
|---|---|
| 语料库规模 | 77 个 `.jsonl`，33.3 GB，5 种 JSON schema |
| 磁盘纯顺序读 | 33.27 GB / **13.1 s**（2600 MB/s，两遍一致） |
| 只用 serde_json 读+解析 | 800 MB/s（单线程） |
| 读+解析+jieba 分词 | 139.8 MB/s（单线程，wiki） |
| 单线程分词+计数 | 46~93 MB/s（随文本难度变化：news 46 / blog 64 / wiki 93） |
| 32 线程分词（缓存命中） | 最高 1391 MB/s |
| 32 线程分词+计数（互斥锁分片表） | 16 线程仅 501 MB/s，**32 线程倒退到 434 MB/s** |
| 32 线程分词+计数（**无锁每线程独占**） | 848 MB/s，单调扩展 |
| 排行 380 万词 + 写 138 MB TSV + 91 MB 索引 | 约 3 s |
| **全量统计端到端** | 33.3 GB / **约 90 s**（约 390 MB/s） |

> 端到端远低于磁盘的 2600 MB/s，也低于 32 线程的 CPU 上限——因为**多个并发读流会显著
> 压低这块盘的吞吐**。单流顺序读 2600 MB/s，但 32 线程开上百个流时只有约 400 MB/s。
> 16 MB 区块（全库 2000+ 流）实测比 64 MB 慢 60%，因此区块大小不要调小。

### 实测踩到的坑（正式实现必须遵守）

1. **语料含非法 UTF-8 的行**（爬取的维基数据）。用 `read_line` + `Err => break` 会让一行坏数据废掉整个区间（实测 4 线程有 2 个区间直接归零，丢掉 262 MB）。
   → 必须全程 `read_until(b'\n')` 按字节读行；解析用 `serde_json::from_slice`（零拷贝）快路径，失败再退 `String::from_utf8_lossy` 抢救该行；坏行**只跳过、绝不断流**，并统计坏行数。
2. **丢弃区间边界半行时必须复用同一个 `BufReader`**。若为丢弃另建一个 `BufReader`，它析构时会带走内部已缓冲但未消费的约 8 KB，文件位置前跳，导致每个区间开头读到一行截断数据（`bad_lines` 会正好等于区间数）。
   → 用 `BufReader::seek`（会自动清空内部缓冲）在同一个 reader 上定位。
3. **计数绝不能用互斥锁分片哈希表**。锁竞争 + 缓存行乒乓让 32 线程比 16 线程更慢。
   → 每线程独占 `FxHashMap`，扫描结束再归并（实测归并仅 0.15 s / 500MB）。
4. **`Cow<'de, str>` 不是零拷贝**。serde 对 `Cow` 只有一条实现：
   ```rust
   impl<'de, 'a, T> Deserialize<'de> for Cow<'a, T> {
       fn deserialize<D>(d: D) -> Result<Self, D::Error> {
           T::Owned::deserialize(d).map(Cow::Owned)   // 永远分配
       }
   }
   ```
   MNBVC 的段落数组是 `[{行号, 是否重复, …, 内容}, …]`，一个 1GB 文件里有上百万个元素、
   每个元素 5 个键。用 `Cow` 读键等于上千万次堆分配——实测这让 gov 域慢了 5.7 倍。
   → 用自定义的 `KeyStr`（`deserialize_str` + `visit_borrowed_str`）读键。
5. **不要在会阻塞的消费端上同步上报进度**。工作线程里直接写 stderr，一旦消费端变慢
   （比如被 PowerShell 管道背压），整个扫描会被连带拖住。
   → 诊断与进度统一走回调，由调用方决定丢弃、限速或异步写出；`--progress none` 只关进度、仍输出日志。

## 1. 目录结构

```
voctier/
├─ Cargo.toml                 # workspace（只含 crates/）
├─ crates/
│  ├─ vocfreq-core/           # 统计核心库：解析 / 清洗 / 分词 / 计数 / 排行 / 产物 / 查询
│  └─ vocfreq-cli/            # 纯 Rust CLI，产出 vocfreq.exe，不依赖 Tauri
└─ apps/desktop/              # Tauri v2 + Svelte 5 前端（独立 Cargo workspace）
   └─ src-tauri/              # 注意：用空 [workspace] 表与上层隔离
```

`crates/vocfreq-cli` 可单独分发，是"纯 rust 工具"；Tauri 通过子进程调用它。

## 2. 语料库 schema 与解析

实测只有 5 种 schema：

| 规则名 | 顶层字段 | 文本字段 | 覆盖 |
|---|---|---|---|
| `mnbvc_paragraph` | `段落`（数组） | `内容` | blog/book/wiki/news/gov 的绝大多数，含 warc/html 文件 |
| `mnbvc_paragraph_ext` | `段落`（数组） | `内容` | news/shenbao（多一个 `拓展字段`，是错别字，不影响解析） |
| `mnbvc_forum` | `回复`（数组） | `回复` | forum（含 HTML 标签，需清洗；**有空对象元素**） |
| `plain_text` | — | `text` | gov/GovReport |
| `parallel_subtitle` | — | `zh_text`，空则退 `cht_text` | parallel/subtitle |

- 前两种用同一条规则即可（`段落[].内容`），因此实际是 **4 条内置规则**。
- **自动探测**：按 top-level key 集合匹配，顺序 `段落` → `回复` → `zh_text` → `text`。
- **自定义规则**：`--rules rules.json` 传入规则数组，格式见 §7。
- 解析实现：通用 `DeserializeSeed`，只在命中目标字段时把字符串收进 `Vec<Cow<str>>`，其余走 `IgnoredAny`。无转义时零拷贝。

## 3. 分词与过滤

- jieba-rs 0.11，默认 `HMM = false`（`--hmm` 可开）。理由：HMM 会让同一实体在不同上下文被切成不同形态，拆散频次、使排行不可复现。实测关闭 HMM 时 jieba 只输出"词典命中的词 + 未命中的单字"。
- **词典可配置**（这是 jieba 的官方推荐做法）：
  - `--user-dict FILE`：叠加到内置 349,046 条词典上（jieba-rs `load_dict` 的语义就是"adding entries to the existing dictionary rather than resetting it"），格式 `词 词频 词性`，词频可省略。
  - `--dict FILE`：`Jieba::empty()` + `load_dict`，整个替换内置词典。
- **词典外词标记**：`jieba.has_word(w)` 是免费查询，因此每个词条都存 `in_dict` 标记，并额外用一份 `HashSet` 区分"来自用户词典"的词。
  - 注意：关闭 HMM 时，词典外词主要来自**用户自定义词典**；要让「元宇宙」这类词成为独立词条，必须靠 `--user-dict` 或二期的新词发现模块。
- **过滤规则**：token 至少含一个字母或数字（CJK 汉字属于 Alphabetic）才计入；丢掉纯标点/空白。可选 `--min-count`、`--no-latin`、`--no-digit`、`--min-len`。

### 关于 jieba 自带词频的重要说明

jieba 词典第三列是**分词用的概率权重**，不是词频统计，不能用来判断稀有度。实测证据：
- 349,046 条中 **159,318 条（45.6%）的权重恰好是保底值 `3`**，另有 40,502 条是 `2`，两者合计 57.2%。
- 排序不可信：「的」只排第 9（318,825），低于「了」（883,634）。

因此稀有度**必须**由本工具从语料库统计得出。

## 4. 统计粒度

**词 + 字**双表。
- 词表：jieba 切分后的 token（经过滤），带 `in_dict` / `from_user` 标记。
- 字表：单字（CJK）频次。用 Unicode 码点直接索引的 `Vec<u64>` 计数（`0x11000` 槽位 ≈ 557 KB/线程），零竞争。

**分域**：全库总表 + 每个一级子目录一张子表（`news` / `wiki` / `blog` / `book` / `forum` / `gov` / `subtitle` / `parallel`）。
实现方式为**按域顺序处理、域内并行**：每个域用一份全新的每线程计数表，产出后落盘并释放，最后把各域计数归并成全库总表。这样 33 GB 只读一遍，内存也受限。

## 5. 产物格式

输出目录结构：

```
<out>/
├─ meta.json              # 构建元信息、分域清单、分带阈值、统计口径
├─ full/
│  ├─ word.tsv            # 可读排行（rank, count, permille, word, in_dict）
│  ├─ word.vfr            # 词表二进制索引（供 mmap 查询）
│  ├─ char.tsv
│  └─ char.vfr
├─ domains/
│  ├─ news/word.tsv + word.vfr + char.tsv + char.vfr
│  └─ ...
└─ oov_candidates.tsv     # 词典外高频候选词（可喂给 --user-dict）
```

### 5.0 表的组织方式与排名语义

产物是 **2 种类型 × 8 个作用域 = 16 张表**：

```
full/word  full/char                                  全库
domains/{blog,book,forum,gov,news,parallel,wiki}/{word,char}
```

**两张表的排名完全独立**，在 `rank.rs` 里分两条路径：

```rust
// 词表：按频次降序，同频次按词的字节序（保证可复现）
pub fn rank_entries_owned(map, flag_fn) -> Vec<RankedEntry>     // rank = 1..n
// 字表：char_entries() 已按频次降序（同频次按码点），直接编号
pub fn rank_chars(chars: Vec<(char, u64)>) -> Vec<RankedEntry>  // rank = 1..n
```

四个维度都是独立的：

| | 词表 | 字表 |
|---|---|---|
| 名次序列 | 1 … 3,805,378 | 1 … 18,750 |
| 分母（算 `pct`） | 26.43 亿（词 token 数） | 43.12 亿（汉字出现次数） |
| 阈值 | 100 / 1k / 5k / 20k / 50k / 150k | 50 / 200 / 600 / 1500 / 3000 / 5000 |
| 分域表 | 各域独立编号 | 各域独立编号 |

**分域排名不能横向直接比大小**。例如 `人工智能` 是 `news#53204` 但 `wiki#2767`——这不代表它在 news 里罕见 19 倍，而是 news 域本身 token 少、名次被压缩了。跨域比较要看 `pct`，不能看名次。

**单字与词的计数口径不同**，这是刻意的：

| 字 | 字表（全文出现次数） | 词表（作为**独立词**出现次数） |
|---|---|---|
| 的 | 149,306,371（rank 1） | 146,761,038（rank 1） |
| 中 | 28,075,982（rank 9） | 11,607,613（rank 7） |
| 国 | 27,360,898（rank 10） | **688,367（rank 546）** |

`国` 出现 2,736 万次，但作为独立词只有 68.8 万次——它几乎总嵌在「中国/国家/全国」里。划句时单字查字表，所以颜色反映「这个字本身常不常见」，而不是「它作为独立词常不常见」。对「这个字罕不罕见」这个问题，字频才是对的答案，否则 `国` 会因为分词结果显得比实际罕见 40 倍。

### 5.0.1 表管理器与分组自定义

- **表开关**：`Settings.enabledTables` 决定哪些表参与**分域对比与排行榜**。全库表始终用于「单个词/字的总体频率」查询——若把它也关掉，划句分析会整片变成「未收录」，那没有意义。开关同时省掉大量无用查表（每个 token 本来要查 7 个分域）。
- **分组自定义**：`Settings.tierMethod` 支持三种口径。
  - `rank`（默认）：直接给七组定**排名绝对值**上界。直观、可跨语料库比较，但看不出每组实际盖住多少正文。
  - `coverage`：给定「每组累计覆盖正文的百分比」，由**覆盖率曲线反解**出排名上界。跨语料库最可比（不必关心词条总数），代价是组的大小差异很大。
  - `even`：按词条数七等分，最简单粗暴。
- **覆盖率曲线**：`tier_curve` 命令返回对数间隔采样的 `(rank, 累计覆盖率)`，约 500 个点。实测全库词表（380 万条）画一次曲线只需 **0.08 秒**，且结果按产物缓存。
  采样刻意在头部密、尾部疏：Zipf 分布的有用信息几乎全在头部，等距采样会把前 100 名挤成一个点。
- **覆盖率 → 排名**的反解在 `log10(rank)` 上线性插值（Rust 侧 `rank_for_coverage`，TS 侧有一份等价实现）。在 rank 上线性插值会在头部产生很大误差，因为 1→2 名与 100000→100001 名的覆盖率跨度完全不同。
  双向验证：拿默认排名阈值反推覆盖率目标（26.7/55.6/78.3/90.8/95.8/98.8%），再反解回来得到 `100/1003/4982/19871/50459/147635`，与默认阈值基本吻合。
- **自定义阈值必须在前端生效**，否则用户改完设置界面毫无变化。因此前端有唯一的 `effectiveBounds(kind, meta, settings, curves)` 负责算出实际生效的六个上界，所有着色与图例都走它；`token.tier` 只在未自定义时作为默认值参考。

### 5.1 `.vfr` 二进制格式（little-endian）

`VFR` = VocTier Frequency Ranking。设计目标：**mmap 打开后 O(log n) 查词、按排名 O(1) 取条目、常驻内存小**。

```
Header（96 字节）
  0   magic              [u8;8]  = b"VOCFREQ1"
  8   version            u32     = 1
  12  kind               u32     0=词表 1=字表
  16  entry_count        u64
  24  block_size         u32     每块记录条数（固定 64）
  28  block_count        u32
  32  total_tokens       u64     所有 count 之和（用于算占比）
  40  index_offset       u64     块索引区起始（固定 96）
  48  heads_offset       u64     块首词区起始
  56  rank_index_offset  u64     排名索引区起始（entry_count==0 时为 0，表示无此区）
  64  flags              u32     bit0=记录里带一个 flags 字节
  68  reserved           [u8;28]

块索引区（block_count × 16 字节）
  u64  record_offset   该块第一条记录的文件偏移
  u32  head_off        块首词在「块首词区」内的偏移
  u16  head_len        块首词字节长度
  u16  _pad

块首词区
  各块首词按块序拼接的 UTF-8 字节。
  存**完整**块首词（不是截断前缀），因此对块做二分是精确的，查词只需线性扫一个块。
  500 万词条时本区约 600 KB，打开时整块载入内存。

排名索引区（entry_count × 4 字节）
  按 rank 下标存放该条记录的文件偏移（u32）。记录区超过 4 GB 时会拒绝写出。
  排行榜翻页靠它做到 O(1)。

记录区（按 word 的 UTF-8 字节序升序排列）
  varint word_len
  bytes  word            UTF-8
  varint count
  varint rank            从 1 开始
  u8     flags           bit0=在 jieba 词典内  bit1=来自用户词典（仅当 header.flags bit0 置位）
```

**查词算法**：对块索引区的块首词做二分，找到第一个「块首词 > 目标」的块，目标只可能落在它前一块；然后线性扫那一块的记录，遇到大于目标的词即可停止（块内有序）。查不到即「语料库未收录」。

**前缀搜索**（排行榜搜索框）：记录区本来就按词的字节序排列，所以前缀匹配的词在文件里是连续的一段——先二分定位到首个块首词 ≥ 前缀的块，再顺序扫到不再匹配为止。

**按排名取条目**：直接查排名索引区，O(1)。

### 5.2 `meta.json`

```jsonc
{
  "schema_version": 1,
  "generated_at": "2026-...",
  "corpus_root": "E:\\...\\语料库",
  "tool_version": "0.1.0",
  "tokenizer": { "engine": "jieba-rs", "version": "0.11.0", "hmm": false,
                 "dict": "builtin", "user_dict": null, "filter": { "min_len": 1 } },
  "totals": { "files": 77, "bytes": 35740000000, "lines": 0, "paras": 0,
              "tokens": 0, "bad_lines": 0 },
  "domains": [ { "name": "wiki", "files": 23, "bytes": 0, "tokens": 0 } ],
  "tables": {
    "full/word": { "kind": "word", "entries": 0, "total_tokens": 0, "tiers": [...] },
    "full/char": { "kind": "char", "entries": 0, "total_tokens": 0, "tiers": [...] }
  }
}
```

### 5.3 七组分带（词表默认）

按**排名绝对值**分带（用户选定的口径）。因为字表只有约 1 万个不重复字，词表有数百万，所以**两表阈值不同**。

词表（按实测累计覆盖率校准，全库 26.43 亿 token）：
| 组 | 排名区间 | 本组覆盖 | 累计覆盖 |
|---|---|---|---|
| 极多 | rank ≤ 100 | 26.7% | 26.7% |
| 很多 | ≤ 1,000 | 28.9% | 55.6% |
| 较多 | ≤ 5,000 | 22.8% | 78.3% |
| 中等 | ≤ 20,000 | 12.5% | 90.8% |
| 较少 | ≤ 50,000 | 4.9% | 95.8% |
| 很少 | ≤ 150,000 | 约 3% | 约 99% |
| 极少 | > 150,000 | 约 1% | 100% |

字表：`≤50 / ≤200 / ≤600 / ≤1500 / ≤3000 / ≤5000 / >5000`

第 8 种状态：**语料库未收录**（灰色 + 虚线下划线），与"极少"区分开。

> **「极多」为什么定在 100**：早期版本用 500，但实测那 500 个词就盖住了 45% 的正文，
> 导致「极多」与「很多」在观感上几乎没有差别。收到 100 之后各段覆盖率衰减得比较匀
> （26.7% → +28.9pp → +22.8pp → +12.5pp → +4.9pp），色阶才有区分度。

> 阈值可在 `meta.json` / 设置页调整，但前端必须**读取 meta.json 里的阈值**，不能硬编码，否则换语料库后色阶会错。

调阈值后必须**重跑一次统计**，因为覆盖率与阈值都写在 `meta.json` 里、界面从那里读。
想先看效果不必重新分词：拿 `data/full/word.tsv` 前若干行累加 `count` 列，
除以 `meta.json` 的 `total_tokens` 就是累计覆盖率。

### 5.4 配色

每组一个 `fg` + `bg`，浅色/深色各一套。

| 组 | 浅色前景 / 背景 | 深色前景 / 背景 |
|---|---|---|
| 极多 | `#B42318` / `#FEE4E2` | `#FDA29B` / `#4E1D18` |
| 很多 | `#B54708` / `#FEF0C7` | `#FEC84B` / `#4E3200` |
| 较多 | `#8A6100` / `#FEF7C3` | `#FDE272` / `#463A00` |
| 中等 | `#3B6E1E` / `#E7F5DC` | `#A6E36E` / `#1F3B0E` |
| 较少 | `#175CD3` / `#E0EAFF` | `#84ADFF` / `#132A5C` |
| 很少 | `#6941C6` / `#F4EBFF` | `#C3B5FD` / `#2D1B69` |
| 极少 | `#475467` / `#F2F4F7` | `#98A2B3` / `#33383F` |
| 未收录 | `#98A2B3` / 透明 + 虚线 | `#667085` / 透明 + 虚线 |

颜色语义为"暖=常见、冷/淡=罕见"，整体走单色相强度渐变以减少视觉噪声。

## 6. CLI 接口

```
vocfreq detect   --corpus <DIR>                        # 只探测 schema，不扫描
vocfreq scan     --corpus <DIR> --out <DIR> [选项]      # 全量统计，产出全部产物
vocfreq info     --table <FILE.vfr>                    # 打印头信息与若干条目
vocfreq lookup   --table <FILE.vfr> <WORD>...          # 验证索引查询
vocfreq segment  --data <OUTDIR> "<文本>"               # 分词并附频率，用于验证前端逻辑
```

`scan` 选项：`--threads N`（默认物理核数）、`--hmm`、`--user-dict FILE`、`--dict FILE`、
`--rules FILE`、`--min-count N`、`--domains a,b,c`、`--no-domains`、`--formats tsv,jsonl`、
`--progress json|bar|none`、`--oov-threshold N`。

**进度上报**：`--progress json` 时向 **stderr** 逐行输出 JSON 事件，供 Tauri 子进程解析并驱动进度条：

```json
{"event":"plan","files":77,"bytes":35740000000}
{"event":"phase","phase":"scan","domain":"wiki","files_done":3,"files_total":23,"bytes_done":1500000000,"bytes_total":8020000000}
{"event":"table","table":"full/word","entries":3123456}
{"event":"done","elapsed_ms":84000}
```

## 7. 配置文件

`--rules rules.json` 接受规则数组（覆盖自动探测）：

```json
[
  { "name": "mnbvc_paragraph", "array_key": "段落", "text_key": "内容", "strip_html": false },
  { "name": "mnbvc_forum", "array_key": "回复", "text_key": "回复", "strip_html": true },
  { "name": "plain_text", "plain_key": "text", "alt_text_keys": [] },
  { "name": "parallel_subtitle", "plain_key": "zh_text", "alt_text_keys": ["cht_text"] }
]
```

匹配方式：某文件的 top-level key 集合若含 `array_key`（或 `plain_key`）即视为命中该规则；也支持 `--rules` 里用 `glob` 字段按路径强制指定。

## 8. 前端（apps/desktop）

四个页面：

1. **生成词频表**：选语料库目录 → 显示探测到的 schema 与文件清单/体积 → 配置分词与输出 → 启动（后台线程调 `scan::scan`，进度经 `scan:progress` 事件推给前端）→ 展示各分组覆盖率。
2. **划句分析**（核心）：输入框 + 划选文字，按词着色展示，悬停显示频次/排名/占比/分组/是否词典外词；支持切换查看不同分域的排名；支持把全文发到悬浮小窗。
3. **排行榜**：分页浏览、按词首前缀搜索、按七组筛选、查看分域对比。
4. **设置**：全局热键、小窗置顶/透明度/尺寸/自动关闭、主题、默认分词选项、默认语料库与产物目录。

### 8.1 Tauri 命令接口（已冻结）

所有命令返回 `Result<T, String>`，错误信息为中文。字段名一律 **snake_case**（与 `vocfreq-core` 的 serde 输出一致）；**入参结构体**用 `#[serde(rename_all = "camelCase")]`，因此前端传 camelCase。

| 命令 | 入参 | 返回 |
|---|---|---|
| `app_info` | — | `AppInfo { name, version, tauri_version, core_version }` |
| `plan_corpus` | `corpus` | `CorpusPlan { corpus, files, bytes, domains[{name,files,bytes,rules}], warnings }` |
| `dataset_status` | `dir` | `DatasetStatus { dir, exists, meta }`（不抛错，`exists=false` 时 `meta=null`） |
| `open_dataset` | `dir` | `Meta`（把已有产物目录装入内存并缓存分词器） |
| `analyze_text` | `text`, `domains`, `dir?` | `Vec<TokenInfo>`（`domains` 与 `settings.enabledTables` 取交集后过滤 `domain_ranks`） |
| `tier_curve` | `path`, `dir?`, `maxPoints?` | `TierCurve { path, kind, entries, total_tokens, points: [[rank, 累计覆盖率]] }` |
| `lookup_word` | `word`, `kind?`, `dir?` | `Option<WordHit>` |
| `list_rank` | `domain?`, `kind?`, `from?`, `limit?`, `dir?` | `Vec<RankRow>` |
| `search_words` | `query`, `kind?`, `domain?`, `limit?`, `dir?` | `Vec<RankRow>` |
| `start_scan` | `params: ScanParams` | 立即返回；结果走事件 |
| `cancel_scan` | — | 置取消标志；产物目录会被删掉 `meta.json` 以标记不完整 |
| `get_settings` / `set_settings` | `settings` | `Settings`（持久化到应用配置目录） |
| `capture_selection` | — | `String`（空串 = 当前无选区） |
| `take_pending_selection` | — | `String`（小窗启动时取走待分析文本） |
| `open_popup` / `hide_popup` | `text?` | 显示/隐藏悬浮小窗 |

事件：`scan:progress`（判别联合，带 `event` 标签：`log` / `plan` / `phase` / `table`）、`scan:done`（`Meta`）、`scan:error`（`String`）。

`Meta.tables` 里带**每张表的阈值与各带实际覆盖率**，界面必须从那里读，不要硬编码分组边界。

**分词器重建规则**：打开产物目录时，分词器必须按 `meta.tokenizer` 里记录的口径重建（hmm、过滤规则、用户词典路径），否则分词结果对不上已落盘的词表，查出来的频次会系统性偏错。

### 8.2 全局取词与悬浮小窗

- 触发：全局热键（默认 `Alt+Q`，可改）。
- 取词：**剪贴板模拟法**——暂存剪贴板 → 模拟 `Ctrl+C` → 等 `GetClipboardSequenceNumber` 变化（不能靠固定 sleep，不同程序差异很大）→ 读走选区 → 立刻还原。不需要鼠标钩子。
  - 若原剪贴板内容不是文本（图片/文件），`arboard` 无法完整还原；此时**保留抓到的文本而不是清空剪贴板**。
  - 取词会阻塞数百毫秒，因此**必须在独立线程里做**，不能占着热键回调线程。
- 小窗：无边框、置顶、不占任务栏。窗口 label 固定为 `popup`，前端按 `getCurrentWindow().label` 决定挂载主界面还是小窗界面——这样**不需要给 Vite 配多入口**。
- 小窗内容：**输入框 + 着色卡片**，带 `data-tauri-drag-region` 的拖拽区与关闭按钮；关闭按钮走 `CloseRequested` 拦截改成隐藏，避免反复重建窗口。
- 位置：光标右下方，越界则翻到光标上方或贴边——宁可位置不完美也不能跑到屏幕外。
- 已知限制：目标程序若以管理员身份运行而 VocTier 不是，Windows UIPI 会阻止 `Ctrl+C` 送达；此时需以管理员启动 VocTier（设置页给出提示）。

## 9. 二期（已确认留到后面）

**新词发现**：用互信息 PMI + 左右邻字熵扫全语料，挖出词典外候选新词（如「元宇宙」「直播带货」「情绪价值」），导出 `oov_candidates.txt`，再由 `--user-dict` 回填分词。
不是简单数 2~4 元组合——那会产生 99% 的语法垃圾（「的+续航」「车+的」）。
