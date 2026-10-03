# 数据文件夹：词典与词频表

这篇讲 VocTier 从「词典编进 exe」改成「**数据文件夹** + 出厂预置」之后的目录模型。

## 一、为什么要改

从前 jieba 的 349,045 条词典是用 `include-flate` 以 zstd 压进**每个**链接了
`vocfreq-core` 的二进制里的（`jieba-rs` 的 `default-dict` feature）。这带来三个问题：

1. **用户改不了。** 想加个词、想让某个领域更准，只能等作者重新发版。
2. **词频表口径不可校验。** 词典既然不可能被换，`meta.json` 里就只记了一句自由文本标签；
   一旦真换了，拿旧表去查就会静默拿到系统性偏错的频次，而且没人看得出来。
3. **顺带拖进 zstd-sys 这个带 C 代码的依赖**（`tools/build-desktop.ps1` 里为它写的
   TEMP 绕行就是因此存在）。

现在词典和词频表都是数据文件夹里的**普通文件**，用文本编辑器就能改。

## 二、目录布局

```
%LOCALAPPDATA%\com.voctier.desktop\     ← 数据文件夹的默认位置，设置里可改
└─ data\
   ├─ dicts\                            ← 词典库：一个词典就是一个 .dict 文件
   │  ├─ 预制词典.dict
   │  └─ 预制词典.dict.origin           ← 来源边车（纯展示，不带任何限制）
   └─ tables\                           ← 词频表库：一张表就是一个产物目录
      ├─ 预制表\
      │  ├─ meta.json
      │  ├─ news\word.vfr   news\char.vfr          ← 每个表组一个平级目录
      │  ├─ wiki\…  blog\…  book\…  forum\…  gov\…
      │  ├─ full\word.vfr   full\char.vfr          ← 全部表组相加出来的那一张，同样平级
      │  └─ 相加：新闻与维基\word.vfr               ← 相加出来的新表，同样平级
      └─ 预制表.origin
```

词频表目录内部是 **schema v3 的布局**：每个**表组**各占一个平级目录，里面是
`word.vfr` + `char.vfr`。表组就是语料库的一级子目录名；`full`（全部表组加在一起）
与 `相加：新闻与维基` 这样的合成表都只是**又一个表组**，没有任何特权。

> **`scan` 默认不产出 `full`。** 它只写各表组自己的表，`full` 一律由
> 「合流多份产物 + 把表组相加」得到 —— 为什么、怎么做见 §七。
> 只有显式 `--full`（或 `write_full`）才会在扫描时顺手算一遍全量，那是**验证基准**。

v2 及以前是 `full/` 与 `domains/` 两棵树、把表组与类型糊在 `full/word` 这样的路径里
—— 那种老产物现在**会被明确拒绝**并提示重新统计（无法就地迁移）。细节见
[`DESIGN.md`](DESIGN.md) §5.0。

三个位置选择是有理由的，不是随手定的：

- **`%LOCALAPPDATA%` 而不是 `%APPDATA%`。** 后者是 Roaming，域环境里会被漫游配置同步走，
  上百 MB 的数据进去是灾难。Tauri 的 `app_local_data_dir()` 给的就是前者。
- **再套一层 `data\`。** `%LOCALAPPDATA%\com.voctier.desktop` 同时是 **WebView2 的默认
  数据目录**（tauri 文档原话）。资产直接摊在根上，用户点"打开数据文件夹"会看到
  `EBWebView` 那一堆缓存，以为程序乱写文件。
- **词频表不需要可写。** 程序对 `.vfr` 只做只读 mmap，从不往里写。需要可写权限的只有
  `dicts\` —— 那也正是用户要改的东西。

### 预置内容怎么进来

安装包**只出 NSIS**，由安装钩子
[`apps/desktop/src-tauri/nsis/installer-hooks.nsh`](../apps/desktop/src-tauri/nsis/installer-hooks.nsh)
在 `NSIS_HOOK_POSTINSTALL` 里用 `File /r` 把预置词典与预置词频表**直接写进**数据文件夹：

```
$LOCALAPPDATA\<identifier>\data\dicts\      ← 预置词典
$LOCALAPPDATA\<identifier>\data\tables\     ← 预置词频表
```

**刻意不走 `bundle.resources`** —— 那条路的语义就是"释放到 `$INSTDIR`"，而
`Program Files` 下普通用户不可写，用户也就删不掉那份预置内容。而预置内容必须能删。

钩子里有三个坑，改动时别踩（前两个已用 Tauri 自带的那份 NSIS 3.11 实测确认）：

1. `.nsh` 是被 `installer.nsi` 在**顶层** `!include` 的，所以顶层只能放 `!define` / `!macro`；
   `File` 这类指令必须在钩子宏体内（宏体在 `Section` 里展开）。
2. **`${__FILEDIR__}` 在宏体里指向 `installer.nsi` 的目录**（`target\<profile>\nsis\<arch>`，
   每次构建还会被清空），而不是 `.nsh` 自己的目录。所以资产根路径必须在 `.nsh`
   **顶层**固化成普通 `!define`（`VOCTIER_SEED_ROOT` 就是这么来的）。
3. `installMode: perMachine` 时 `$LOCALAPPDATA` 会指到 `ProgramData`，
   钩子里必须 `SetShellVarContext current`。

用 `POSTINSTALL` 而不是 `PREINSTALL`：模板里 `PREINSTALL` 跑在 `CheckIfAppIsRunning`
**之前**，应用还在运行时就往数据目录写文件，可能撞上正被 mmap 的 `.vfr`。

### 验证到哪一步了

诚实地区分一下，别把"编译过了"当成"装过了"：

| 项目 | 状态 |
|---|---|
| 钩子能被**真实的** Tauri 打包流程接受、并产出安装包 | ✅ 已验证（`VocTier_0.1.0_x64-setup.exe` = 44.76 MB；生成的 `installer.nsi` 里 L32 是 `!include "...\installer-hooks.nsh"`、L705 是 `!ifmacrodef NSIS_HOOK_POSTINSTALL`） |
| 钩子里的宏最终展开成正确路径 | ✅ 已验证（用 `makensis /PPO` 预处理复现真实定义顺序，输出是 `SetOutPath "$LOCALAPPDATA\com.voctier.desktop\data\dicts"`；**0 处**字面量 `${BUNDLEID}`） |
| `$OUTDIR` 在钩子结束时还原 | ✅ 同上，PPO 输出末尾有 `SetOutPath "$INSTDIR"` |
| 预置内容进安装包后的体积 | ✅ 实测 96.6 MB → 安装包 44.76 MB（LZMA，含主程序 11.52 MB） |
| `%LOCALAPPDATA%\com.voctier.desktop\data\` 与 Rust 侧 `app_local_data_dir()` 一致 | ✅ 已核对源码 |
| `prepare-seed.ps1 -IntoDataFolder` 铺出来的树 | ✅ 已验证（与安装器应当产生的完全一致，CLI 对它能正常查词） |
| **真的装一遍、检查落地目录树** | ❌ **没验成** —— 本开发环境里 NSIS 安装器执行不起来：一个只往 `%TEMP%` 写标记文件的最小安装器同样什么都不产出。这是环境限制，不是配置问题 |

> 为什么 `.nsh` 里可以用顶层 `!define` 去拼 `${BUNDLEID}`，即使模板要到几十行之后才定义它？
> 因为 `!define` 只是把**文本**存下来，真正解析发生在 `!insertmacro` 那一刻 ——
> 那时 `${BUNDLEID}` 早就有了。NSIS 会在插入后的文本上继续做定义展开，所以两级
> （`${VOCTIER_DATA_DIR}` → `${BUNDLEID}`）也能一次穿到底。这一点是用 `PPO` 证实的，
> 不是推理出来的。

所以**发布前请在一台干净的机器上装一遍**，确认
`%LOCALAPPDATA%\com.voctier.desktop\data\` 下出现 `dicts\` 与 `tables\` 且能查词。
这是唯一还没被证据覆盖的一环。

## 三、三条铁律

### 1. 预置的和用户自建的**没有本质区别**

安装包只是"帮你放了两个进去"。它们和用户自己新建的完全是同一种东西：同样能删、
能改名、能替换、能被删掉之后自己做一个。所以代码里**没有 `builtin` 之类的权限位**，
只有一个纯展示用的来源标记（`.origin` 边车：`seeded` / `imported` / `scanned` / `unknown`）。

**表组之间也没有等级。** 预置表里带的是 `full` 加各表组**全部**表组，装完之后用户
可以在「表管理」页任选一个当主词频表，也可以把几个表组**相加**成一张新表（相加出来的表
存在同一个产物目录里，是又一个平级表组，来源记在 `meta.json` 的 `source_tables` 里）。
`full` 只是"所有表组加在一起"的那一个，没有任何特权。

来源标记存在**同名边车文件**里，而不是塞进词典文件本身：塞进去就要在词典里加一行 `#`
注释，用户手改一次就可能弄丢；而且"来源"是安装包与用户操作的属性，不是词典内容的属性 ——
内容改了指纹就该变，来源却不该因为改了一个字就变成 `imported`。

### 2. **永不覆盖**

安装包里带的预置内容覆盖同名文件；应用内的导入遇到重名会**自动加后缀**
（`我的.dict` → `我的 (2).dict`）。这条规则同时也是"以后发更全的词典"的答案：
新版本自带的词典不会盖掉用户的东西，它作为**另一个条目**出现，用户自己决定切过去还是删掉旧的。

### 3. 表与词典用**内容指纹**绑定

`meta.json` 的 `tokenizer.dicts[]` 记的是 `{id, name, path, entries, sha256}`，而**校验只认
`sha256`** —— 路径换台机器就失效，指纹不会。匹配顺序是「指纹 → 名字」：

| 情况 | 判定 | 界面该怎么表现 |
|---|---|---|
| 指纹对得上 | `ok` | 绿色「词典一致」 |
| 名字对得上、指纹不同 | `drifted` | 黄色「词典已变：X，频次可能不准」+ 一键重算 |
| 名字和指纹都找不到 | `missing` | 红色「找不到词典：X」+ 一键重算 |
| 记录里没有任何指纹 | `legacy` | 中性「老产物，无词典记录，无从校验」+ 建议重算 |

⚠ 最后一行的判据是**"有没有指纹"而不是"记录列表空不空"**。v1 老产物的
`"dict": "builtin(jieba dict.txt, 349046 entries)"` 反序列化出来正好是一条没指纹的记录，
列表非空却什么也说明不了。判错的话每一份老产物升级后都会顶着一条红色"词典缺失"告警，
直接训练用户忽略告警。这个坑有测试钉住。

打开一张表时如果词典对不上，应用**不会静默照用**：它退化到用数据文件夹里现有的词典，
但同时把告警记下来，交给界面显示、并写进 `startup.log`。

## 四、词典文件格式

就是 jieba 的格式，只有三条规则：

```
# 以 # 开头的整行是注释
中国 100 n
人工智能 1000000
直播带货
```

- `词 词频 词性`，后两列可省略。**词频是分词用的概率权重，不是词频统计** ——
  稀有度一律由语料统计得出，别拿它当频次用。
- 以 `#` 开头的整行是注释，空行跳过。
  > 代价：**以 `#` 开头的词无法在词典里表达**（比如微博话题词 `#话题`）。
  > 这是为了让自己产出的 `oov_candidates.tsv`（开头有 10 行 `#` 说明）和用户手写的注释
  > 都能直接用，属于刻意取舍，有测试钉住。
- **省略词频 ≠ 词频 0。**
  - 省略 → 按 jieba 的 `suggest_freq` 折算，词能正常切出来。
  - **显式写 `0`** → 词会被登记进词典（`has_word` 为真、产物里 `in_dict` 标记也是真）
    但路径概率是 `ln(0) = -inf`，**永远不可能被切分出来**。几乎总是笔误，
    扫描时和界面上都会警告。
- 词频列写了非整数、或一行超过 3 列（多半是词里误带了空格）→ **报错并指出行号**。
  jieba 自己会静默忽略多余列，用户会以为词加进去了，其实没有。

### 为什么不用 jieba 的 `load_dict` 直接读文件

因为它在两个地方会坑人，都由 `vocfreq_core::dict` 兜住：

1. **它不认注释。** `# VocTier 词典外高频条目` 这行会被当成词条解析，第二列 `VocTier`
   不是整数 → `InvalidDictEntry` 直接失败。而这份文件正是本工具**自己产出的**、
   文档还明确让用户拿去回填词典的。旧版本在这条路径上是必然失败的（实测确认）。
   如果注释的第二列恰好是数字，则会更糟：`#` 被当成一个词静默插进词典。
2. **它会留下半装载状态。** `load_dict` 一进门就把 `total` 归零，中途出错又不走
   `finish_load()`，分词器会停在 `total` 与 `log_total` 打架、部分词已进 trie 的中间态。
   所以词典一律"先整份读进内存、逐行校验，确认无误再喂进去"。

## 五、开发与发布

### 组装预置内容

```powershell
.\tools\prepare-seed.ps1                  # 组装到 apps\desktop\src-tauri\seed\
.\tools\prepare-seed.ps1 -RefreshDict     # 顺便从 cargo 注册表刷新词典原件
.\tools\prepare-seed.ps1 -IntoDataFolder  # 再铺一份到数据文件夹（不装安装包也能开发）
```

产物：

| 路径 | 内容 | 大小 |
|---|---|---|
| `assets\seed\dicts\预制词典.dict` | 词典**原件**，随仓库携带（MIT） | 4.84 MB |
| `apps\desktop\src-tauri\seed\` | 安装包实际携带的内容，**不入库** | 约 97 MB |

词典原件入库是**故意的**：否则全新 clone 出来的仓库没法构建出可用的安装包。
预置词频表约 92 MB，太大，所以从仓库里的 `data\` 现摘。

> 预置词频表现在带的是**全部表组**（各一级子目录，外加相加出来的 `full`）。各表组表
> 加起来与 `full` 是同一批 token，所以体积与"只带一张全量表"基本同量级；换来的是用户
> 装完就能按表组对比、也能自己把几个表组相加成主表。相加本身在应用里做（`compose_tables`），
> 不占安装包。

> ⚠ 组装 `meta.json` 那一步走的是 `vocfreq prepare-seed`（Rust 侧），**不是脚本拼 JSON**。
> 因为 `meta.json` 里 `tier_stats[].max_rank` 用的是 `u64::MAX`
> （18446744073709551615），PowerShell 的 `ConvertFrom-Json` 会把它读成 Double、
> 回写成 `1.8446744073709552E+19`，之后 Rust 侧根本反序列化不回来。

`prepare-seed` 还会做出厂前必须做的三件事：把词典链钉成一份并写入指纹、**把全部表组
一起带上**（不只 `full` —— 只带一张全量表等于把"按表组对比"和"自己相加成主表"这两条路
堵死了）、把 `corpus_root` 里的绝对路径换成中性描述（`build-desktop.ps1` 花了力气抹掉
二进制里的构建机路径，出厂产物里再塞一个绝对路径就白做了）。

### 构建

```powershell
.\tools\prepare-seed.ps1
.\tools\build-desktop.ps1 -Bundle      # 只出 NSIS
```

## 六、已知代价与限制

- **安装包变大**：预置词典 4.84 MB + 预置词频表约 92 MB，压缩后安装包约 +60 MB。
  作为交换，exe 小了约 1.8 MB（zstd 词典那部分），CLI 从 3.86 MB 降到约 2 MB。
  **这不是瘦身方案，是定制方案。**
- **CLI 不再自包含**：`vocfreq.exe` 从哪儿拷都能跑的日子结束了，现在必须有词典，
  或者用 `--dict` / `--dict-dir` 指路。
- **删词做不到**。jieba-rs 没有删除 API，只有 `add_word` / `load_dict` 覆盖词频。
  所以"自定义"能覆盖加词和调权重，**删不掉内置词**。要删词只能整份另做一份词典，
  或者在 `tokenize.rs` 的 `accept()` 那一层加用户停用词表（尚未实现）。
- **升级会覆盖预置内容**。NSIS 的 `File` 只覆盖同名文件、不删多余文件。如果用户在
  `预制词典.dict` 上直接改过，升级安装会把它冲掉 —— 想保住自己的改动，就另存一份。
  同理，用户在预置表里**相加出来的新表组目录**不会被升级删掉（`File` 不做删除），
  但覆盖同名文件时可能让它引用的表发生变化（`.origin` 上的版本提示会随之变）。
- **相加会占一份磁盘**。`compose` 是把结果**物化**成真实 `.vfr`（不是只记一条
  "A+B+C"的标记），代价大约是一张扫描出来的表那么大。换来的是排行榜翻页与覆盖率曲线
  仍然 O(1) / 顺序读，不需要为虚拟合成再造一套索引。源表不需要了可以删掉回收，
  新表本身也可以再被相加。**相加要求所有源表的词典链与分词口径一致**（见 §七）：
  不一致会直接报错拒绝，不会静默合并。
- **卸载默认保留用户数据**。安装器的"删除应用数据"复选框默认不勾选；勾了会整个删掉
  `%LOCALAPPDATA%\com.voctier.desktop`（连同 WebView2 缓存）。
- **MSI 未做**。只出 NSIS。要走 MSI 得另做一遍"装到用户目录"的方案。

## 七、按表组扫描 → 合流 → 相加出 full

全量语料可能大到**一块盘放不下同时在场**（实测 140 GB）。这种时候只能**一个表组一个表组**
地处理，于是产生一个以前不成立的需求：把**分几次扫出来的几份产物并成一份**。

```powershell
# 1) 一个表组一份产物 —— scan 默认不写 full，所以每份产物只有自己那个表组
vocfreq scan --corpus D:\语料 --out D:\分片\A --dict 词典.dict --domains news
vocfreq scan --corpus D:\语料 --out D:\分片\B --dict 词典.dict --domains wiki
vocfreq scan --corpus D:\语料 --out D:\分片\C --dict 词典.dict --domains book

# 2) 合流成一份标准产物（表组各自平级，谁也不动谁的频次）
vocfreq merge --from-data D:\分片\A --from-data D:\分片\B --from-data D:\分片\C `
              --scope 主表 --out D:\词频表

# 3) 把各表组相加出 full —— 这一步就是普通的 compose，在合流产物内部做
vocfreq compose --from-data D:\词频表\主表 `
                --source news/word --source wiki/word --source book/word `
                --source news/char --source wiki/char --source book/char `
                --scope full
```

最后得到的 `D:\词频表\主表\` 是一份**标准 v3 单产物**（`meta.json` + 平级表组目录），
桌面端「表管理」直接就能打开它，**不需要任何改动**。

### 为什么 `scan` 默认不写 `full`

因为上面那条工作流里它是纯浪费：每份分片都会算一遍全库，写几百 MB，而且**下一次
`scan` 会整体覆盖 `meta.json` 与 `full/`** —— 前几次写的全被冲掉，`meta.tables` 里
只剩最后一次的记录。`full` 交给第 3 步的相加来产出，数学上与"一次性全量扫描"**逐条
相等**（推导见 `compose.rs` 顶部的注释；`vocfreq-cli/src/tests.rs` 里两条端到端测试
分别对着"各表组累加"和"一次全量扫描的产物"逐条比对）。

需要一份显式的全量对照基准时用 `--full`：

```powershell
vocfreq scan --corpus D:\语料 --out D:\基准 --dict 词典.dict --full   # 只为验证
```

### 合流的硬门槛：词典链与分词口径必须完全一致

`merge` **逐份校验** `meta.json` 里的 `tokenizer`，任何一处不同都**报错拒绝**，并指出
是哪一份产物、哪个字段不同：

- 词典链：**逐份比 `sha256`**（指纹是唯一可靠判据 —— 路径换台机器就失效，名字是用户
  随手起的）。链的长度不同、某一环内容不同，都拒绝。
- 分词口径：`hmm` / `min_len` / `max_len` / `keep_latin` / `keep_digit` /
  `skip_single_char`，外加 `engine` / `version`。
- **没有指纹的产物**（schema v1 老产物）一律拒绝：两份都"没有记录"看着一致，其实什么
  也没说明，而"是不是同一份词典"正是合流正确性的全部依据。

不合并词典链、不挑一份当基准、不产出半截产物 —— 目标目录要么不存在，要么是完整的。
（实现上先铺到同级的临时目录、最后整体改名，任何一步失败都清掉临时目录。）

### 不被当成门槛的东西

合流**不**要求这些一致，因为它们不影响正确性：

| 字段 | 为什么不比 |
|---|---|
| `min_count` | 每张表自己记着；相加时取各源的最大值，排名才与全量重扫一致 |
| `tiers` / `tier_stats` / `tier_pct` | 各表条目数不同，本来就该不同 |
| `tier_names` / `tier_keys` | 展示名，由本程序的常量决定，新产物必然一致 |
| `corpus_root` / `generated_at` | 合流本来就是把几块语料并起来；各源不同就换成一句中性描述 |
| `domains`（语料切片清单） | 描述的是**语料**不是表，合流时按名字并起来、计数相加 |

> 前置约束：**语料文件必须放在语料根的一级子目录里**，表组 = 一级目录名
> （`plan_corpus` 的既有约定）。所以"一个表组一份产物"靠 `--domains <表组>` 切，
> 而不是靠把语料挪成不同的根。

