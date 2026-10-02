# VocTier

中文字词频率分析工具。用你自己的语料库算出**真实的**字词频率，再拿它去回答「这句话里哪些词常见、哪些词罕见」。

两个交付物：

| 交付物 | 位置 | 说明 |
|---|---|---|
| **vocfreq** —— 纯 Rust 命令行工具 | `crates/vocfreq-cli` | 不依赖 Tauri，可单独分发。扫描大规模语料库，产出字词排行榜与可 mmap 查询的二进制索引 |
| **VocTier 桌面端** —— Tauri + Svelte 5 | `apps/desktop` | 主要的软件前端。调用同一个统计核心，提供划句分词着色、排行榜浏览、全局热键取词与悬浮小窗 |

两者共用 `crates/vocfreq-core`，因此**命令行算出来的表和桌面端用的表是同一套代码产出的**，不会出现口径不一致。

---

## 一、为什么要自己统计，而不是用 jieba 自带的词频

jieba 词典第三列确实是词频，但**它是分词用的概率权重，不是词频统计**。实测你手上这份 `dict.txt`（349,046 条）：

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

产物：

```
data/
├─ meta.json              构建元信息、分域清单、分带阈值、各带实际覆盖率
├─ full/word.tsv          全库词表（可读排行）
├─ full/word.vfr          全库词表（二进制索引，供 mmap 查询）
├─ full/char.tsv/.vfr     全库字表
├─ domains/<域>/…         每个分域各一套（news / wiki / blog / book / forum / gov / parallel）
└─ oov_candidates.tsv     词典外高频候选词，可喂给 --user-dict
```

### 2. 分析一句话

```powershell
# 分词 + 频率信息 + 分组
vocfreq segment --data .\data "数字经济与人工智能深度融合"

# 带 ANSI 真彩，预览界面配色
vocfreq segment --data .\data --color "数字经济与人工智能深度融合"

# JSON（桌面端走同一套核心逻辑）
vocfreq segment --data .\data --json "数字经济与人工智能深度融合"
```

输出示例（真实数据，gov 域）：

```
词              频次         排名         占比  分组     标记
新           1506391         18  0.50431%  极多     单字→字表
时代          238200         60  0.14697%  极多
中国          863676          8  0.53289%  极多
人工智能        30979        805  0.01911%  很多
```

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
vocfreq scan     --corpus <DIR> --out <DIR> [选项]  # 全量统计
vocfreq info     --table <FILE.vfr>                # 查看产物头信息与样例
vocfreq lookup   --table <FILE.vfr> <词>…           # 查词频与排名
vocfreq segment  --data <DIR> "<文本>"              # 分词 + 频率信息
vocfreq oov      --data <DIR> [选项]                # 从已有产物重导词典外条目，无需重扫
vocfreq curve    --data <DIR> --table full/word     # 看覆盖率曲线，并把覆盖率目标换算成排名阈值
```

`scan` 的常用选项：

| 选项 | 说明 |
|---|---|
| `--threads N` | 线程数，0 = 自动（默认） |
| `--hmm` | 开启 HMM 新词发现。**默认关闭**，因为 HMM 会让同一实体在不同上下文被切成不同形态，拆散频次、使排行不可复现 |
| `--user-dict FILE` | 叠加自定义词典（jieba 格式 `词 词频 词性`，词频可省略） |
| `--dict FILE` | 完全替换内置词典 |
| `--domains a,b` | 只统计指定域 |
| `--no-domains` | 不产出分域子表 |
| `--min-count N` | 只保留出现 ≥ N 次的词条 |
| `--keep-digit` | 保留纯数字 token（默认丢弃） |
| `--no-latin` | 丢弃纯英文 token |
| `--skip-single-char` | 单字不进词表（单字另有字表承载） |
| `--progress json` | 向 stderr 逐行输出 JSON 事件，供桌面端解析 |

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
# 用导出的候选做成用户词典
vocfreq oov --data .\data --min-count 500          # 写 data\oov_candidates.tsv
# 取前 30 个繁体字做成 userdict，再跑一次
vocfreq scan --corpus "…" --out .\data --user-dict .\my_dict.txt
```

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

按**排名绝对值**分七组（词表与字表阈值不同，因为字表只有约 1.9 万个不重复字，套用词表阈值会让所有字都落进「极多~很少」而失去区分度）：

| 分组 | 词表排名 | 字表排名 |
|---|---|---|
| 极多 | ≤ **100** | ≤ 50 |
| 很多 | ≤ 1,000 | ≤ 200 |
| 较多 | ≤ 5,000 | ≤ 600 |
| 中等 | ≤ 20,000 | ≤ 1,500 |
| 较少 | ≤ 50,000 | ≤ 3,000 |
| 很少 | ≤ 150,000 | ≤ 5,000 |
| 极少 | > 150,000 | > 5,000 |
| **未收录** | 单独一类，灰色 + 虚线下划线 | 同左 |

阈值**不是硬编码的**：`meta.json` 里带每张表的阈值和**各带实际覆盖率**，界面读它而不是自己猜。首轮全量跑完的实测覆盖率（全库词表，380 万条，26.43 亿 token）：

| 分组 | 本组覆盖 | 累计覆盖 |
|---|---|---|
| 极多（前 100 名） | 26.7% | 26.7% |
| 很多（→1,000） | 28.9% | 55.6% |
| 较多（→5,000） | 22.8% | 78.3% |
| 中等（→20,000） | 12.5% | 90.8% |
| 较少（→50,000） | 4.9% | 95.8% |
| 很少（→150,000） | 约 3% | 约 99% |
| 极少（>150,000） | 约 1% | 100% |

只有 **100 个词**就盖住了 26.7% 的正文，前 1000 个盖住 55.6%——这就是 Zipf 分布的真实形态。阈值是按这套覆盖率校准的：早期版本把「极多」定在 ≤500，那 500 个词就吃掉 45% 的正文，导致「极多」和「很多」看起来差不多、色阶失去区分度。

> **给以后调阈值的人**：改 `crates/vocfreq-core/src/rank.rs` 里的 `default_word_tiers` / `default_char_tiers`，然后**重跑一次统计**，因为覆盖率与阈值都写在 `meta.json` 里、界面从那里读。想先看效果可以拿 `data/full/word.tsv` 前若干行累加 `count` 列除以 `meta.json` 的 `total_tokens` 直接算，不用重新分词。

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
├─ crates/
│  ├─ vocfreq-core/            # 统计核心：解析 / 清洗 / 分词 / 计数 / 排行 / 产物 / 查询
│  └─ vocfreq-cli/             # 纯 Rust CLI，产出 vocfreq.exe
└─ apps/desktop/               # Tauri v2 + Svelte 5（独立 workspace）
   └─ src-tauri/
```

`apps/desktop/src-tauri` 用空 `[workspace]` 表与上层 workspace 隔离，互不影响。

---

## 八、已知限制

- **全局取词用剪贴板模拟法**：暂存剪贴板 → 模拟 Ctrl+C → 读走选区 → 还原。因此
  - 若原剪贴板内容不是文本（图片、文件），无法完整还原；此时会**保留抓到的文本而不是清空剪贴板**；
  - 目标程序若以管理员身份运行而 VocTier 不是，Windows 的 UIPI 会阻止 Ctrl+C 送达，需要在设置里改用管理员启动。
- **HMM 默认关闭**：词典外的新词会被切成碎片。回填 `--user-dict` 是解决办法（见第二节）。
- **新词发现（PMI + 左右邻字熵）尚未实现**，属于二期。它和「词典外词回填」是同一件事的两半。
- 排行分带阈值是按绝对排名切的。换一个规模差很多的语料库后，建议看一眼 `meta.json` 里的覆盖率再决定是否调整。

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
pnpm tauri build                # 连同 NSIS/MSI 安装包
# 或者用封装好的脚本：
.\tools\build-desktop.ps1
.\tools\build-desktop.ps1 -Bundle
```

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

- **先看 `meta.json`**：里面记录了分词口径（HMM 开关、是否保留数字/英文、用户词典路径）
  与每张表的阈值、各带实际覆盖率。用 `vocfreq segment` 时若 HMM 设置与建表时不一致，
  命令会给出警告，但**频次仍可能系统性偏错**——务必用同一套口径。
- **换了语料库要重跑统计**，不能只换 `--data` 指向的目录：分词粒度与阈值都固化在产物里。
- **分域排名不能横向比大小**（见 `docs/DESIGN.md` §5.0），跨域比较看 `pct`。

### 构建时 `LNK1104: 无法打开文件 ...Temp\lnk{...}.tmp`

`lib.exe` 在系统 TEMP 里建临时文件失败，通常是并发构建或多个进程抢同一个 TEMP。
把构建临时目录指到工作区内即可：

```powershell
$env:TMP = "$PWD\.tmp"; $env:TEMP = $env:TMP; cargo build --release
```
