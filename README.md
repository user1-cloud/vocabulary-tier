# VocTier

中文字词频率分析工具。用你自己的语料库算出**真实的**字词频率，再拿它去回答「这句话里哪些词常见、哪些词罕见」。

两个交付物：

| 交付物 | 位置 | 说明 |
|---|---|---|
| **vocfreq** —— 纯 Rust 命令行工具 | `crates/vocfreq-cli` | 不依赖 Tauri，可单独分发。扫描大规模语料库，产出字词排行榜与可 mmap 查询的二进制索引 |
| **VocTier 桌面端** —— Tauri + Svelte 5 | `apps/desktop` | 主要的软件前端。调用同一个统计核心，提供划句分词着色、排行榜浏览、词库/表管理、设置，以及全局热键取词与悬浮小窗 |

两者共用 `crates/vocfreq-core`，因此**命令行算出来的表和桌面端用的表是同一套代码产出的**，不会出现口径不一致。

---

## 一、为什么要自己统计，而不是用 jieba 自带的词频

jieba 词典第三列确实是词频，但**它是分词用的概率权重，不是词频统计**。实测你手上这份 `dict.txt`（349,045 条 —— 注意长期在文档与代码里写的 349,046 是个**多算一条**的数字，它来自 jieba 内部 `count('\n') + 1` 的预留估算，不是真实条目数；`vocfreq` 现在报的是逐行解析出来的真实值）：

| 现象 | 数据 |
|---|---|
| 权重恰好等于保底值 `3` 的条目 | **159,318 条（45.6%）** |
| 权重等于 `2` 的条目 | 40,502 条 |
| 两者合计 | **57.2%** |
| 「的」的排名 | 第 **9** 位（318,825），低于「了」（883,634） |

真实中文语料里「的」永远排第一。所以拿它判断稀有度，一半以上的词会显示成「一样罕见」。

于是本工具用 jieba 负责**怎么切**，用你的语料库负责**数出多常见**。

---

## 二、快速开始

### 1. 先跑命令行工具建立词频表

```powershell
# 看看语料库都有哪些格式、多少数据
cargo run --release --bin vocfreq -- detect --corpus "E:\path\to\语料库"

# 全量统计（本机 33 GB 语料约 90 秒）
cargo run --release --bin vocfreq -- scan `
    --corpus "E:\path\to\语料库" `
    --out    ".\data" `
    --progress bar
```

产物（**每个作用域一个平级目录**）：

```
data/
├─ meta.json              构建元信息、语料切片清单、每张表的阈值与前%、来源
├─ news/word.tsv          每个一级子目录各一套（word + char）
├─ news/word.vfr          二进制索引，供 mmap 查询
├─ news/char.tsv/.vfr     该作用域的字表
├─ wiki/ … blog/ …        其余作用域
├─ full/word.tsv/.vfr     全量（= 所有作用域相加），**由 compose 产出**，见 §四
├─ 相加：财经/word.vfr     相加出来的新表，与上面完全平级
└─ oov_candidates.tsv     词典外高频候选词，可喂给 --user-dict
```

> **`scan` 默认不产出 `full`。** 只写各作用域自己的表；`full` 由
> 「`vocfreq merge` 合流多份产物 + `vocfreq compose` 把作用域相加」得到。
> 全量语料常常大到一块盘放不下，只能一个域一个域地扫成多份产物，那种工作流里每次
> `scan` 都写一遍 `full` 纯是浪费（而且因为整体覆盖，前几次写的会被冲掉）。
> 需要一份全量对照基准时用 `--full` 显式要它。
>
> **`full` 没有特权。** 谁负责回答「这个词有多常见」由你在应用里指定的**主词频表**
> 决定（默认 `full`，没有 `full` 时用排序后的第一个作用域）。所有表都能当主表，
> 也都能互相相加。
>
> ⚠ 老产物（`full/` + `domains/` 两棵树那种）现在**读不了**，会明确提示重新统计。

### 2. 分析一句话

```powershell
# 分词 + 频率信息 + 分组
vocfreq segment --data .\data "数字经济与人工智能深度融合"

# 带 ANSI 真彩，预览界面配色
vocfreq segment --data .\data --color "数字经济与人工智能深度融合"

# JSON（桌面端走同一套核心逻辑）
vocfreq segment --data .\data --json "数字经济与人工智能深度融合"
```

输出示例（真实数据）：

```
词              频次         排名           前%        占比  分组     标记
新           1506391         18      0.000473%  0.50431%  极多     单字→字表
               各表对比: news 前0.0006%  wiki 前0.0009%  gov 前0.0005%
时代          238200         60      0.001577%  0.14697%  极多
中国          863676          8      0.000210%  0.53289%  极多
人工智能        30979        805      0.021160%  0.01911%  很多
```

「前%」= 排名 ÷ 主词频表的条目数 × 100 —— 跨表可比，排名绝对值不可比
（分域表只有几万条、全量表有几百万条，同一句词在两张表里的名次差几十倍很正常）。

### 3. 启动桌面端

```powershell
cd apps\desktop
pnpm install
pnpm tauri dev
```

---

## 三、命令行参考

```
vocfreq detect   --corpus <DIR>                    # 只探测格式，不统计
vocfreq scan     --corpus <DIR> --out <DIR> [选项]  # 统计，产出各作用域的表（默认不含 full）
vocfreq info     --table <FILE.vfr>                # 查看产物头信息与样例
vocfreq lookup   --table <FILE.vfr> <词>…           # 查词频与排名
vocfreq segment  --data <DIR> "<文本>"              # 分词 + 频率信息（含前%与各表对比）
vocfreq oov      --data <DIR> [--scope news]        # 从已有产物重导词典外条目，无需重扫
vocfreq curve    --data <DIR> --table news/word     # 覆盖率曲线，把覆盖率目标换算成排名阈值
vocfreq pct      --data <DIR> --table full/word     # 前%上界 ↔ 排名阈值（默认口径的换算工具）
vocfreq compose  --from-data <DIR> --source a/word --source b/word --scope 相加
vocfreq merge    --from-data <DIR> --from-data <DIR> --scope 主表 --out <DIR>   # 把几份产物合流
vocfreq prepare-seed --from-data <DIR> --dict <FILE> --out <DIR>   # 从产物摘出预置内容，供安装包携带
```

表的指代一律是 **`作用域/类型`**（`full/word`、`news/char`、`相加：财经/word`）；
找不到表时命令会把这份产物里实际可用的表列出来，老写法 `domains/news/word` 因此会立刻
得到一句能照着改的提示。`--no-domains` 表示"不要各分域的子表"；要和 `--full` 一起用，
否则产物里一张表都没有。

### 相加：把几张表加起来

```powershell
# 把 news 与 wiki 两个作用域相加成一张新表（写在同一个产物目录里，和别的表完全平级）
vocfreq compose --from-data .\data --source news/word --source wiki/word --scope "相加：新闻与维基"
```

相加在数学上是**精确**的：扫描本身就是"逐作用域扫完再累加"，每个 token 只属于一个作用域，
所以各作用域表相加**逐条等于**全量扫描出来的 `full`（有测试拿扫描产物的分域表相加与
`full` 逐条比对来钉这件事）。落地方式是**物化成真实 `.vfr`**，代价是一份表大小的磁盘，
换来排行榜翻页与覆盖率曲线仍是 O(1) / 顺序读；源表不需要了可以删掉回收。

源表**可以跨产物**（`--from-data` 可重复）：全量语料大到一块盘放不下、只能分几次扫时，
就是这条路的用法。前提是各产物的**词库链与分词口径完全一致**，不一致会直接报错拒绝，
不会静默合并 —— 频次是同一套切分规则下的计数，凑合加起来会得到一份自相矛盾的表。

在桌面端不用敲命令：「表管理」页勾几张表 → 起个名字 → 相加；加出来的表能直接设成主表，
也能再被相加。

### 合流：把几份产物并成一份（分域扫描的工作流）

全量语料可能大到**一块盘放不下同时在场**（实测 140 GB）。这种时候只能一个域一个域地
扫，最后把分几次扫出来的产物并起来：

```powershell
# 1) 一个域一份产物 —— scan 默认不写 full，所以每份只有自己那个作用域
vocfreq scan --corpus D:\语料 --out D:\分片\A --dict 词库.dict --domains news
vocfreq scan --corpus D:\语料 --out D:\分片\B --dict 词库.dict --domains wiki
vocfreq scan --corpus D:\语料 --out D:\分片\C --dict 词库.dict --domains book

# 2) 合流成一份标准产物（作用域各自平级，谁也不动谁的频次）
vocfreq merge --from-data D:\分片\A --from-data D:\分片\B --from-data D:\分片\C `
              --scope 主表 --out D:\词表

# 3) 把各作用域相加出 full
vocfreq compose --from-data D:\词表\主表 `
                --source news/word --source wiki/word --source book/word `
                --source news/char --source wiki/char --source book/char `
                --scope full
```

`D:\词表\主表\` 是一份**标准产物**，桌面端「表管理」直接就能打开，不需要任何改动。

**合流的硬门槛是词库链与分词口径完全一致**：`merge` 逐份校验 `meta.json` 里的
`tokenizer`（词库链逐份比 `sha256`，外加 `hmm` / `min_len` / `max_len` / `keep_latin` /
`keep_digit` / `skip_single_char` / `engine` / `version`），任何一处不同都**报错拒绝**并
指出是哪一份产物、哪个字段不同。没有指纹的老产物也拒绝 —— "两份都没有记录"看着一致，
其实什么也没说明。它**不合并词库链、不挑一份当基准、不产出半截产物**（目标目录要么不
存在，要么是完整的）。作用域名跨产物撞车同样拒绝。

需要一份显式的全量对照基准时，用一次 `--full` 的全量扫描：

```powershell
vocfreq scan --corpus D:\语料 --out D:\基准 --dict 词库.dict --full
```

完整说明（含"哪些字段不被当成门槛"）见 [`docs/DATA_LAYOUT.md`](docs/DATA_LAYOUT.md) §七。

`scan` 的常用选项：

| 选项 | 说明 |
|---|---|
| `--threads N` | 线程数，0 = 自动（默认） |
| `--hmm` | 开启 HMM 新词发现。**默认关闭**，因为 HMM 会让同一实体在不同上下文被切成不同形态，拆散频次、使排行不可复现 |
| `--dict FILE` | 指定词库（jieba 格式 `词 词频 词性`，词频与词性可省略）。**可重复，第一份是主词库**，其余依次叠加 |
| `--dict-dir DIR` | 取目录里所有 `.dict` 组成词库链（按文件名排序），排在 `--dict` 之后 |
| `--user-dict FILE` | 追加叠加词库（可重复），排在最后。其中的词会被标记成「来自用户词典」 |
| `--domains a,b` | 只统计指定域 |
| `--no-domains` | 不产出分域子表（要和 `--full` 一起用，否则一张表都没有） |
| `--full` | **顺带**产出全量作用域 `full`。默认不产：它由「合流 + 相加」得到，见上面那节 |
| `--min-count N` | 只保留出现 ≥ N 次的词条 |
| `--keep-digit` | 保留纯数字 token（默认丢弃） |
| `--no-latin` | 丢弃纯英文 token |
| `--skip-single-char` | 单字不进词表（单字另有字表承载） |
| `--no-tsv` | 不写可读 TSV，只要二进制索引 |
| `--oov-min-count N` | 词典外候选词的最小频次（默认 500） |
| `--progress json` | 向 stderr 逐行输出 JSON 事件，供桌面端解析 |

> ⚠ **词库必须显式指定。** 从词库外置那次改动起，jieba 的词典不再编进 exe 了 ——
> 不指定就是直接报错，而不是悄悄用一份内置词典跑出一张口径不明的表。
> 完整的目录模型见 [`docs/DATA_LAYOUT.md`](docs/DATA_LAYOUT.md)。
>
> 词库格式只有三条规则：`词 词频 词性`（后两列可省）、以 `#` 开头的整行是注释、
> 空行跳过。**词频省略**时按 jieba 的 `suggest_freq` 折算；把词频**显式写成 `0`**
> 的词会被登记进词典却永远切不出来，两边都会出警告提醒。

### 词典外的条目：实测能捞到什么

关闭 HMM 时，**jieba 只能输出「词典命中的词」或「未命中的单字」**。这条约束决定了
「词典外候选」能提供什么，实测结果如下（全库词表 380 万条）：

| 筛选条件 | 结果 |
|---|---|
| 含汉字，最小词长 **2** | **0 条** —— 多字中文新词不可能作为独立 token 出现 |
| 含汉字，最小词长 1 | **1,082 条**，全是**繁体单字**：對(73万) 業(62万) 機(54万) 華(53万) 該(51万)… |
| 不限汉字，最小词长 2，频次 ≥20 万 | 62 条 **URL/代码碎片**：`com`(258万) `https`(138万) `App`(144万) `www` `chksm`… |

两个结论：

1. **「元宇宙」「区块链」「直播带货」这类词永远不会出现在候选里**。因为 jieba 会把它们切成 `元/宇宙`、`区块/链`，候选榜上你只会看到碎片。要让它们成词，只有两条路：开 `--hmm`，或等二期的**新词发现**（互信息 + 左右邻字熵）——那也是唯一能从语料里**主动挖出**多字新词的办法。
2. 含汉字的候选为什么全是繁体字？因为 **jieba 词典只有简体**，而你的语料（申报、旧人民日报、繁体维基）含大量繁体文本。这批候选是真能用的：

```powershell
# 用导出的候选做成叠加词库，再跑一次
vocfreq oov --data .\data --min-count 500          # 写 data\oov_candidates.tsv
# 取前 30 个繁体字做成叠加词库，再跑一次（--dict 指定主词库，--user-dict 叠加）
vocfreq scan --corpus "…" --out .\data `
             --dict .\dicts\预制词库.dict --user-dict .\my_dict.dict
```

> 这份 `oov_candidates.tsv` 就是 `#` 注释那个坑的现场：它的开头有 10 行 `#` 说明，
> 而 **jieba 自己的 `load_dict` 不认注释** —— 第二列不是整数就直接
> `InvalidDictEntry` 报错。所以词库读取一律走 `vocfreq_core::dict`，它先剥注释、
> 整份校验，再喂给 jieba。实测旧版本拿这份文件喂 `--user-dict` 是**必然失败**的。

验证效果（同一个繁体句）：

```
# 不带用户词典
處    420021    1191    中等    词典外 单字→字表
# 带上用户词典
處    420021    1191    中等    用户词典 单字→字表
```

`oov` 子命令可以**在已有产物上反复重跑**，调阈值不需要重新统计：

```powershell
vocfreq oov --data .\data --min-count 100 --min-len 1          # 只看含汉字的
vocfreq oov --data .\data --min-count 200000 --no-cjk-only     # 看 URL/代码噪声，用来排查清洗规则
```

---

## 四、性能实测

本机：Ryzen 9 8945HX（16C/32T）+ 31 GB RAM + Samsung PM9A1 1 TB NVMe。

| 项目 | 实测 |
|---|---|
| 语料库 | 77 个 `.jsonl`，33.3 GB，5 种 JSON schema |
| 磁盘纯顺序读 | 33.27 GB / **13.1 s**（2600 MB/s，两遍一致） |
| **全量统计（含分词、计数、排行、落盘）** | 33.3 GB / **约 90 秒** |
| 其中排行 380 万词 + 写 138 MB TSV + 91 MB 索引 | 约 3 秒 |
| 单线程分词吞吐 | 46~93 MB/s（随文本难度变化） |
| 32 线程分词吞吐（缓存命中） | 最高 1391 MB/s |

几个实测结论已固化进代码，见 `docs/DESIGN.md` 第 0 节：

1. **语料里有非法 UTF-8 的行**（爬取的维基数据）。用 `read_line` + `Err => break` 会让一行坏数据废掉整个区间（实测 4 线程有 2 个区间直接归零，丢掉 262 MB）。现在全程 `read_until(b'\n')` 按字节读行，坏行只跳过、绝不断流，并统计坏行数。
2. **丢弃区间边界半行必须复用同一个 `BufReader`**。另建一个 reader 会在析构时带走内部已缓冲的约 8 KB，导致每个区间开头读到一行截断数据。
3. **计数绝不能用互斥锁分片哈希表**：锁竞争让 32 线程比 16 线程更慢（501 → 434 MB/s）。改成每线程独占 + 末尾归并后为 848 MB/s 且单调扩展。
4. **`Cow<'de, str>` 不是零拷贝**。serde 对 `Cow` 只有一条「先反序列化成 Owned 再包起来」的通用实现，永远分配。用它读 JSON 键等于每个键都造一个 `String`；换成自定义的 `KeyStr` 后 gov 域快了 5 倍。
5. **区块大小不是越小越好**：16 MB 区块（全库 2000+ 并发读流）比 64 MB 慢 60%。这台盘单流能跑 2600 MB/s，但扛不住几百个并发流。

---

## 五、频率分组

默认按**前%**分七组：`前% = 排名 ÷ 该表条目总数 × 100`，也就是「这个词排在前百分之几」。
**词表与字表的前%不同**：字表只有约 1.9 万个不重复字，套用词表那套会让头几档只剩个位数的字。

| 分组 | 词表前% | 字表前% | ≈ 全库词表排名（380 万条） |
|---|---|---|---|
| 极多 | 前 0.0026% | 前 0.26% | ≤ 99 |
| 很多 | 前 0.026% | 前 1.05% | ≤ 990 |
| 较多 | 前 0.132% | 前 3.16% | ≤ 5,024 |
| 中等 | 前 0.526% | 前 7.89% | ≤ 20,017 |
| 较少 | 前 1.32% | 前 15.8% | ≤ 50,231 |
| 很少 | 前 3.95% | 前 26.3% | ≤ 150,313 |
| 极少 | 前 100% | 前 100% | 其余全部 |
| **未收录** | 单独一类，灰色 + 虚线下划线 | 同左 | —— |

词表那六档不是新拍的：它们就是把**旧的绝对排名阈值**（100 / 1000 / 5000 / 20000 /
50000 / 150000）放在实测的 380 万条全库词表上换算出来的，所以换成前%之后**色阶观感与
从前一致**。这样做的理由是：排名绝对值只在同一张表内有意义，而分域表只有几万条、
全量表有几百万条 —— 同一个绝对阈值套上去，分域表会整片挤进「极多」。前%只跟位次有关，
换表、换主表都不会失真。

阈值**不是硬编码的**：`meta.json` 里带每张表的前%上界（`tier_pct`）、绝对排名阈值
（`tiers`）和**各档实际覆盖率**，界面读它而不是自己猜。首轮全量跑完的实测覆盖率
（全库词表，380 万条，26.43 亿 token）：

| 分组 | 本组覆盖 | 累计覆盖 |
|---|---|---|
| 极多（前 0.0026%） | 26.7% | 26.7% |
| 很多（→0.026%） | 28.9% | 55.6% |
| 较多（→0.132%） | 22.8% | 78.3% |
| 中等（→0.526%） | 12.5% | 90.8% |
| 较少（→1.32%） | 4.9% | 95.8% |
| 很少（→3.95%） | 约 3% | 约 99% |
| 极少（前 100%） | 约 1% | 100% |

只有 **99 个词**就盖住了 26.7% 的正文，前 990 个盖住 55.6%——这就是 Zipf 分布的真实形态。
阈值是按这套覆盖率校准的：早期版本把「极多」定在 ≤500，那 500 个词就吃掉 45% 的正文，
导致「极多」和「很多」看起来差不多、色阶失去区分度。

> **给以后调阈值的人**：改 `crates/vocfreq-core/src/rank.rs` 里的 `DEFAULT_TIER_PCT` /
> `DEFAULT_CHAR_TIER_PCT`，然后**重跑一次统计**（阈值与覆盖率都写在 `meta.json` 里、
> 界面从那里读）。想先看效果不必重新分词：
>
> ```powershell
> vocfreq pct   --data .\data --table full/word          # 前%上界 -> 排名阈值
> vocfreq curve --data .\data --table full/word          # 覆盖率曲线
> ```
>
> 设置页里还能改用**绝对排名**、**累计覆盖率**或**词条数等分**三套口径（`tierMethod`），
> 但默认与推荐都是前%。

---

## 六、语料库格式

内置 4 条规则，自动探测（按顶层 key 匹配）：

| 规则 | 顶层字段 | 文本字段 | 覆盖 |
|---|---|---|---|
| `mnbvc_paragraph` | `段落`（数组） | `内容` | blog / book / wiki / news / gov 的绝大多数，**含 warc/html 文件** |
| `mnbvc_forum` | `回复`（数组） | `回复` + `主题` | forum，内容含 HTML 需清洗，且存在空对象元素 |
| `plain_text` | — | `text` | gov/GovReport |
| `parallel_subtitle` | — | `zh_text`，空则退 `cht_text` | parallel/subtitle |

自定义规则（`--rules rules.json`）：

```json
[
  { "name": "mnbvc_paragraph", "array_key": "段落", "text_key": "内容", "strip_html": false },
  { "name": "my_format", "plain_key": "content", "alt_text_keys": ["body"], "strip_html": true }
]
```

---

## 七、目录结构

```
voctier/
├─ Cargo.toml                  # workspace（只含 crates/）
├─ docs/DESIGN.md              # 设计文档：产物格式、色阶、实测结论
├─ docs/DATA_LAYOUT.md         # 数据文件夹：词库与词表的目录模型、安装包预置怎么落地
├─ assets/seed/dicts/          # 预置词库原件（jieba 的 dict.txt，MIT，4.84 MB），随仓库携带
├─ crates/
│  ├─ vocfreq-core/            # 统计核心：解析 / 清洗 / 分词 / 计数 / 排行 / 产物 / 查询 / 词库 / 相加
│  └─ vocfreq-cli/             # 纯 Rust CLI，产出 vocfreq.exe
├─ tools/
│  ├─ prepare-seed.ps1         # 组装安装包要携带的预置词库与预置词表
│  ├─ build-desktop.ps1        # 构建桌面端（-Bundle 出 NSIS 安装包）
│  └─ dev-settings.ps1         # 直接改桌面端设置，省得每次在界面里点
└─ apps/desktop/               # Tauri v2 + Svelte 5（独立 workspace）
   └─ src-tauri/
      ├─ nsis/installer-hooks.nsh   # 安装钩子：把预置内容直写进用户数据目录
      └─ seed/                      # 组装好的预置内容（不入库，见 .gitignore）
```

`apps/desktop/src-tauri` 用空 `[workspace]` 表与上层 workspace 隔离，互不影响。

---

## 八、已知限制

- **全局取词用剪贴板模拟法**：暂存剪贴板 → 模拟 Ctrl+C → 读走选区 → 还原。因此
  - 若原剪贴板内容不是文本（图片、文件），无法完整还原；此时会**保留抓到的文本而不是清空剪贴板**；
  - 目标程序若以管理员身份运行而 VocTier 不是，Windows 的 UIPI 会阻止 Ctrl+C 送达，需要在设置里改用管理员启动。
- **HMM 默认关闭**：词典外的新词会被切成碎片。回填 `--user-dict` 是解决办法（见第二节）。
- **新词发现（PMI + 左右邻字熵）尚未实现**，属于二期。它和「词典外词回填」是同一件事的两半。
- **老产物（schema < 3）不能读**：从前是 `full/` + `domains/` 两棵树、把作用域与类型糊在
  `full/word` 这样的路径里，现在每个作用域各占一个平级目录。这种布局变化无法就地迁移，
  打开时应用会明确说明并要求用同一套语料库与词库**重新统计**（CLI 的 `curve` / `pct`
  同样会报错并列出这份产物里实际可用的表）。
- **相加会多占一份磁盘**：`compose` / 「表管理 → 相加」是把结果物化成真实 `.vfr`，
  大约等于一张扫描出来的表。源表不需要了可以删掉回收；真正的"零拷贝虚拟合成"留到以后
  （`meta.json` 的 `source_tables` 就是为它留的接口）。
- **`scan` 默认不产出 `full`**：只写各作用域自己的表，`full` 由「`merge` 合流 +
  `compose` 相加」得到（`--full` 可显式要一份全量，通常只为留对照基准）。这样分域
  分次扫才不会被"每次整体覆盖 `full` 与 `meta.json`"冲掉前几次的结果。
- **合流要求词库链与分词口径完全一致**：`merge` 与多源 `compose` 会逐份校验
  `tokenizer` 并比 `sha256`，不一致直接拒绝；没有指纹的老产物也拒绝。它不会替你合并
  两条词库链，也不会留下一份半截产物。
- 绝对排名那套分带阈值仍然可用（设置里切 `tierMethod = rank`），但**默认是前%**：
  绝对阈值只在同一张表内有意义，换一张规模差很多的表（或换主表）就会失真。

## 九、许可证

MIT。jieba-rs 与其词典同为 MIT。

---

## 十、故障排查

### 桌面端白屏，并显示「无法访问此页面 / localhost 拒绝连接」

**这是最常见的坑，而且几乎全是构建方式不对造成的。**

Tauri 用**编译期** `cfg(dev)` 决定加载哪个地址：

```rust
#[cfg(dev)]      let url = config.build.dev_url;        // → http://localhost:1420
#[cfg(not(dev))] let url = ...frontend_dist...;         // → 内嵌的前端资源
```

这个 cfg 由 **tauri CLI** 通过环境变量驱动 build script 设置。如果直接
`cargo build --release`，就绕过了 CLI，被编成 **dev 模式**——运行时去连
`http://localhost:1420`，没有 dev server 就白屏。

**正确做法**（永远走 Tauri CLI）：

```powershell
cd apps\desktop
pnpm tauri build --no-bundle    # 只出 exe
pnpm tauri build                # 出 NSIS 安装包
# 或者用封装好的脚本：
.\tools\build-desktop.ps1
.\tools\prepare-seed.ps1        # 先组装安装包要带的预置词库/词表
.\tools\build-desktop.ps1 -Bundle
```

安装包**只出 NSIS**（MSI 暂时不做）。预置内容由安装钩子
（`apps\desktop\src-tauri\nsis\installer-hooks.nsh`）直接写进用户数据目录
`%LOCALAPPDATA%\com.voctier.desktop\data\`，**不在 `$INSTDIR` 留副本** ——
装在 `Program Files` 里的东西普通用户不可写，而预置内容必须能被用户删改。
细节见 [`docs/DATA_LAYOUT.md`](docs/DATA_LAYOUT.md)。

`cargo build` **只在**检查 Rust 侧能否编译时使用（`cargo check` 足够），不要拿它的产物去运行。

复核一个 exe 是哪种模式：

```powershell
.\tools\build-desktop.ps1 -Check
```

或直接看 `%APPDATA%\com.voctier.desktop\startup.log` 里的「构建模式」那一行。

> 这个坑特别难查，因为故障现象具有欺骗性：**窗口会正常出现、全局热键能注册、
> 后端日志一切正常、词频表也能载入**，只有 WebView 里是空的。排查时务必确认
> 日志里有 `IPC get_settings：前端已加载并成功调用后端 ✔` —— 有这一行才说明
> 前端 JS 真的跑起来了。

### 桌面端启动时就退出（窗口一闪而过）

启动阶段 panic，去看 `%APPDATA%\com.voctier.desktop\crash.log`。应用已装 panic hook，
会把 panic 信息与 backtrace 写进去。

如果 `crash.log` 里是 `failed to create webview`，说明 WebView2 建不了窗口。应用会
依次尝试 5 个数据目录：

| 顺序 | WebView2 数据目录 |
|---|---|
| 1 | `VOCTIER_WEBVIEW_DATA_DIR` 环境变量（若设了） |
| 2 | WebView2 默认位置 `%LOCALAPPDATA%\<identifier>\EBWebView` |
| 3 | `%APPDATA%\com.voctier.desktop\webview2` |
| 4 | 程序所在目录 `\webview2-data` |
| 5 | 系统临时目录 `\voctier-webview2` |

**卡在第 2 步通常是那个目录被弄坏了**——例如应用曾被强制结束（任务管理器结束进程、
断电），`EBWebView` 会残留在不可用状态，后续启动直接失败。删掉即可：

```powershell
Remove-Item "$env:LOCALAPPDATA\com.voctier.desktop" -Recurse -Force
```

日志都在 `%APPDATA%\com.voctier.desktop\`：

- `startup.log` —— 每次启动都追加：exe 路径、工作目录、**构建模式**、每个 WebView2
  候选的成功/失败、载入了哪张词频表、热键是否注册成功、**前端有没有真的调通后端**
- `crash.log` —— panic 信息 + backtrace，以及前端 JS 的异常与「白屏检测」结果

排查时把这两个文件发出来即可。要换日志位置就设 `VOCTIER_LOG_DIR`。

> 补充：`<程序目录>\webview2-data` 或 `%APPDATA%\...\webview2` 下会出现一个 `EBWebView`
> 目录（几百 MB），这是 WebView2 的缓存，**属于正常现象**，不是安装残留。

### `--progress none` 为什么还会打印日志

`none` 只关掉**进度**，诊断日志仍然输出——否则统计跑完你什么摘要都看不到。
要完全静默请重定向 stderr：`vocfreq scan … 2>$null`。

### 统计结果和预期不符

- **先看 `meta.json`**：里面记录了分词口径（HMM 开关、是否保留数字/英文、**词库链**）
  与每张表的阈值、各带实际覆盖率。用 `vocfreq segment` 时若 HMM 设置与建表时不一致、
  或词库指纹与建表时不同，命令都会给出警告，但**频次仍可能系统性偏错**——务必用同一套口径。
  > 词库现在是数据文件夹里的文件，可能被换掉。`meta.json` 的 `tokenizer.dicts[]` 里存了
  > 每份词库的 `sha256`，桌面端的「词表管理」就是靠它标出「词库已变 / 找不到词库」的。
  > v1 老产物没有这个字段，只能判为「无从校验」并建议重算。
- **换了语料库要重跑统计**，不能只换 `--data` 指向的目录：分词粒度与阈值都固化在产物里。
- **换了词库也要重跑统计**，同理：词库决定了怎么切词，切法变了频次就全变了。
- **表之间比"多少名"没有意义**（见 `docs/DESIGN.md` §5.0）：分域表只有几万条、全量表有几百万条，
  同一句词在两张表里的名次可以差几十倍。要横向比就比**前%**（排名 ÷ 条目数）或 `pct`（占全部 token 的百分比）。
- **打开老产物时报「这是 schema v2 的老布局产物」**：那是布局变化导致的，不是文件损坏。
  用同一套语料库与词库重新统计一次即可，`meta.json` 与 `.vfr` 都无法就地迁移。

### 构建时 `LNK1104: 无法打开文件 ...Temp\lnk{...}.tmp`

`lib.exe` 在系统 TEMP 里建临时文件失败，通常是并发构建或多个进程抢同一个 TEMP。
把构建临时目录指到工作区内即可：

```powershell
$env:TMP = "$PWD\.tmp"; $env:TEMP = $env:TMP; cargo build --release
```
