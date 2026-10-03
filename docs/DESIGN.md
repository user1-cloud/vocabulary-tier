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
   每个元素 5 个键。用 `Cow` 读键等于上千万次堆分配——实测这让 gov 表组慢了 5.7 倍。
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

`crates/vocfreq-cli` 可单独分发，是"纯 Rust 工具"，不依赖 Tauri。桌面端**并不通过子进程调用 CLI**：
`apps/desktop/src-tauri` 以 crate 路径依赖（`vocfreq-core = { path = "../../../crates/vocfreq-core" }`）**直接复用同一个统计核心**，
因此命令行工具与桌面端产出的结果与分词粒度必然一致（cli 侧负责交互，统计一律落在 core）。

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
- **词典完全外置，且是一条「词典链」**（`jieba-rs` 的 `default-dict` feature 已关闭，所以 `Jieba::new()` 根本不存在了）：
  - `--dict FILE`（可重复）：按给出顺序装载，**第一份是主词典**，其余依次叠加。
  - `--dict-dir DIR`：目录里所有 `.dict`，按文件名排序，接在 `--dict` 之后。
  - `--user-dict FILE`（可重复）：同上，接在最后；其中的词会被标记成 `from_user`。
  - 同名条目**后者覆盖前者的词频**（jieba `load_dict` 的既有语义：已存在的词只改词频，不新增记录），所以顺序有意义。
  - 词典链不能为空：没有词典时 jieba 只能吐单字，产物会静默退化成一张废表，所以直接报错。
- **词典读取一律走 `vocfreq_core::dict`，不用 jieba 的 `load_dict` 直接读文件**，三个理由：
  1. **注释**。jieba 的 `load_dict` 会把 `#` 开头的行也当词条：第二列不是整数就 `InvalidDictEntry` 失败，恰是数字则把 `#` 当成一个词静默插进词典。而 `artifact::write_oov` 产出的候选词文件开头就有 10 行 `#` 说明，文档还让用户拿它去喂词典 —— 这条闭环曾经是**必然失败**的。规则定为：**`#` 开头的整行是注释**（代价是以 `#` 开头的词无法在词典里表达，属于刻意取舍，有测试钉住）。
  2. **不留半装载状态**。jieba 的 `load_dict` 一进门就把 `total` 归零，中途出错又不走 `finish_load()`，分词器会停在 `total` 与 `log_total` 打架、部分词已进 trie 的中间态。改为先把文件整份读进内存、逐行校验，确认无误再喂进去。
  3. **内容指纹**。词典现在可被用户随手替换，`meta.json` 里就必须记住"这张表基于哪份词典"，见 §5.2。
- **省略词频 ≠ 词频 0**。jieba 的 `load_dict` 把省略词频按 `0` 处理，而 `freq = 0` 的路径概率是 `ln(0) = -inf`：该词会被登记进词典（`has_word` 为真、`in_dict` 标记为真）却**永远不可能被切分出来**。文档说词频可省略，用户省略了却是这个结果，属于隐蔽的坑。所以省略词频的条目改走 `Jieba::add_word(None)`（由 `suggest_freq` 折算，与 Python 版 jieba 一致），并且 `DictLoadReport` 会把条数报出来让界面提示。**显式写成 `0`** 的条目则单独统计、单独警告 —— 那几乎总是笔误。
- **词典里的顺序必须保真**：只要这份文件里有省略词频的条目，就整份逐条 `add_word`，不重排。`suggest_freq` 的结果取决于"此刻词典里已经有什么"，重排会让同一份文件得出不同结果，破坏可复现性。
- **词典外词标记**：`jieba.has_word(w)` 是免费查询，因此每个词条都存 `in_dict` 标记，并额外用一份 `HashSet` 记录**叠加词典**（词典链里第一份之外）贡献的词，即 `from_user`。
  - 这个位的含义在词典外置后收紧过一次：从前是"来自用户词典（不在 jieba 内置词典里）"，现在是"用户额外叠加上去的"。产物格式没变，只是语义更准。
  - 注意：关闭 HMM 时，词典外词主要来自**叠加词典**；要让「元宇宙」这类词成为独立词条，必须靠 `--user-dict` 或二期的新词发现模块。
- **过滤规则**：token 至少含一个字母或数字（CJK 汉字属于 Alphabetic）才计入；丢掉纯标点/空白。可选 `--min-count`、`--no-latin`、`--no-digit`、`--min-len`。

### 关于 jieba 自带词频的重要说明

jieba 词典第三列是**分词用的概率权重**，不是词频统计，不能用来判断稀有度。实测证据：
- 349,045 条中 **159,318 条（45.6%）的权重恰好是保底值 `3`**，另有 40,502 条是 `2`，两者合计 57.2%。
  > 注：长期写作"349,046 条"是个**多算一条**的数字，来自 jieba 内部 `count('\n') + 1` 的预留估算；逐行解析出来的真实条目数是 349,045。`meta.json` 里现在记的是真实值。
- 排序不可信：「的」只排第 9（318,825），低于「了」（883,634）。

因此稀有度**必须**由本工具从语料库统计得出。

## 4. 统计粒度

**词 + 字**双表。
- 词频表：jieba 切分后的 token（经过滤），带 `in_dict` / `from_user` 标记。
- 字表：单字（CJK）频次。用 Unicode 码点直接索引的 `Vec<u64>` 计数（`0x11000` 槽位 ≈ 557 KB/线程），零竞争。

**表组（scope）**：语料库的每个一级子目录算一个表组（`news` / `wiki` / `blog` /
`book` / `forum` / `gov` / `subtitle` / `parallel`），`full` = 全部加在一起，也是表组
之一。实现方式为**按表组顺序处理、表组内并行**：每个表组用一份全新的每线程计数表，产出后
落盘并释放。这样 33 GB 只读一遍，内存也受限。

> **`full` 不是特权表组。** 它只是"所有表组相加"的那一个，与 `news` 完全平级
> （见 §5.0）。谁负责回答"这个词有多常见"由用户指定的**主表组**决定。
>
> **`scan` 默认不产出 `full`**（`ScanConfig::write_full` 默认 false）：它只写各表组
> 自己的表，`full` 由「`vocfreq merge` 合流多份产物 + `vocfreq compose` 把表组相加」
> 得到（见 `DATA_LAYOUT.md` §七）。原因很实际：全量语料常常大到一块盘放不下，只能一个表组
> 一个表组地扫成**多份**产物，而每次 `scan` 都整体覆盖 `meta.json` 与 `full/`，分次扫时
> 前几次写的全被冲掉。`--full` 保留下来只为一件事 —— 一次性全量扫一遍留一份**对照基准**。

**相加可以复现 `full`**：`overall.merge(&counts)` 这一步就是加法，而每个 token 只属于
一个表组，所以 `full.count(词) == Σ 各表组.count(词)` **逐条相等**。
`vocfreq compose` 正是利用这一点（测试里既拿"扫描产出的表组相加"与"扫描产出的 full"
逐条比对，也拿"表组扫成几份产物 → merge → compose"与"一次全量扫描"逐条比对）。
唯一的坑是**低频过滤阈值要跟着走**：源表各自按 `min_count` 丢过一批词，相加方必须丢掉
同一批（取各源的最大值），否则排名会整体前移。

**跨产物合计的前提**：`merge` 与多源 `compose` 都要求各源产物的 `tokenizer`（词典链
`sha256` + 分词口径）**完全一致**，不一致就报错拒绝（`compose::check_mergeable`）。
频次是同一套切分规则下的计数，凑合加起来会得到一份自相矛盾、且事后无从察觉的表。

## 5. 产物格式

输出目录结构（**schema v3：每个表组一个平级目录**；`full/` 只在显式 `--full`
或相加之后才存在）：

```
<out>/
├─ meta.json              # 构建元信息、语料切片清单、每张表的阈值与分带统计、来源
├─ news/                  # 一个表组 = 一个目录
│  ├─ word.tsv  word.vfr
│  └─ char.tsv  char.vfr
├─ wiki/ …
├─ full/                  # "全部表组相加"（默认不在这里，由相加产出），与别的目录完全平级
│  ├─ word.tsv  word.vfr
│  └─ char.tsv  char.vfr
├─ 相加：财经/             # 相加出来的新表也是同级的一个表组
│  └─ word.vfr           # 来源记在 meta.json 的 source_tables 里
└─ oov_candidates.tsv     # 词典外高频候选词（可喂给 --user-dict）
```

> **v3 之前不是这样。** 老布局是 `full/word.vfr` + `domains/<表组>/word.vfr` 两棵树，
> 把"表组"与"类型"糊在一个路径字符串里，`full` 还享有"总体频率只查它"的特权。
> 读取端现在**明确拒绝** schema < 3 的产物并告诉用户重扫（错误信息里会写清是 v 几、
> 以及"无法就地迁移"），不试图兼容 —— 硬读只会得到一句"文件不存在"。

### 5.0 表的组织方式与排名语义

产物是 **2 种类型 × N 个表组** 张表（N 取决于磁盘上扫过哪些一级子目录，
以及之后相加出来多少张）：

```
news/word  news/char  wiki/word  wiki/char  …   每个一级子目录各一套
full/word  full/char                            全部表组相加（由相加产出）
相加：财经/word                                  相加出来的新表（与上面完全平级）
```

**两张表的排名完全独立**，在 `rank.rs` 里分两条路径：

```rust
// 词频表：按频次降序，同频次按词的字节序（保证可复现）
pub fn rank_entries_owned(map, flag_fn) -> Vec<RankedEntry>     // rank = 1..n
// 字表：char_entries() 已按频次降序（同频次按码点），直接编号
pub fn rank_chars(chars: Vec<(char, u64)>) -> Vec<RankedEntry>  // rank = 1..n
```

四个维度都是独立的：

| | 词频表 | 字表 |
|---|---|---|
| 名次序列 | 1 … 3,805,378 | 1 … 18,750 |
| 分母（算 `pct`） | 26.43 亿（词 token 数） | 43.12 亿（汉字出现次数） |
| 阈值（**前%**，默认口径） | 0.0026 / 0.026 / 0.132 / 0.526 / 1.32 / 3.95 % | 0.26 / 1.05 / 3.16 / 7.89 / 15.8 / 26.3 % |
| 表组 | 各表组独立编号 | 各表组独立编号 |

**排名不能跨表直接比大小**。例如 `人工智能` 是 `news#53204` 但 `wiki#2767`——这不代表
它在 news 里罕见 19 倍，而是 news 表组本身 token 少、名次被压缩了。跨表比较要看
**前%**（`排名 ÷ 该表条目数 × 100`）或 `pct`（占全部 token 的百分比），不能看名次。

**单字与词的计数口径不同**，这是刻意的：

| 字 | 字表（全文出现次数） | 词频表（作为**独立词**出现次数） |
|---|---|---|
| 的 | 149,306,371（rank 1） | 146,761,038（rank 1） |
| 中 | 28,075,982（rank 9） | 11,607,613（rank 7） |
| 国 | 27,360,898（rank 10） | **688,367（rank 546）** |

`国` 出现 2,736 万次，但作为独立词只有 68.8 万次——它几乎总嵌在「中国/国家/全国」里。划句时单字查字表，所以颜色反映「这个字本身常不常见」，而不是「它作为独立词常不常见」。对「这个字罕不罕见」这个问题，字频才是对的答案，否则 `国` 会因为分词结果显得比实际罕见 40 倍。

### 5.0.1 主表组、表开关与分组自定义

- **主表组（主词频表）**：`Settings.primaryScope` 存一个**纯表组名**，默认 `full`；
  产物里没有它时回落到排序后的第一个表组。它决定**"这个词有多常见"的唯一口径** ——
  划句分析的着色与分组、排行榜、分组阈值预览、覆盖率曲线全部以它为准，其余表组只做
  **对比**（`TokenInfo.table_ranks`，每项带 `scope / kind / rank / top_pct / entries / count`）。
  粒度必须是全局的：不然同一句话在两个页面会是两种颜色。主表组缺某一类表时
  （相加只加了词频表就会出现）按**类**回落到 `full`、再回落到第一张，绝不让整句变「未收录」。
- **`Settings.enabledTables` 已废弃**。从前它是"参与表组对比与排行榜"的表开关；铺平之后
  不需要了（任何表组都能当主表、都能相加，对比查询本来就是 O(1) 的二分），代码里只
  留着读得进老设置文件，**不再有任何行为**。
- **相加（`vocfreq compose` / `compose_tables`）**：把若干张表加起来成同一份产物里的
  另一个平级表组。落地方式是**物化成真实 `.vfr`**（不是"只记一条 A+B+C 的标记"），
  理由是排行榜要按排名翻页、覆盖率曲线要顺序读遍全表，虚拟合成会让这两件事没法靠排名
  索引 O(1) 完成。磁盘代价是一份表大小（源表不需要了可以删掉回收）。
  `TableMeta::source_tables` 仍把来源记下来 —— 将来真要改成零拷贝虚拟合成，
  那个字段与 `query::TableReader` 就是现成的接口。源表**可以跨产物**（全量语料放不下时
  只能分几次扫），但**前提是各产物的词典链与分词口径完全一致**：不一致时
  `compose::check_mergeable` 直接报错拒绝，绝不"挑一份当基准"。理由与实现见
  `DATA_LAYOUT.md` §七。
- **合流（`vocfreq merge`）**：把 N 份独立产物并成一份标准产物，**不改变任何频次**，
  只是把各源的表组目录摆在一起、把 `tables[]` / `domains[]` / `totals` 并起来。
  过的是与多源相加同一道门槛。表组名跨产物撞车也拒绝（表组就是目录名，同名就是
  谁盖谁 = 静默丢数据）。命令行专属，桌面端这一版不暴露它（UI 零改动）。
- **分组自定义**：`Settings.tierMethod` 支持四种口径。
  - `top_pct`（**默认**）：给定七组的**前%上界**，由该表的条目数换算成排名上界
    （`rank::rank_for_pct` / 前端 `format.ts::rankForPct`）。只跟"位次"有关、与表规模无关，
    所以换表、换主表都不会失真。默认六档 `0.0026 / 0.026 / 0.132 / 0.526 / 1.32 / 3.95 %`
    不是新拍的：它们是把旧的绝对排名默认值（100/1k/5k/20k/50k/150k）放在实测的
    380 万条全库词频表上换算出来的，**色阶观感与从前完全一致**。
    **取整方向必须是 `ceil`**：判前%要拿边界条目自己的前%去比，`ceil` 才不会把边界那一条
    推到下一档；`floor` 还会在条目少时把六档压成同一个排名，七组出现空档、组号到配色的
    映射整体错位。空表返回 0（不是 1），让调用方一眼看出"这张表没有条目，别拿百分比切它"。
  - `rank`：直接给七组定**排名绝对值**上界。直观，但阈值是为某一张表的规模校准的。
  - `coverage`：给定「每组累计覆盖正文的百分比」，由**覆盖率曲线反解**出排名上界。
  - `even`：按词条数七等分，最简单粗暴。
- **覆盖率曲线**：`tier_curve` 命令返回对数间隔采样的 `(rank, 累计覆盖率)`，约 500 个点。实测全库词频表（380 万条）画一次曲线只需 **0.08 秒**，且结果按产物缓存。
  采样刻意在头部密、尾部疏：Zipf 分布的有用信息几乎全在头部，等距采样会把前 100 名挤成一个点。
  覆盖率与**前%**是两回事：覆盖率问"前 N 个词盖住了正文的百分之多少"（随分布走，后头部极陡），
  前%问"这个词排在前百分之几"（纯位次，跨表可比）。
- **覆盖率 → 排名**的反解在 `log10(rank)` 上线性插值（Rust 侧 `rank_for_coverage`，TS 侧有一份等价实现）。在 rank 上线性插值会在头部产生很大误差，因为 1→2 名与 100000→100001 名的覆盖率跨度完全不同。
  双向验证：拿旧的默认排名阈值反推覆盖率目标（26.7/55.6/78.3/90.8/95.8/98.8%），再反解回来得到 `100/1003/4982/19871/50459/147635`，与旧阈值基本吻合 —— 这就是前%六档的来历。
- **自定义阈值必须在前端生效**，否则用户改完设置界面毫无变化。因此前端有唯一的 `effectiveBounds(kind, meta, settings, curves, tableKey)` 负责算出实际生效的六个上界，所有着色与图例都走它；`token.tier` 只在未自定义时作为默认值参考。

### 5.1 `.vfr` 二进制格式（little-endian）

`VFR` = VocTier Frequency Ranking。设计目标：**mmap 打开后 O(log n) 查词、按排名 O(1) 取条目、常驻内存小**。

```
Header（96 字节）
  0   magic              [u8;8]  = b"VOCFREQ1"
  8   version            u32     = 1
  12  kind               u32     0=词频表 1=字表
  16  entry_count        u64
  24  block_size         u32     每块记录条数（固定 64）
  28  block_count        u32
  32  total_tokens       u64     所有 count 之和（用于算占比）
  40  index_offset       u64     块索引区起始（固定 96）
  48  heads_offset       u64     块首词区起始
  56  rank_index_offset  u64     排名索引区起始（entry_count==0 时为 0，表示无此区）
  64  flags              u32     bit0=记录里带一个 flags 字节
  68  records_offset     u64     记录区起始偏移（0 = 老写入端没记）
  76  records_bytes      u64     记录区字节数（0 = 老写入端没记）
  84  reserved           [u8;12]

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

> **68/76 这两个字段不是可有可无的。** 它们覆盖了原来 `reserved` 的前 16 字节。
> **绝不能靠文件长度推断记录区末尾**：最后一块的结束位置原本取的是 mmap 长度，而
> `BufWriter` 会不会在记录区之后再落几个字节是实现细节 —— 实测那条路会让整表遍历
> 把尾巴上的字节当记录解析，516 条的样本只遍历出 513 条（`VfrTable::positions` 就是
> 踩了这个坑才加的这两个字段）。写入端现在还会把文件截到记录区末尾，所以老读取端
> （只认 `mmap.len()`）在正常情况下也不会读到多余的字节。

**查词算法**：对块索引区的块首词做二分，找到第一个「块首词 > 目标」的块，目标只可能落在它前一块；然后线性扫那一块的记录，遇到大于目标的词即可停止（块内有序）。查不到即「语料库未收录」。

**前缀搜索**（排行榜搜索框）：记录区本来就按词的字节序排列，所以前缀匹配的词在文件里是连续的一段——先二分定位到首个块首词 ≥ 前缀的块，再顺序扫到不再匹配为止。

**按排名取条目**：直接查排名索引区，O(1)。

**整表顺序遍历**（`positions()`，相加与全表比对用）：块本来就是按词序落的，顺着块一路读下去就是全表，纯流式、不缓存任何条目。

### 5.2 `meta.json`

```jsonc
{
  // 1 = 词典当年编在 exe 里，dict 只是一句自由文本、没有指纹；
  // 2 = 词典外置 + 老布局（full/ 与 domains/ 两棵树，path 是 "full/word" 这种路径）；
  // 3 = **表组铺平**：每个表组一个平级目录，path 变成纯表组名。
  // 读取方遇到 1/2 必须**明确报错并让用户重扫**（老布局无法就地迁移），
  // 遇到 1 的 tokenizer 还要额外降级成"词典不可校验"而**不是**报错。
  "schema_version": 3,
  "generated_at": "2026-...",
  "corpus_root": "E:\\...\\语料库",
  "tool_version": "0.1.0",
  "tokenizer": {
    "engine": "jieba-rs", "version": "0.11", "hmm": false,
    // 按装载顺序：dicts[0] 是主词典，其后都是叠加词典。
    // 用户换了词典之后，界面靠这里的 sha256 立刻看出"这张表的口径可能已经不对了"。
    "dicts": [ { "id": "预制词典", "name": "预制词典",
                 "path": "C:\\Users\\...\\data\\dicts\\预制词典.dict",
                 "entries": 349045, "sha256": "13951982…" } ],
    // v1 兼容：只在读老产物时出现，新产物永不写出
    // "dict": "builtin(jieba dict.txt, 349046 entries)", "user_dict": null,
    "min_len": 1, "max_len": 64,
    "keep_latin": true, "keep_digit": false, "skip_single_char": false
  },
  "totals": { "files": 77, "bytes": 35740000000, "lines": 0, "paras": 0,
              "tokens": 0, "bad_lines": 0 },
  // **语料切片**清单（扫到了哪些一级子目录），不是表清单 —— 表看 tables。
  // 相加产生的表里它保持源产物的值（它描述语料，不描述表）。
  "domains": [ { "name": "wiki", "files": 23, "bytes": 0 } ],
  "tables": [
    { "path": "full", "kind": "word", "entries": 0, "total_tokens": 0, "vfr_bytes": 0,
      "tiers": [...], "tier_stats": [...],
      // 低频过滤阈值（相加时要沿用，否则与全量重扫的结果对不上）
      "min_count": 1,
      // **默认分组口径**：七组的前%上界（6 个，0..100）
      "tier_pct": [0.0026, 0.026, 0.132, 0.526, 1.32, 3.95],
      // 空 = 扫描出来的表；非空 = 把这几张表相加出来的
      "source_tables": [] },
    { "path": "news", "kind": "char", ... }
  ]
}
```

> **`path` 的语义在 v3 变过一次。** 从前它是 `full/word`、`domains/news/char` 这种
> 「路径」，把表组与类型糊在一个字符串里。现在它只是**表组名**（同时是子目录名），
> 类型在 `kind` 里；两者合起来才是表身份 —— 拼字符串一律走
> `artifact::table_key(scope, kind)`（TS 侧 `types.ts::tableKey`，前端还有
> `format.ts::tableKeyOf(table)`）。**全仓库只有这几处拼它**，别无副本。
> 设置里的 `primaryScope` 存的是**纯表组名**（与 path 同构）；老设置里的
> `enabledTables` 存的是"表组/类型"形式，两者都能对上。

> **字段命名**：入参 camelCase、出参 snake_case。`DictRef` 的**序列化永远是对象**，
> 但反序列化额外接受一个纯字符串 —— 那是 v1 的 `"dict": "builtin(…)"` 写法，
> 会被降级成"有名字但无指纹"。所以判断"这份产物有没有可校验的词典记录"要看
> **有没有指纹**，不能看记录列表空不空：v1 的 `dict` 反序列化出来正好是一条
> 没指纹的记录，列表非空却什么也说明不了。这个坑有测试钉住
> （`binding_legacy_for_v1_products_without_fingerprints`）—— 判错的话每一份老产物
> 升级后都会顶着一条红色"词典缺失"告警，直接训练用户忽略告警。

> **分组的「身份」是 `tier_keys`，不是 `tier_names`。**
>
> `tier_names` 是**展示文案**（中文，接入界面多语言后会随语言变化）；`tier_keys` 是与语言无关的
> **稳定标识**，与 `tier_names` / `tiers` / `tier_stats` **同序**，取值见
> `crates/vocfreq-core/src/rank.rs::TIER_KEYS`：
> `very_common / common / fairly_common / medium / fairly_rare / rare / very_rare`。
>
> 界面把「组号」（`token.tier`、`tiers[i]`、`tier_stats[i]` 的下标）经 `tier_keys` 映射到配色调色板
> 与本地化文案（见 `apps/desktop/src/lib/tier-colors.ts`），于是：
>
> - 产物调整了分组**顺序**也不会串色（映射走 key，不走位置）；
> - 产物出现前端**不认识**的 key（新增档位）会退化成中性灰，而不是静默套错颜色；
> - **已发布的 key 不能改名** —— 改名等于换身份，老产物会对不上色。
>
> 本字段是 `#[serde(default)]` 的：引入它**之前**产出的 `meta.json` 没有它，反序列化得到空数组，
> 界面回落到前端常量（顺序一致）。但 v3 布局变化之后老产物本来就读不了（见上文），
> 所以这条兜底只剩"手改过 meta 的产物"这一种场景。

### 5.3 七组分带（默认口径：前%）

按**前%**分带：`前% = 排名 ÷ 该表总条目数 × 100`，六档上界见 `rank::DEFAULT_TIER_PCT`。
**词频表与字表的前%不同**：字表只有约 1.9 万个不重复字，套用词频表那套会让头几档只剩个位数的字、
九成以上的字全部挤进「极少」。

词频表默认前%（括号内是它在实测的 380 万条全库词频表上换算出来的排名，也就是**旧的绝对阈值**）：

| 组 | 前%上界 | ≈ 排名（全库 380 万条） | 该档累计覆盖 |
|---|---|---|---|
| 极多 | 前 0.0026% | ≤ 99 | 26.7% |
| 很多 | 前 0.026% | ≤ 990 | 55.6% |
| 较多 | 前 0.132% | ≤ 5,024 | 78.3% |
| 中等 | 前 0.526% | ≤ 20,017 | 90.8% |
| 较少 | 前 1.32% | ≤ 50,231 | 95.8% |
| 很少 | 前 3.95% | ≤ 150,313 | 约 99% |
| 极少 | 前 100% | 其余全部 | 100% |

字表默认前%：`0.26 / 1.05 / 3.16 / 7.89 / 15.8 / 26.3 %`
（由旧的 `≤50 / ≤200 / ≤600 / ≤1500 / ≤3000 / ≤5000` 在约 1.9 万字的表上换算而来）。

第 8 种状态：**语料库未收录**（灰色 + 虚线下划线），与"极少"区分开。

> **「极多」为什么这么紧**：早期版本用绝对排名 500，但实测那 500 个词就盖住了 45% 的
> 正文，导致「极多」与「很多」在观感上几乎没有差别。收到前 0.0026% 之后各段覆盖率衰减
> 得比较匀（26.7% → +28.9pp → +22.8pp → +12.5pp → +4.9pp），色阶才有区分度。

> **前%只跟位次有关，与表的规模无关。** 这正是它取代绝对排名当默认口径的原因：布局铺平
> 之后任意表组都能当主表，而表组只有几万条、全量表有几百万条 —— 同一个绝对阈值套上去，
> 表组会整片挤进「极多」。

> 阈值可在 `meta.json` / 设置页调整。绝对排名与覆盖率两套口径都还在（`tierMethod`），
> 但界面**默认读前%**，且这一切都来自 `meta.tables[].tier_pct`，不要硬编码。

调阈值后**重跑一次统计**最省事（覆盖率与阈值都写在 `meta.json` 里、界面从那里读）。
想先看效果不必重新分词：`vocfreq pct --data <DIR> --table full/word` 直接把前%换算成排名阈值，
`vocfreq curve` 则给出覆盖率曲线。

### 5.4 配色

每组一个 `fg` + `bg`，浅色/深色各一套。

调色板按**稳定标识 `tier_keys`** 索引（`apps/desktop/src/lib/tier-colors.ts` 里的
`TIER_PALETTES`），**不按中文组名索引** —— 组名会随界面语言变化，按它取色会在接入多语言时
整片串色。`Record<TierKey, …>` 的写法还让「漏配某个 key」变成编译错误。

颜色语义 = **游戏稀有度色阶**（白 → 绿 → 蓝 → 紫 → 橙 → 红 → 金）：越罕见的词，
颜色越像游戏里的高稀有度物品；最常见的词给最素的灰白。单调递增，不玩色温渐变。

| 组 | 稀有度 | 浅色前景 / 背景 | 深色前景 / 背景 |
|---|---|---|---|
| 极多 | 普通 · 灰白 | `#475467` / `#F2F4F7` | `#CBD5E1` / `#2C323B` |
| 很多 | 优秀 · 绿 | `#137A3A` / `#E6F7EA` | `#6EE7A0` / `#173A24` |
| 较多 | 精良 · 蓝 | `#1C4BC4` / `#E4EFFF` | `#7CC4FF` / `#14304D` |
| 中等 | 史诗 · 紫 | `#6B28C9` / `#F3ECFF` | `#C8A8FF` / `#2F2350` |
| 较少 | 传说 · 橙 | `#B03C08` / `#FFEEDA` | `#FFB066` / `#46280F` |
| 很少 | 神话 · 红 | `#B91C1C` / `#FFE6E3` | `#FF8F85` / `#4A2320` |
| 极少 | 神器 · 金 | `#7A5C00` / `#FFF3C4` | `#FFD75E` / `#4A3A0D` |
| 未收录 | —— | `#98A2B3` / 透明 + 虚线 | `#667085` / 透明 + 虚线 |

浅色模式前景对自身底色对比度 ≥ 4.5（WCAG AA 正文），深色模式 ≥ 6。
橙 / 红 / 金 是相邻的三档暖色，靠色相 + 明度拉开；要调只需动这三档的明度。
唯一权威定义在 `apps/desktop/src/lib/tier-colors.ts`，本表与 CLI 的 `TIER_RGB` 必须跟它一致。

## 6. CLI 接口

```
vocfreq detect   --corpus <DIR>                        # 只探测 schema，不扫描
vocfreq scan     --corpus <DIR> --out <DIR> [选项]      # 统计，产出各表组的表（默认不含 full）
vocfreq info     --table <FILE.vfr>                    # 打印头信息与若干条目
vocfreq lookup   --table <FILE.vfr> <WORD>...          # 验证索引查询
vocfreq segment  --data <OUTDIR> "<文本>"               # 分词并附频率，用于验证前端逻辑
vocfreq oov      --data <DIR> [--scope news]           # 从已有产物重导词典外候选，无需重扫
vocfreq curve    --data <DIR> --table news/word         # 覆盖率曲线，把覆盖率目标换算成排名阈值
vocfreq pct      --data <DIR> --table full/word         # 前%上界 ↔ 排名阈值（默认口径的换算工具）
vocfreq compose  --from-data <DIR> --source a/word --source b/word --scope 相加  # 把几张表加起来
vocfreq merge    --from-data <DIR> --from-data <DIR> --scope 主表 --out <DIR>     # 把几份产物合流成一份
vocfreq prepare-seed --from-data <DIR> --dict <FILE> --out <DIR>                 # 摘出安装包预置内容
```

表的指代一律是 **`表组/类型`**（`full/word`、`news/char`、`相加：财经/word`）。
`curve` / `pct` 找不到表时会把这份产物里**实际可用**的表列出来 —— 老写法的
`domains/news/word` 因此会立刻得到一句能照着改的提示，而不是"文件不存在"。

`scan` 选项：`--threads N`（默认物理核数）、`--hmm`、`--dict FILE`（可重复）、
`--dict-dir DIR`、`--user-dict FILE`（可重复）、
`--rules FILE`、`--min-count N`、`--keep-digit`、`--no-latin`、`--skip-single-char`、
`--no-tsv`（不写可读 TSV，只要二进制索引）、`--domains a,b,c`、`--no-domains`
（不要各表组的子表）、`--full`（**顺带**产出全量表组 `full`；默认不产，它由
"合流 + 相加"得到，见 `DATA_LAYOUT.md` §七）、`--oov-min-count N`、
`--progress json|bar|none`。

`compose` 选项：`--from-data DIR`（源产物，**可重复**：源表可以跨产物，此时要求各产物的
词典链与分词口径完全一致，否则报错拒绝）、`--source 表组/类型`（可重复，至少一个）、
`--scope 新表组名`、`--kind word|char`（不给 = 源表里出现过的每一类都相加）、
`--out DIR`（不给 = 写回第一个源产物）。

`merge` 选项：`--from-data DIR`（**可重复，至少两个**）、`--out DIR`（目标父目录）、
`--scope 目标产物名`（不给 = `--out` 本身就是产物目录）。合流的硬门槛见
`DATA_LAYOUT.md` §七：词典链与分词口径逐份校验，不一致就拒绝，且**绝不产出半截产物**。

**进度上报**：CLI 用 `--progress json` 向 **stderr** 逐行输出 JSON 事件，供命令行场景把进度接给外部工具解析。
桌面端**不解析这份 JSON** —— 它直接复用 core 的 `scan::scan` 回调拿到 `Progress`，在后台线程里统计，
再把 `scan:progress` 事件推给前端驱动进度条（见 `apps/desktop/src-tauri/src/lib.rs`）：

```json
{"event":"plan","files":77,"bytes":35740000000}
{"event":"phase","phase":"scan","domain":"wiki","units_done":3,"units_total":23,"bytes_done":1500000000,"bytes_total":8020000000,"percent":0.18}
{"event":"table","table":"news/word","entries":3123456,"total_tokens":0,"tier_stats":[]}
{"event":"done","elapsed_ms":84000,"out":"…"}
```

> `Progress::Plan/Table` 里的 `domains` / `table` 字段名保持不变（前端契约），但它们现在的
> 含义分别是「**语料切片**清单」与「**表组/类型**」：`table` 从 `full/word` 变成 `full/word`
> 其实没变 —— 因为表身份本来就长这样，变的只是磁盘布局与 `meta.tables[].path` 的语义。

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

六个页面：

1. **生成词频表**：选语料库目录 → 显示探测到的 schema 与文件清单/体积 → 配置分词与输出 → 启动（后台线程调 `scan::scan`，进度经 `scan:progress` 事件推给前端）→ 展示各分组覆盖率。
2. **划句分析**（核心）：输入框 + 划选文字，按词着色展示（**颜色只看主词频表**），悬停显示频次/排名/**前%**/占比/分组/是否词典外词；「对比范围」勾选要看哪些表组，每个 token 下方列出它在各表里的**前%**；支持把全文发到悬浮小窗。
3. **排行榜**：分页浏览、按词首前缀搜索、按七组筛选、切换表组（默认主表）、「前%」列、词条详情。
4. **词典管理**：浏览/增删数据文件夹 `dicts\` 下的词典，按指纹校验当前词频表与词典是否一致。
5. **表管理**：**一张统一的频率表清单**（每个表组 = 一张词频表 + 一张字表，完全平级）+
   **主词频表单选** + **相加**（把若干表加起来成一张新表）+ 分组自定义。
6. **设置**：全局热键、小窗置顶/透明度/尺寸/自动关闭、主题、默认分词选项、默认语料库与产物目录。

### 8.1 Tauri 命令接口（已冻结）

所有命令返回 `Result<T, String>`，错误信息为中文。字段名一律 **snake_case**（与 `vocfreq-core` 的 serde 输出一致）；**入参结构体**用 `#[serde(rename_all = "camelCase")]`，因此前端传 camelCase。

| 命令 | 入参 | 返回 |
|---|---|---|
| `app_info` | — | `AppInfo { name, version, tauri_version, core_version }` |
| `plan_corpus` | `corpus` | `CorpusPlan { corpus, files, bytes, domains[{name,files,bytes,rules}], warnings }`（`domains` 是**语料切片**） |
| `dataset_status` | `dir` | `DatasetStatus { dir, exists, meta }`（不抛错，`exists=false` 时 `meta=null`；判据是 `Dataset::open` 能不能成功） |
| `open_dataset` | `dir` | `Meta`（把已有产物目录装入内存并缓存分词器） |
| `analyze_text` | `text`, `domains`, `dir?` | `Vec<TokenInfo>`（**着色/频次/分组一律查主表组**；`domains` 只过滤对比列 `table_ranks` 里保留哪些表组，空 = 全留） |
| `tier_curve` | `path`, `dir?`, `maxPoints?` | `TierCurve { path, kind, entries, total_tokens, points: [[rank, 累计覆盖率]] }` |
| `lookup_word` | `word`, `kind?`, `dir?` | `Option<WordHit>`（含 `top_pct` / `entries` / `scope`） |
| `list_rank` | `domain?`, `kind?`, `from?`, `limit?`, `dir?` | `Vec<RankRow>`（每行带 `top_pct` / `pct`；`domain` 空 = **主表组**） |
| `search_words` | `query`, `kind?`, `domain?`, `limit?`, `dir?` | `Vec<RankRow>` |
| `set_primary_scope` | `scope` | `Meta`（换主词频表；后端重开数据集，全部页面口径随之切换） |
| `compose_tables` | `params { sources[], scope, kind? }` | `Vec<ComposedTable>`（把若干张表相加成新表组；`spawn_blocking` 里跑，秒级重活） |
| `start_scan` | `params: ScanParams` | 立即返回；结果走事件 |
| `cancel_scan` | — | 置取消标志；产物目录会被删掉 `meta.json` 以标记不完整 |
| `get_settings` / `set_settings` | `settings` | `Settings`（持久化到应用配置目录；改 `primaryScope` 会顺手重开数据集） |
| `capture_selection` | — | `String`（空串 = 当前无选区） |
| `take_pending_selection` | — | `String`（小窗启动时取走待分析文本） |
| `open_popup` / `hide_popup` | `text?` | 显示/隐藏悬浮小窗 |

事件：`scan:progress`（判别联合，带 `event` 标签：`log` / `plan` / `phase` / `table`）、`scan:done`（`Meta`）、`scan:error`（`String`）。

`Meta.tables` 里带**每张表的阈值（`tiers` 绝对排名 + `tier_pct` 前%）与各带实际覆盖率**，界面必须从那里读，不要硬编码分组边界。

**`TokenInfo` 的新字段**（schema v3）：`top_pct`（主表里的前%）、`entries`（主表条目数）、
`table_ranks: TableRank[]`（取代 v2 的 `domain_ranks`，每项 `{scope, kind, rank, top_pct, entries, count}`）。
字段名有契约测试钉住（`token_info_field_names_are_a_frozen_contract`，同时钉住 `domain_ranks`
**必须已消失**）。

**分词器重建规则**：打开产物目录时，分词器必须按 `meta.tokenizer` 里记录的口径重建（hmm、过滤规则、**词典链**），否则分词结果对不上已落盘的词频表，查出来的频次会系统性偏错。

重建分两步，第二步才是这次词典外置的关键：

1. 过滤规则（hmm / min_len / max_len / keep_latin / keep_digit / skip_single_char）直接照抄 `meta.tokenizer`。
2. 词典链按 `meta.tokenizer.dicts[]` 里的**指纹 → 名字**在数据文件夹的 `dicts\` 里找回实际文件（不追记录里那个可能早已失效的绝对路径）。找不齐就退化到用现有词典，**但同时把告警记下来交给界面显示、写进 `startup.log`** —— 绝不静默照用。v1 老产物没有指纹，判为"无从校验"（`legacy`）而非"缺失"，见 §5.2。

CLI 的 `segment` 走同一套规则（`chain_from_meta`），并且会在指纹不一致时打警告。

### 8.2 全局取词与悬浮小窗

- 触发：全局热键（默认 `Alt+Q`，可改）。
- 取词：**剪贴板模拟法**——暂存剪贴板 → 模拟 `Ctrl+C` → 等 `GetClipboardSequenceNumber` 变化（不能靠固定 sleep，不同程序差异很大）→ 读走选区 → 立刻还原。不需要鼠标钩子。
  - 若原剪贴板内容不是文本（图片/文件），`arboard` 无法完整还原；此时**保留抓到的文本而不是清空剪贴板**。
  - 取词会阻塞数百毫秒，因此**必须在独立线程里做**，不能占着热键回调线程。
- 小窗：无边框、置顶、不占任务栏。窗口 label 固定为 `popup`，前端按 `getCurrentWindow().label` 决定挂载主界面还是小窗界面——这样**不需要给 Vite 配多入口**。
- 小窗内容：**输入框 + 着色卡片**，带 `data-tauri-drag-region` 的拖拽区与关闭按钮；关闭按钮走 `CloseRequested` 拦截改成隐藏，避免反复重建窗口。
- 位置：光标右下方，越界则翻到光标上方或贴边——宁可位置不完美也不能跑到屏幕外。
- 已知限制：目标程序若以管理员身份运行而 VocTier 不是，Windows UIPI 会阻止 `Ctrl+C` 送达；此时需以管理员启动 VocTier（设置页给出提示）。

### 8.3 界面多语言（i18n）

只做**界面**多语言；分词、统计、分组口径与分析文本的语言无关，不受影响。当前注册了
`zh-CN`（源语言），英文等翻译补齐后加文件即可生效。

**文件分工**

| 文件 | 职责 |
|---|---|
| `lib/messages/zh-CN.ts` | **源语言**。它的 key 集合就是 `MessageKey`，别处漏 key 直接编译失败 |
| `lib/messages/types.ts` | `Messages = Record<MessageKey, string>` —— 这就是「漏翻检测器」 |
| `lib/messages/index.ts` | 语言注册表 `LOCALES` / `MESSAGES` / 语言显示名 |
| `lib/i18n.svelte.ts` | `t()`、`locale` 状态、`setLocale`、分组标签解析 |
| `lib/locale-sync.ts` | 主窗口 ↔ 小窗的语言同步（StorageEvent + Tauri 事件总线两条路） |

**新增一种语言只需要两步**：新建 `messages/<lang>.ts` 写成 `const x: Messages = { … }`（漏 key 编译不过），
然后在 `LOCALES` / `MESSAGES` / `LOCALE_LABELS` 各加一行。设置页下拉、首屏 `<html lang>`、
小窗同步、分组标签全都从注册表读，不用改别处。

**语言与主题存两处，规则相同（这是踩过坑的，别改回去）**

| 存储 | 角色 |
|---|---|
| `localStorage['voctier-locale']` / `['voctier-theme']` | 首屏**同步**读的快速通道（避免闪成错语言/错主题），同时是**实时权威值**：切换后立即写入，两个窗口靠它 + 广播保持一致 |
| `Settings.locale` / `Settings.theme` | 设置文件里的记录，**只在点「保存设置」时写入**；`adoptLocaleFromSettings()` / `adoptThemeFromSettings()` **只在本机还没有记录时**（首装 / 清过 storage）才采用它 |

关键点：**设置文件里的这两个字段在保存前可能是旧值，绝不能拿它反过来覆盖实时状态**。踩过的坑：
设置页与小窗每次挂载都读一次设置文件并无条件采用，于是「切英文 → 离开设置页再回来」变回中文、
「顶栏切深色 → 进设置页」变回浅色；点「保存设置」时同理，会把表单里的旧值写回界面。所以
设置页的选中态（`locale.value` / `theme.mode`）与保存时发出去的字段都以共享状态为准，
不读本页表单副本。

**纯度规则（重要，别破坏）**

- `t()` 读 `$state`，因此是**响应式读点**：在模板 / `$derived` 里调用，切语言自动重渲染。
- **计算**类纯函数一律不调 `t()`、不读 locale：`format.ts` 的 `boundsInfo` / `effectiveBounds` /
  `rankForCoverage`、`tier-colors.ts` 全模块、`segments.ts` 的 `normalizeToken` / `summarizeTokens`。
  它们要么被大量 `$derived` 调用，要么被非组件代码复用；读全局状态会让返回值随语言漂移。
  需要提示用户时返回**错误码**，由展示层去 `t()`。
- `tier-colors.ts` 保持**叶子纯模块**（不依赖 `i18n.svelte.ts`，也不依赖 `format.ts`）：
  配色是「这组有多常见」的语义映射，**不该随界面语言变化**；而且它被大量组件复用，
  保持无状态、可单独复用最省事。数字本地化因此单独放在零依赖的 `lib/number-locale.ts`，
  由 `tier-colors.ts` / `format.ts` / `segments.ts` 共用同一个 `NUMBER_LOCALE`。
  - 副作用：Node 工具**不能**直接 `import` 这个 `.ts` —— Node 的类型剥离不做扩展名解析，
    而无扩展名 import 是本仓库的统一风格。`.tmp/pw/dump-tier-colors.cjs` 因此改走
    Vite dev server 的模块图（浏览器里 `import()`）来取真实调色板。
- 模块级常量里**不要存文案**：导航表 `NAV_ITEMS` 存的是 `labelKey` 而不是 `label`，
  因为常量只在模块加载时求值一次，存字符串会让语言切换失效。

**分组标签的本地化路径**：组号 → 产物自带的 `tier_keys` → `tier.<key>` 消息 → 查不到才回落到
`meta.tier_names`。所以接入新语言时**不需要改任何页面**，只要消息表里有 `tier.*` 七条，
图例、排行榜筛选、划句分析标签会一起变（见 §5.2 的身份说明）。

**刻意不翻的东西**：Rust 侧启动/崩溃日志、`crates/vocfreq-cli` 的终端输出（那是排障与命令行
交付物，见 README §十）；`Settings.locale` 之外的后端错误串仍是中文。

> **进度**
>
> 已完成：i18n 内核与 locale 持久化（`Settings.locale` + localStorage 镜像 + 跨窗口同步）；
> 共享层（导航 / 主题 / 布局骨架 / 格式 / 表名 / 数字 locale / `common.*` 短词）；
> 分组标签；`format.ts::boundsInfo` 的**错误码化**（`BoundsWarning = { key, params }`）；
> **设置页**与**生成词频表页**整页文案（各 0 处残留）。
> 为此新增 `i18n.svelte.ts::splitMessage()`：句子里要加粗的值不拆句，按占位符切段渲染。
>
> 待办：划句分析 / 排行榜 / 表管理三个业务页、悬浮小窗 `PopupApp`、
> `TokenDetail` / `TierStatsTable` 等组件正文，以及 `api/bridge.ts` 里**用户可见**的错误串
> （mock 假数据里的中文不算文案）。

> ⚠️ **改完消息表要重启 dev server 再验证**。
> 实测（本机 Windows + 大量 `target/` 目录）Vite 的文件监听会漏掉改动：服务端继续提供
> **旧版组件模块**，而消息表模块是新的，于是界面上会直接渲染出 `settings.paths.browse`
> 这样的**文案 key 字面量**。踩到这个现象时不要怀疑 `t()`，先确认服务端模块是不是旧的：
>
> ```powershell
> # 磁盘上已经没有这个 key 了，但服务端还在提供 → 说明是旧模块
> (Invoke-WebRequest http://localhost:1420/src/routes/SettingsPage.svelte -UseBasicParsing).Content -match 'settings\.paths\.browse'
> ```
>
> 惯用做法是**换一个端口起一个全新的 dev server** 再跑回归（`pnpm dev --port 1421 --strictPort`），
> 而不是在旧实例上反复刷新。

## 9. 二期（已确认留到后面）

**新词发现**：用互信息 PMI + 左右邻字熵扫全语料，挖出词典外候选新词（如「元宇宙」「直播带货」「情绪价值」），导出 `oov_candidates.txt`，再由 `--user-dict` 回填分词。
不是简单数 2~4 元组合——那会产生 99% 的语法垃圾（「的+续航」「车+的」）。
