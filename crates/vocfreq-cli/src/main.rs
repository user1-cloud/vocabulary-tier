//! VocTier 语料库字词频率统计工具（纯 Rust，不依赖 Tauri）。

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use vocfreq_core::artifact;
use vocfreq_core::dict;
use vocfreq_core::query::{Dataset, VfrTable};
use vocfreq_core::scan::{self, Progress, ScanConfig};
use vocfreq_core::source::{self, SourceRule};
use vocfreq_core::tokenize::{TokenizeOpts, Tokenizer};

#[derive(Parser)]
#[command(
    name = "vocfreq",
    version,
    about = "VocTier 字词频率统计：从大规模语料库产出字词排行榜与可 mmap 查询的索引",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Copy, Clone, ValueEnum)]
enum ProgressMode {
    /// 逐行输出 JSON 事件（供 Tauri 子进程解析）
    Json,
    /// 人类可读的进度条
    Bar,
    /// 静默
    None,
}

#[derive(Subcommand)]
enum Cmd {
    /// 只探测语料库各文件的 JSON 格式，不做统计
    Detect {
        /// 语料库根目录
        #[arg(long)]
        corpus: PathBuf,
        /// 自定义规则 JSON 文件
        #[arg(long)]
        rules: Option<PathBuf>,
    },

    /// 统计并产出频率表（**默认只产各作用域自己的表，不含 `full`**）
    ///
    /// `full` 由「`merge` 合流多份产物 + `compose` 把作用域相加」得到：分域分次扫描时
    /// 每次 scan 都写一遍 full 纯属浪费（而且会冲掉前几次的结果，见 docs/DATA_LAYOUT.md §七）。
    /// 需要一份全量对照基准就加 `--full`。
    Scan(Box<ScanArgs>),

    /// 查看 .vfr 产物的头信息与样例
    Info {
        /// .vfr 文件路径
        #[arg(long)]
        table: PathBuf,
    },

    /// 查询词的频次与排名
    Lookup {
        /// .vfr 文件路径
        #[arg(long)]
        table: PathBuf,
        /// 要查的词
        words: Vec<String>,
        /// 顺便打印前 N 名
        #[arg(long, default_value_t = 0)]
        top: u32,
    },

    /// 分词并显示每个词的频率信息（前端的划句分析走同一套核心逻辑）
    Segment {
        /// vocfreq scan 的产物目录
        #[arg(long)]
        data: PathBuf,
        /// 要分析的文本；省略则从标准输入读取
        text: Vec<String>,
        /// 输出 JSON
        #[arg(long)]
        json: bool,
        /// 开启 HMM 新词发现（须与建表时一致）
        #[arg(long)]
        hmm: bool,
        /// 词库来源。不给则用建表时 `meta.json` 里记录的那条链
        #[command(flatten)]
        dicts: DictArgs,
        /// 用 ANSI 真彩色显示分组配色
        #[arg(long)]
        color: bool,
        /// 只看这些分域的排名
        #[arg(long, value_delimiter = ',')]
        domains: Vec<String>,
    },

    /// 从一份已有的产物目录里摘出「预置词表」，供安装包随包携带。
    ///
    /// 为什么要用工具来做这件事、而不是脚本拼 JSON：`meta.json` 里
    /// `tier_stats[].max_rank` 用的是 `u64::MAX`（18446744073709551615），
    /// PowerShell 的 `ConvertFrom-Json` 会把它读成 Double、再回写成
    /// `1.8446744073709552E+19`，之后 Rust 侧根本反序列化不回来。
    PrepareSeed(Box<PrepareSeedArgs>),

    /// 把若干张表**相加**成一张新表（各作用域平等，全量表只是"全部相加"的结果）
    Compose(Box<ComposeArgs>),

    /// 把**若干份产物合流**成一份（各源产物的词库链与分词口径必须完全一致）
    Merge(Box<MergeArgs>),

    /// 前%换算：前%上界 ↔ 排名阈值（默认分组口径就是前%）
    Pct {
        /// vocfreq scan 的产物目录
        #[arg(long)]
        data: PathBuf,
        /// 表身份：`作用域/类型`，例如 full/word、news/char
        #[arg(long, default_value = "full/word")]
        table: String,
        /// 要换算的前%上界（百分数）。不给就用该类型的默认口径
        #[arg(long, value_delimiter = ',')]
        pcts: Vec<f64>,
        /// 反过来算：给若干排名，打印它们各自的前%
        #[arg(long, value_delimiter = ',')]
        ranks: Vec<u32>,
    },

    /// 从已生成的词表里重新导出「词典外候选词」，无需重跑统计
    Oov {
        /// vocfreq scan 的产物目录
        #[arg(long)]
        data: PathBuf,
        /// 用哪个作用域的词表（要该作用域写出过可读 TSV）
        #[arg(long, default_value = "full")]
        scope: String,
        /// 最小频次
        #[arg(long, default_value_t = 500)]
        min_count: u64,
        /// 最小词长（按字符数）。默认 1：关闭 HMM 时多字中文候选必然为空
        #[arg(long, default_value_t = 1)]
        min_len: usize,
        /// 只保留含汉字的候选（默认开启；用 --no-cjk-only 关掉可以看到 URL 碎片）
        #[arg(long)]
        no_cjk_only: bool,
        /// 最多导出多少条
        #[arg(long, default_value_t = 200_000)]
        limit: usize,
    },

    /// 查看某张表的累计覆盖率曲线，并把覆盖率目标换算成排名阈值
    ///
    /// 用于「按覆盖率分组」：先看曲线，再挑七个目标覆盖率，命令会告诉你对应的排名上界。
    Curve {
        /// vocfreq scan 的产物目录
        #[arg(long)]
        data: PathBuf,
        /// 表路径：full/word、full/char、domains/<域>/word、domains/<域>/char
        #[arg(long, default_value = "full/word")]
        table: String,
        /// 曲线采样点数
        #[arg(long, default_value_t = 600)]
        points: usize,
        /// 要换算的六个覆盖率目标（百分数）
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "26.7,55.6,78.3,90.8,95.8,98.8"
        )]
        targets: Vec<f64>,
    },
}

/// 词库来源的三个选项。`scan` 与 `segment` 共用，所以抽出来一份 ——
/// 两处各抄一遍迟早会不同步。
#[derive(clap::Args, Clone)]
struct DictArgs {
    /// 指定词库（jieba 格式：词 词频 词性，词频与词性可省略）。可重复，
    /// **第一份是主词库**，其余依次叠加
    #[arg(long = "dict", value_name = "FILE")]
    dict: Vec<PathBuf>,
    /// 从目录里取所有 .dict 组成词库链（按文件名排序），排在 --dict 之后
    #[arg(long, value_name = "DIR")]
    dict_dir: Option<PathBuf>,
    /// 追加叠加词库（可重复），排在最后。里面的词会被标记成「来自用户词典」
    #[arg(long, value_name = "FILE")]
    user_dict: Vec<PathBuf>,
}

#[derive(clap::Args)]
struct ScanArgs {
    /// 语料库根目录
    #[arg(long)]
    corpus: PathBuf,
    /// 产物输出目录
    #[arg(long)]
    out: PathBuf,
    /// 线程数；0 表示按可用核心数
    #[arg(long, default_value_t = 0)]
    threads: usize,
    /// 开启 HMM 新词发现（默认关闭，以保证结果可复现）
    #[arg(long)]
    hmm: bool,
    /// 词库来源
    #[command(flatten)]
    dicts: DictArgs,
    /// 自定义语料解析规则 JSON
    #[arg(long)]
    rules: Option<PathBuf>,
    /// 只统计这些域（逗号分隔）；默认全部
    #[arg(long, value_delimiter = ',')]
    domains: Vec<String>,
    /// 不产出各分域的子表。要和 `--full` 一起用，否则产物里一张表都没有
    #[arg(long)]
    no_domains: bool,
    /// **顺带产出全量作用域 `full`**。
    ///
    /// 默认不产：`full` 一律由「`vocfreq merge` 合流分域产物 + `vocfreq compose` 把
    /// 各作用域相加」得到（见 docs/DATA_LAYOUT.md §七）。这个开关只有两个用途：
    /// 一次性全量扫一遍留一份**对照基准**，以及只想要一张全量表时。
    #[arg(long)]
    full: bool,
    /// 只保留出现次数 >= N 的词条
    #[arg(long, default_value_t = 1)]
    min_count: u64,
    /// 是否把纯数字 token 也计入（默认丢弃，保持词表干净）
    #[arg(long)]
    keep_digit: bool,
    /// 丢弃纯英文 token
    #[arg(long)]
    no_latin: bool,
    /// 丢弃单字词（单字另有字表承载）
    #[arg(long)]
    skip_single_char: bool,
    /// 不写可读 TSV（只要二进制索引）
    #[arg(long)]
    no_tsv: bool,
    /// 词典外候选词的最小频次
    #[arg(long, default_value_t = 500)]
    oov_min_count: u64,
    /// 进度输出方式
    #[arg(long, value_enum, default_value_t = ProgressMode::Bar)]
    progress: ProgressMode,
}

/// `compose` 的参数。
#[derive(clap::Args)]
struct ComposeArgs {
    /// 源产物目录。可给多个：**源表可以跨产物**，但各产物的词库链与分词口径
    /// 必须完全一致，否则直接报错拒绝（这是相加正确性的硬门槛）。
    #[arg(
        long = "from-data",
        value_name = "DIR",
        required = true,
        value_delimiter = ','
    )]
    from_data: Vec<PathBuf>,
    /// 要相加的表，`作用域/类型` 形式。可重复
    #[arg(long = "source", value_name = "[SCOPE/]KIND", required = true)]
    sources: Vec<String>,
    /// 新作用域的名字（会变成目录名）
    #[arg(long)]
    scope: String,
    /// 只要这一类（`word` 或 `char`）；不给 = 源表里出现过的每一类都相加
    #[arg(long)]
    kind: Option<String>,
    /// 输出目录；不给 = 写回第一个 `--from-data`（新作用域成为同一份产物里的另一张表）
    #[arg(long)]
    out: Option<PathBuf>,
}

/// `merge` 的参数。
#[derive(clap::Args)]
struct MergeArgs {
    /// 源产物目录。至少两份 —— 每份是一次独立 `scan` 的产物（通常只有自己的那个域）
    #[arg(
        long = "from-data",
        value_name = "DIR",
        required = true,
        value_delimiter = ','
    )]
    from_data: Vec<PathBuf>,
    /// 目标父目录
    #[arg(long)]
    out: PathBuf,
    /// 目标产物目录名；不给 = 直接把 `--out` 当产物目录
    #[arg(long)]
    scope: Option<String>,
}

/// `prepare-seed` 的参数。
#[derive(clap::Args)]
struct PrepareSeedArgs {
    /// 源产物目录（要有 meta.json 与各作用域的 *.vfr）
    #[arg(long)]
    from_data: PathBuf,
    /// 预置词表要绑定到的那份词库文件。它的指纹与词条数会写进产物的 meta.json
    #[arg(long)]
    dict: PathBuf,
    /// 输出目录：会写入 meta.json 与各作用域的表
    #[arg(long)]
    out: PathBuf,
    /// 出厂时展示的语料库描述。
    ///
    /// 不给就自动生成一句不含绝对路径的说法。**绝不要把构建机上的真实路径写进去**：
    /// tools/build-desktop.ps1 花了力气把构建机路径从二进制里抹掉，出厂产物里再塞一个
    /// `E:\某个人的目录\语料库` 就全白做了。
    #[arg(long)]
    corpus_label: Option<String>,
    /// 表名，只用于打印
    #[arg(long, default_value = "预制表")]
    table_name: String,
}

/// 出厂用的语料库描述：只说"是什么样的语料"，不带任何绝对路径。
fn neutral_corpus_label(meta: &artifact::Meta) -> String {
    format!(
        "预置语料库：{} 个域 / {} 个文件 / {:.1} GiB",
        meta.domains.len(),
        meta.totals.files,
        meta.totals.bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    )
}

/// 从一份已有的产物目录里摘出「预置词表」。
///
/// 做四件出厂前必须做的事：
/// 1. **把词库链钉成一份**并写入它的指纹 —— 这样用户在数据文件夹里换了词库之后，
///    界面能立刻告诉他"这张预置表对应的词库已经变了"。
/// 2. **把全部作用域一起带上**（不只 `full`）。装完之后用户在表管理页就能按分域
///    对比，也能自己把几个域相加成主表 —— 只带一张全量表等于把这条路堵死了。
/// 3. **抹掉语料库的绝对路径**，换成一句中性的描述。
/// 4. 校验一遍：每个作用域都得有词表，缺了就地报错，而不是留一份半截的预置内容
///    让用户装完才发现。
fn cmd_prepare_seed(a: PrepareSeedArgs) -> Result<()> {
    let meta_path = a.from_data.join("meta.json");
    let bytes =
        std::fs::read(&meta_path).with_context(|| format!("读取 {} 失败", meta_path.display()))?;
    let mut meta: artifact::Meta = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} 不是合法的 meta.json", meta_path.display()))?;

    let rd =
        dict::read_dict(&a.dict).with_context(|| format!("读取词库 {} 失败", a.dict.display()))?;
    if rd.report.freq_zero > 0 {
        eprintln!(
            "[warn] 词库 {} 里有 {} 条把词频显式写成了 0，这些词永远切不出来",
            rd.dict.name, rd.report.freq_zero
        );
    }

    // 1) 词库链只留这一份，并清掉 v1 的两个兼容字段（新产物不该再写它们）
    //
    //    `path` 刻意清空：出厂产物里塞一个构建机的绝对路径，既把目录结构泄了出去
    //    （build-desktop.ps1 花了力气把这类路径从 exe 里抹掉），又毫无用处 ——
    //    装到用户机器上那个路径根本不存在。校验只认 sha256，找回词库则靠
    //    「数据文件夹的 tables\ 与 dicts\ 同级」这条布局约定（见 CLI 的
    //    `chain_from_meta` 与桌面端的 `library::resolve_chain`）。
    let mut seed_ref = rd.dict.clone();
    seed_ref.path = String::new();
    meta.tokenizer.dicts = vec![seed_ref];
    meta.tokenizer.legacy_dict = None;
    meta.tokenizer.legacy_user_dict = None;

    // 2) 语料库描述去路径化
    meta.corpus_root = a
        .corpus_label
        .clone()
        .unwrap_or_else(|| neutral_corpus_label(&meta));
    meta.schema_version = artifact::SCHEMA_VERSION;

    // 3) 全部作用域都带上。至少要有一张词表，否则这份预置内容查不了词。
    if meta.tables.iter().all(|t| t.kind != "word") {
        bail!(
            "{} 里一张词表都没有，预置内容必须至少带一张 word 表",
            meta_path.display()
        );
    }

    // 4) 写 meta.json + 拷各作用域的表
    std::fs::create_dir_all(&a.out)?;
    artifact::write_meta(&a.out.join("meta.json"), &meta)?;

    let mut copied = 0u64;
    let mut scopes: Vec<String> = meta.scopes();
    scopes.sort();
    for scope in &scopes {
        for kind in ["word", "char"] {
            let t = match meta.table(scope, kind) {
                Some(t) => t,
                None => continue,
            };
            let src_dir = a.from_data.join(scope);
            let dst_dir = a.out.join(scope);
            std::fs::create_dir_all(&dst_dir)?;
            for ext in ["vfr", "tsv"] {
                let src = src_dir.join(format!("{kind}.{ext}"));
                if src.exists() {
                    std::fs::copy(&src, dst_dir.join(format!("{kind}.{ext}")))
                        .with_context(|| format!("拷贝 {} 失败", src.display()))?;
                }
            }
            let vfr = dst_dir.join(format!("{kind}.vfr"));
            if !vfr.exists() {
                bail!(
                    "{} 在 meta.json 里登记了，但磁盘上找不到 {} —— 这份产物是半截的，\
                     不能拿来做出厂预置内容",
                    t.key(),
                    a.from_data
                        .join(scope)
                        .join(format!("{kind}.vfr"))
                        .display()
                );
            }
            copied += t.vfr_bytes;
        }
    }

    println!("✅ 预置词表「{}」已生成", a.table_name);
    println!("   输出：{}", a.out.display());
    println!("   词库：{}", rd.dict.short());
    println!("   作用域 {} 个：{}", scopes.len(), scopes.join(" / "));
    for t in &meta.tables {
        println!(
            "   表 {}：{} 条 / {} token",
            t.key(),
            t.entries,
            t.total_tokens
        );
    }
    println!("   .vfr 合计 {:.1} MB", copied as f64 / 1e6);
    println!("   语料库描述：{}", meta.corpus_root);
    println!(
        "\n接下来把输出目录放进安装包的 seed\\tables\\<表名>\\ 下（见 tools\\prepare-seed.ps1）。\n\
         装完之后用户可以在「表管理」页任选一个作用域当主表，也可以把几个域相加成一张新表。"
    );
    Ok(())
}

fn load_rules(path: &Option<PathBuf>) -> Result<Vec<SourceRule>> {
    match path {
        None => Ok(source::builtin_rules()),
        Some(p) => {
            let bytes =
                std::fs::read(p).with_context(|| format!("读取规则文件 {} 失败", p.display()))?;
            let rules: Vec<SourceRule> =
                serde_json::from_slice(&bytes).context("规则文件不是合法的 JSON 规则数组")?;
            Ok(rules)
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Detect { corpus, rules } => cmd_detect(corpus, rules),
        Cmd::Scan(a) => cmd_scan(*a),
        Cmd::Info { table } => cmd_info(table),
        Cmd::Lookup { table, words, top } => cmd_lookup(table, words, top),
        Cmd::Segment {
            data,
            text,
            json,
            hmm,
            dicts,
            color,
            domains,
        } => cmd_segment(data, text, json, hmm, dicts, color, domains),
        Cmd::PrepareSeed(a) => cmd_prepare_seed(*a),
        Cmd::Compose(a) => cmd_compose(*a),
        Cmd::Merge(a) => cmd_merge(*a),
        Cmd::Pct {
            data,
            table,
            pcts,
            ranks,
        } => cmd_pct(data, table, pcts, ranks),
        Cmd::Oov {
            data,
            scope,
            min_count,
            min_len,
            no_cjk_only,
            limit,
        } => cmd_oov(data, min_count, min_len, !no_cjk_only, limit, &scope),
        Cmd::Curve {
            data,
            table,
            points,
            targets,
        } => cmd_curve(data, table, points, targets),
    }
}

// ---------------------------------------------------------------- curve

/// 打印累计覆盖率曲线，并把覆盖率目标反解成排名阈值。
///
/// 这是「按覆盖率分组」的换算工具：前端 `tier_curve` 命令与这里的
/// [`vocfreq_core::query::rank_for_coverage`] 用的是同一套逻辑。
///
/// ⚠ 覆盖率**不是**默认口径。默认是「前%」（`排名 ÷ 条目数`），见 [`Cmd::Pct`]。
fn cmd_curve(data: PathBuf, table: String, points: usize, targets: Vec<f64>) -> Result<()> {
    use vocfreq_core::query::{KIND_CHAR, rank_for_coverage};

    let ds =
        Dataset::open(&data).with_context(|| format!("打开产物目录 {} 失败", data.display()))?;
    let t = ds.table_by_key(&table).ok_or_else(|| {
        anyhow::anyhow!(
            "找不到表 {table}；可用取值是「作用域名/类型」，例如 full/word、news/char。\n\
             本产物里有：{}",
            ds.tables
                .iter()
                .map(|x| x.key())
                .collect::<Vec<_>>()
                .join("、")
        )
    })?;
    let h = t.vfr.header();

    let t0 = std::time::Instant::now();
    let curve = t.vfr.coverage_curve(points);
    let elapsed = t0.elapsed();
    if curve.is_empty() {
        bail!("{table} 是空表，没有曲线");
    }

    println!(
        "表 {}  kind={}  {} 条  {} token",
        table,
        if h.kind == KIND_CHAR { "char" } else { "word" },
        h.entry_count,
        h.total_tokens
    );
    println!(
        "曲线：{} 个采样点，顺序读遍记录区耗时 {:.2}s（前端会缓存这份结果）",
        curve.len(),
        elapsed.as_secs_f64()
    );

    println!("\n曲线抽样（排名 -> 累计覆盖率）：");
    let step = (curve.len() / 14).max(1);
    for (r, c) in curve.iter().step_by(step) {
        println!("  {:>10}  {:>7.3}%", r, c * 100.0);
    }
    let (lr, lc) = curve.last().unwrap();
    println!("  {lr:>10}  {:>7.3}%", lc * 100.0);

    println!("\n覆盖率目标 -> 排名阈值（这就是「按覆盖率分组」要用的七个上界）：");
    let mut prev = 0u32;
    for (i, tg) in targets.iter().enumerate() {
        let r = rank_for_coverage(&curve, tg / 100.0).unwrap_or(0);
        let name = vocfreq_core::rank::TIER_NAMES
            .get(i)
            .copied()
            .unwrap_or("(第7组)");
        let warn = if r < prev {
            "  ⚠ 目标覆盖率未严格递增，会得到空组"
        } else {
            ""
        };
        println!("  {name:<4} 目标 {tg:>6.2}%  ->  rank <= {r}{warn}");
        prev = r.max(prev);
    }
    println!(
        "\n把上界填进设置里的 tierWordBounds / tierCharBounds，并把 tierMethod 设为 coverage，"
    );
    println!("分组就会按覆盖率来切（默认口径是「前%」，见 vocfreq pct）。");
    Ok(())
}

// ---------------------------------------------------------------- pct

/// 把「前%上界」换算成排名阈值，或反过来。
///
/// **前%是默认口径**：`前% = 排名 ÷ 该表条目数 × 100`。它与覆盖率是两回事 ——
/// 覆盖率问的是"前 N 个词盖住了正文的百分之多少"，前%问的是"这个词排在前百分之几"。
/// 覆盖率随分布走（Zipf 后头部极陡），前%是纯位次，跨表、跨语料库都可比。
fn cmd_pct(data: PathBuf, table: String, pcts: Vec<f64>, ranks: Vec<u32>) -> Result<()> {
    let ds =
        Dataset::open(&data).with_context(|| format!("打开产物目录 {} 失败", data.display()))?;
    let t = ds.table_by_key(&table).ok_or_else(|| {
        anyhow::anyhow!(
            "找不到表 {table}；本产物里有：{}",
            ds.tables
                .iter()
                .map(|x| x.key())
                .collect::<Vec<_>>()
                .join("、")
        )
    })?;
    let kind = t.kind.clone();
    let entries = t.entries;
    println!(
        "表 {table}（{kind}）  {entries} 条  {} token",
        t.total_tokens
    );

    if !ranks.is_empty() {
        println!("\n排名 -> 前%：");
        for r in ranks {
            println!(
                "  rank {r:>10}  ->  前 {}",
                fmt_top_pct(vocfreq_core::rank::pct_for_rank(r, entries))
            );
        }
    } else {
        println!("\n前%上界 -> 排名上界（默认七组口径）：");
        let defaults = vocfreq_core::rank::default_tier_pct_for(&kind).to_vec();
        let list = if pcts.is_empty() {
            defaults.clone()
        } else {
            pcts.clone()
        };
        let mut prev = 0u32;
        for (i, p) in list.iter().enumerate() {
            let r = vocfreq_core::rank::rank_for_pct(*p, entries);
            let name = vocfreq_core::rank::TIER_NAMES
                .get(i)
                .copied()
                .unwrap_or("(第7组)");
            let warn = if r < prev {
                "  ⚠ 未严格递增，会得到空组"
            } else {
                ""
            };
            println!(
                "  {name:<4} 前 {:>10}  ->  rank <= {r}{warn}",
                fmt_top_pct(*p)
            );
            prev = r.max(prev);
        }
        if list == defaults {
            println!("\n（上面是这张表的默认口径，等于 meta.json 里的 tier_pct 换算结果）");
        }
        println!("\n把前%上界填进设置里的 tierPct / tierPctChar，并把 tierMethod 设为 top_pct。");
    }
    Ok(())
}

// ---------------------------------------------------------------- compose

/// 把若干张表相加成一张新表。
///
/// 相加在数学上是精确的：`scan` 本身就是"逐作用域扫完再累加"，每个 token 只属于一个
/// 作用域，所以各作用域表相加 == 全量扫描出来的表（逐条相等，有测试钉住）。
///
/// 源表**可以跨产物**（全量语料放不下时只能分几次扫），前提是各产物的词库链与分词
/// 口径完全一致 —— 这一关由核心库严格校验，不一致就报错拒绝。
fn cmd_compose(a: ComposeArgs) -> Result<()> {
    let first = a
        .from_data
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("--from-data 至少要给一个产物目录"))?;
    let spec = vocfreq_core::compose::ComposeSpec {
        from: first.clone(),
        products: a.from_data.iter().skip(1).cloned().collect(),
        sources: a.sources.clone(),
        scope: a.scope.clone(),
        kind: a.kind.clone(),
        out: a.out.clone(),
    };
    let written = vocfreq_core::compose::compose(&spec).with_context(|| {
        format!(
            "相加失败（源产物 {}）",
            a.from_data
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("、")
        )
    })?;

    println!("✅ 相加完成：作用域「{}」", a.scope);
    for c in &written {
        println!(
            "   {} 表：{} 条 / {} token / {:.1} MB",
            c.kind,
            c.entries,
            c.total_tokens,
            c.bytes as f64 / 1e6
        );
        println!("   来源：{}", c.sources.join(" + "));
    }
    println!("   落地：{}", written[0].out);
    println!("\n它现在和别的表完全平级：可以当主作用域（设置里的 primaryScope），也可以再被相加。");
    Ok(())
}

// ---------------------------------------------------------------- merge

/// 把若干份独立产物**合流**成一份。
///
/// 这是"全量语料放不下、只能一个域一个域扫"那条工作流的中间一步：每份产物各扫一个域，
/// 合流成一份标准产物，再用 `compose` 把各作用域相加出 `full`。
///
/// 硬门槛是各源产物的**词库链与分词口径完全一致**（逐份校验、比内容指纹）。不一致就
/// 报错拒绝，并指出是哪一份产物、哪个字段不同 —— 绝不静默合并，也绝不产出半截产物。
fn cmd_merge(a: MergeArgs) -> Result<()> {
    let spec = vocfreq_core::merge::MergeSpec {
        from: a.from_data.clone(),
        out: a.out.clone(),
        scope: a.scope.clone(),
    };
    let r = vocfreq_core::merge::merge(&spec).with_context(|| {
        format!(
            "合流失败（源产物 {}）",
            a.from_data
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("、")
        )
    })?;

    println!("✅ 合流完成，用时 {:.1}s", r.elapsed_ms as f64 / 1000.0);
    println!("   源产物 {} 份：", r.from.len());
    for p in &r.from {
        println!("     {p}");
    }
    println!("   目标产物：{}", r.out);
    println!("   作用域 {} 个：{}", r.scopes.len(), r.scopes.join(" / "));
    println!(
        "   表 {} 张 / {} 条 / {} token / {:.1} MB",
        r.tables,
        r.entries,
        r.total_tokens,
        r.bytes as f64 / 1e6
    );
    println!(
        "\n下一步：把各作用域相加出 full ——\n  \
         vocfreq compose --from-data {} --source <域>/word ... --scope full\n\
         合流产物是一份**标准产物**，桌面端的「表管理」也能直接打开它。",
        r.out
    );
    Ok(())
}

// ---------------------------------------------------------------- oov

/// 从已有的 `<作用域>/word.tsv` 重新导出词典外候选词。
///
/// 这样调筛选条件（最小频次、词长、是否只留汉字）时不必重跑几十分钟的统计。
/// 词表本身按频次降序，所以输出天然也是按频次降序。
fn cmd_oov(
    data: PathBuf,
    min_count: u64,
    min_len: usize,
    cjk_only: bool,
    limit: usize,
    scope: &str,
) -> Result<()> {
    use std::io::BufRead;

    let tsv = data.join(scope).join("word.tsv");
    if !tsv.exists() {
        bail!(
            "{} 不存在。\n\
             `oov` 读的是可读 TSV，而它只在统计时带 --tsv（默认带）才会写出来；\n\
             另外作用域名要写对（现在的做法是每个作用域一个目录，例如 full、news）。",
            tsv.display()
        );
    }
    let f = std::fs::File::open(&tsv).with_context(|| format!("打开 {} 失败", tsv.display()))?;
    let rd = std::io::BufReader::with_capacity(1 << 22, f);

    let mut out: Vec<(Box<str>, u64)> = Vec::new();
    let mut scanned = 0u64;
    let mut kept = 0u64;
    for line in rd.lines() {
        let line = line?;
        if line.starts_with('#') || line.starts_with("rank\t") {
            continue;
        }
        scanned += 1;
        let mut it = line.split('\t');
        let _rank = it.next();
        let count: u64 = match it.next().and_then(|s| s.parse().ok()) {
            Some(c) => c,
            None => continue,
        };
        let _pct = it.next();
        let word = match it.next() {
            Some(w) => w,
            None => continue,
        };
        let flags: u8 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        // bit0 = 在 jieba 词典内；我们要的正是「不在词典内」的
        if flags & 1 != 0 {
            continue;
        }
        if count < min_count || word.chars().count() < min_len {
            continue;
        }
        if cjk_only && !word.chars().any(vocfreq_core::clean::is_cjk_ideograph) {
            continue;
        }
        kept += 1;
        if out.len() < limit {
            out.push((word.into(), count));
        }
    }

    if out.is_empty() {
        bail!("按当前条件没有筛出任何词典外候选词（min_count={min_count}, min_len={min_len}）");
    }
    let dst = data.join("oov_candidates.tsv");
    vocfreq_core::artifact::write_oov(&dst, &out)?;
    println!(
        "扫描 {scanned} 条词表，词典外且符合条件的共 {kept} 条，已写入 {}（{:.1} KB）",
        dst.display(),
        std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0) as f64 / 1024.0
    );
    println!("\n前 25 条：");
    for (w, c) in out.iter().take(25) {
        println!("{c:>10}  {w}");
    }
    Ok(())
}

// ---------------------------------------------------------------- detect

fn cmd_detect(corpus: PathBuf, rules: Option<PathBuf>) -> Result<()> {
    let rules = load_rules(&rules)?;
    let rows = scan::detect_report(&corpus, &rules)?;
    let mut by_rule: std::collections::BTreeMap<String, (usize, u64)> = Default::default();
    println!("{:<62} {:<18} {:>10}", "文件", "识别规则", "体积");
    println!("{}", "-".repeat(92));
    for (path, rule, bytes) in &rows {
        println!(
            "{:<62} {:<18} {:>9.1}MB",
            truncate(path, 60),
            rule,
            *bytes as f64 / 1e6
        );
        let e = by_rule.entry(rule.clone()).or_insert((0, 0));
        e.0 += 1;
        e.1 += bytes;
    }
    println!("{}", "-".repeat(92));
    println!(
        "共 {} 个文件，合计 {:.2} GB",
        rows.len(),
        rows.iter().map(|r| r.2).sum::<u64>() as f64 / 1e9
    );
    for (rule, (n, bytes)) in by_rule {
        println!("  {rule:<20} {n:>4} 文件  {:>8.2} GB", bytes as f64 / 1e9);
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_string();
    }
    format!(
        "...{}",
        chars[chars.len() - n + 3..].iter().collect::<String>()
    )
}

// ---------------------------------------------------------------- scan

fn cmd_scan(a: ScanArgs) -> Result<()> {
    let rules = match &a.rules {
        Some(_) => Some(load_rules(&a.rules)?),
        None => None,
    };

    let mut cfg = ScanConfig::new(a.corpus.clone(), a.out.clone());
    cfg.threads = a.threads;
    cfg.dicts = resolve_dicts(&a.dicts.dict, &a.dicts.dict_dir, &a.dicts.user_dict)?;
    cfg.rules = rules;
    cfg.only_domains = a.domains.clone();
    cfg.skip_domain_tables = a.no_domains;
    cfg.write_full = a.full;
    cfg.min_count = a.min_count;
    cfg.write_tsv = !a.no_tsv;
    cfg.oov_min_count = a.oov_min_count;
    cfg.tok = TokenizeOpts {
        hmm: a.hmm,
        min_len: 1,
        max_len: 64,
        keep_latin: !a.no_latin,
        keep_digit: a.keep_digit,
        skip_single_char: a.skip_single_char,
    };

    let mode = a.progress;
    // 进度回调要求 `Fn`（会被多个 rayon 线程共享调用），因此状态必须放在 Mutex 里，
    // 不能用闭包直接可变捕获。
    let last_bar = std::sync::Mutex::new(String::new());
    let cb = move |p: Progress| {
        let mut lb = match last_bar.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match mode {
            // none 只关掉「进度」，诊断日志仍然输出——否则用户看不到任何统计结果摘要
            ProgressMode::None => {
                if let Progress::Log { level, message } = &p {
                    eprintln!("[{level}] {message}");
                }
            }
            ProgressMode::Json => {
                if let Ok(s) = serde_json::to_string(&p) {
                    eprintln!("{s}");
                }
            }
            ProgressMode::Bar => match &p {
                Progress::Log { level, message } => {
                    eprint!("\r{}\r", " ".repeat(lb.len()));
                    lb.clear();
                    eprintln!("[{level}] {message}");
                }
                Progress::Plan {
                    files,
                    bytes,
                    domains,
                    ..
                } => {
                    eprintln!(
                        "发现 {files} 个文件 / {:.2} GB / {} 个域",
                        *bytes as f64 / 1e9,
                        domains.len()
                    );
                }
                Progress::Phase {
                    domain,
                    bytes_done,
                    bytes_total,
                    percent,
                    ..
                } => {
                    let bar_len = 28;
                    let filled = ((percent * bar_len as f64) as usize).min(bar_len);
                    let line = format!(
                        "  {:<8} [{}{}] {:>5.1}%  {:>6.1}/{:.1} GB",
                        domain,
                        "#".repeat(filled),
                        "-".repeat(bar_len - filled),
                        percent * 100.0,
                        *bytes_done as f64 / 1e9,
                        *bytes_total as f64 / 1e9
                    );
                    eprint!("\r{line}");
                    *lb = line;
                }
                Progress::Table { table, entries, .. } => {
                    eprint!("\r{}\r", " ".repeat(lb.len()));
                    lb.clear();
                    eprintln!("  表 {table}: {entries} 条");
                }
                Progress::Done { .. } => {}
            },
        }
    };

    let meta = scan::scan(&cfg, &cb)?;
    if matches!(a.progress, ProgressMode::Bar) {
        println!();
    }
    println!("{}", vocfreq_core::artifact::describe(&meta));
    // 分组覆盖率用于校准阈值。默认口径下 `scan` 不产 `full`，那就拿第一个作用域的词表
    // 报一遍 —— 它仍然是"这张表的分布长什么样"，比什么都不打印有用。
    let sample = meta
        .table(vocfreq_core::artifact::SCOPE_FULL, "word")
        .or_else(|| meta.tables.iter().find(|t| t.kind == "word"));
    match sample {
        Some(t) => {
            println!("\n分组覆盖率（用于校准阈值，取自 {}/word）：", t.path);
            println!(
                "  {:<6} {:<12} {:>10} 词  覆盖 {:>6.2}%  累计 {:>6.2}%",
                "词表", "分组", "词条数", "占比", "累计"
            );
            for s in &t.tier_stats {
                println!(
                    "  {:<8} {:<10} {:>10}      {:>6.2}%        {:>6.2}%",
                    "",
                    s.name,
                    s.entries,
                    s.coverage * 100.0,
                    s.cumulative * 100.0
                );
            }
        }
        None => println!("\n（没有词表，跳过分组覆盖率）"),
    }
    if !cfg.write_full
        && meta
            .table(vocfreq_core::artifact::SCOPE_FULL, "word")
            .is_none()
    {
        println!(
            "\n下一步：把各分域的产物合流，再把作用域相加出 full ——\n  \
             vocfreq merge --from-data <分片A> --from-data <分片B> --scope <表名> --out <产物目录>\n  \
             vocfreq compose --from-data <产物目录> --source <域1>/word --source <域2>/word --scope full"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- info / lookup

fn cmd_info(table: PathBuf) -> Result<()> {
    let t = VfrTable::open(&table).with_context(|| format!("打开 {} 失败", table.display()))?;
    let h = t.header();
    println!("文件      {}", t.path());
    println!("版本      {}  magic 校验通过", h.version);
    println!(
        "类型      {}",
        if h.kind == vocfreq_core::query::KIND_CHAR {
            "字表"
        } else {
            "词表"
        }
    );
    println!("条目数    {}", h.entry_count);
    println!("总 token  {}", h.total_tokens);
    println!("块        {} 块 × {} 条", h.block_count, h.block_size);
    println!(
        "区段      header=0..96  块索引={}  块首词={}  排名索引={}",
        h.index_offset, h.heads_offset, h.rank_index_offset
    );
    println!("\n前 10 名：");
    println!("{:>6}  {:>12}  {:>9}  {}", "排名", "频次", "占比", "词");
    for e in t.range_by_rank(1, 10) {
        let pct = e.count as f64 * 100.0 / h.total_tokens.max(1) as f64;
        println!("{:>6}  {:>12}  {:>8.5}%  {}", e.rank, e.count, pct, e.word);
    }
    let mid = (h.entry_count / 2).max(1) as u32;
    println!("\n第 {mid} 名附近：");
    for e in t.range_by_rank(mid, 3) {
        println!("{:>6}  {:>12}  {:>9}  {}", e.rank, e.count, "", e.word);
    }
    Ok(())
}

fn cmd_lookup(table: PathBuf, words: Vec<String>, top: u32) -> Result<()> {
    let t = VfrTable::open(&table).with_context(|| format!("打开 {} 失败", table.display()))?;
    let total = t.header().total_tokens.max(1);
    if top > 0 {
        println!("前 {top} 名：");
        for e in t.range_by_rank(1, top) {
            println!(
                "{:>8}  {:>12}  {:>9.6}%  {}",
                e.rank,
                e.count,
                e.count as f64 * 100.0 / total as f64,
                e.word
            );
        }
        println!();
    }
    if words.is_empty() {
        return Ok(());
    }
    println!("{:>8}  {:>12}  {:>9}  {}", "排名", "频次", "占比", "词");
    for w in &words {
        match t.lookup(w) {
            Some(h) => println!(
                "{:>8}  {:>12}  {:>8.6}%  {}",
                h.rank,
                h.count,
                h.count as f64 * 100.0 / total as f64,
                w
            ),
            None => println!("{:>8}  {:>12}  {:>9}  {}", "-", "-", "-", w),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- segment

/// 分组配色，与 `docs/DESIGN.md` §5.4 / `tier-colors.ts` 的**浅色前景色**一致，
/// 便于在终端预览界面效果。
///
/// 语义是游戏稀有度色阶：极多=普通（灰白）… 极少=神器（金）。
const TIER_RGB: [(u8, u8, u8); 7] = [
    (0x47, 0x54, 0x67), // 极多 普通 · 灰白
    (0x13, 0x7A, 0x3A), // 很多 优秀 · 绿
    (0x1C, 0x4B, 0xC4), // 较多 精良 · 蓝
    (0x6B, 0x28, 0xC9), // 中等 史诗 · 紫
    (0xB0, 0x3C, 0x08), // 较少 传说 · 橙
    (0xB9, 0x1C, 0x1C), // 很少 神话 · 红
    (0x7A, 0x5C, 0x00), // 极少 神器 · 金
];
const UNKNOWN_RGB: (u8, u8, u8) = (0x98, 0xA2, 0xB3);

/// 把 `--dict` / `--dict-dir` / `--user-dict` 解析成一条词库链。
///
/// 顺序：`--dict`（按给出顺序）→ `--dict-dir` 里的（按文件名排序）→ `--user-dict`
/// （按给出顺序）。这样**主词库永远是显式指定的那一份**，目录扫描只用来补充，
/// 不会悄悄顶替主词库。
///
/// 按规范化路径去重：`--dict main.dict --dict-dir dicts\` 这种写法很自然，
/// 但 main.dict 会被列两次 —— 重复装载不报错，只是白费时间，还可能让
/// 「叠加词库」的标记变得莫名其妙。
fn resolve_dicts(
    dicts: &[PathBuf],
    dict_dir: &Option<PathBuf>,
    user_dicts: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let push = |p: PathBuf, chain: &mut Vec<PathBuf>| {
        let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if !chain
            .iter()
            .any(|q| std::fs::canonicalize(q).unwrap_or_else(|_| q.clone()) == key)
        {
            chain.push(p);
        }
    };

    for p in dicts {
        push(p.clone(), &mut chain);
    }
    if let Some(dir) = dict_dir {
        let found = dict::list_dicts(dir);
        if found.is_empty() {
            bail!("{} 里没有 .dict 词库文件", dir.display());
        }
        for p in found {
            push(p, &mut chain);
        }
    }
    for p in user_dicts {
        push(p.clone(), &mut chain);
    }

    if chain.is_empty() {
        bail!(
            "没有词库。现在词库不再编进程序里，必须显式指定一个：\n  \
             --dict <FILE>      指定一份词库；可重复，第一份是主词库\n  \
             --dict-dir <DIR>   取目录里所有 .dict（按文件名排序）\n  \
             桌面端的数据文件夹里默认放在 dicts\\ 子目录下。\n\
             提示：词库格式是 jieba 的「词 词频 词性」，词频与词性可省略，\
             以 # 开头的整行是注释。"
        );
    }
    Ok(chain)
}

/// 按 `meta.json` 记录的词库链，在**表所在目录**附近找回实际的词库文件。
///
/// 三种来源，按可靠性排序：
///
/// 1. 记录里的绝对路径还活着 → 直接用。
/// 2. **数据文件夹的约定布局**：表在 `<数据文件夹>\tables\<名字>\`，词库在同级的
///    `<数据文件夹>\dicts\`。出厂预置的表就是靠这一条 —— 它在 `path` 里**不记路径**
///    （记了会把构建机的目录结构泄露出厂产物，而且装到用户机器上那个路径根本不存在）。
/// 3. 在 `dicts\` 里按 **sha256** 找 —— 用户把词库改过名也认得出来。
///
/// 每一处不一致都必须报出来：否则用户会拿一份跟词表对不上的词库去分析，
/// 频次系统性偏错却毫不知情。但也只是警告，不阻断 —— 产物本身仍然能查。
fn chain_from_meta(table_dir: &Path, m: &vocfreq_core::artifact::TokenizerMeta) -> Vec<PathBuf> {
    // `<表目录>\..\..\dicts` 就是数据文件夹的 dicts\
    let dicts_dir = table_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|d| d.join("dicts"));

    let mut out = Vec::new();
    let mut found_any = false;

    for d in &m.resolved_dicts() {
        let hit = resolve_one(d, &dicts_dir);
        match hit {
            Some(p) => {
                warn_if_drifted(&p, d);
                out.push(p);
                found_any = true;
            }
            None => {
                let recorded = if d.path.is_empty() {
                    "（出厂预置表刻意不记路径）".to_string()
                } else {
                    d.path.clone()
                };
                let where_to_look = dicts_dir
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| {
                        r"（表不在数据文件夹的 tables\ 下，没有对应的 dicts\）".to_string()
                    });
                eprintln!(
                    "[warn] 找不到建表时用的词库「{}」。\n  \
                     记录里的路径：{}\n  \
                     数据文件夹：{}\n  \
                     可用 --dict / --dict-dir 手动指定。",
                    d.name, recorded, where_to_look
                );
            }
        }
    }

    if !found_any && !m.resolved_dicts().is_empty() {
        eprintln!("[warn] 上面这条链一份都没找回来，请用 --dict 或 --dict-dir 指定");
    }
    out
}

/// 找一条记录对应的词库文件。
fn resolve_one(rec: &vocfreq_core::dict::DictRef, dicts_dir: &Option<PathBuf>) -> Option<PathBuf> {
    // 1) 记录里的绝对路径（出厂预置表为空，自然跳过）
    if !rec.path.is_empty() {
        let p = PathBuf::from(&rec.path);
        if p.exists() {
            return Some(p);
        }
    }
    let dir = dicts_dir.as_deref()?;

    // 2) 指纹优先：用户把词库改过名也认得出来
    if rec.is_verifiable() {
        for c in dict::list_dicts(dir) {
            if let Ok(h) = dict::sha256_file(&c) {
                if h.eq_ignore_ascii_case(&rec.sha256) {
                    return Some(c);
                }
            }
        }
    }

    // 3) 按名字兜底（词库被改过 → 指纹不同，但仍是"同一份"）
    let by_name = dir.join(format!("{}.dict", rec.name));
    by_name.exists().then_some(by_name)
}

/// 内容与建表时不一致就警告。**不阻断**：用户可能就是想试试新词库。
fn warn_if_drifted(p: &Path, rec: &vocfreq_core::dict::DictRef) {
    if !rec.is_verifiable() {
        return;
    }
    match dict::sha256_file(p) {
        Ok(h) if !h.eq_ignore_ascii_case(&rec.sha256) => eprintln!(
            "[warn] 词库 {} 的内容与建表时不一致（文件被改过，或换成了同名的另一份），\
             分词结果可能与词表对不上",
            p.display()
        ),
        Ok(_) => {}
        Err(e) => eprintln!("[warn] 校验词库 {} 失败：{e}", p.display()),
    }
}

fn cmd_segment(
    data: PathBuf,
    text: Vec<String>,
    json: bool,
    hmm: bool,
    dicts: DictArgs,
    color: bool,
    domains: Vec<String>,
) -> Result<()> {
    let ds =
        Dataset::open(&data).with_context(|| format!("打开产物目录 {} 失败", data.display()))?;
    if hmm != ds.meta.tokenizer.hmm {
        eprintln!(
            "[warn] 你指定的 HMM={hmm} 与建表时的 HMM={} 不一致，分词结果可能与词表对不上",
            ds.meta.tokenizer.hmm
        );
    }
    let opts = TokenizeOpts {
        hmm,
        min_len: ds.meta.tokenizer.min_len,
        max_len: ds.meta.tokenizer.max_len,
        keep_latin: ds.meta.tokenizer.keep_latin,
        keep_digit: ds.meta.tokenizer.keep_digit,
        skip_single_char: ds.meta.tokenizer.skip_single_char,
    };
    // 词库链必须跟建表时一致，否则分词结果对不上已经落盘的词表，查出来的频次会
    // 系统性偏错。所以默认照 meta.json 记录的链重建，只有用户显式给了才覆盖。
    let explicit =
        !dicts.dict.is_empty() || dicts.dict_dir.is_some() || !dicts.user_dict.is_empty();
    let chain = if explicit {
        resolve_dicts(&dicts.dict, &dicts.dict_dir, &dicts.user_dict)?
    } else {
        chain_from_meta(&data, &ds.meta.tokenizer)
    };
    if chain.is_empty() {
        bail!(
            "没有可用的词库，无法重建分词器。\n\
             产物里记录的是：{}\n\
             请用 --dict <FILE> 或 --dict-dir <DIR> 指定现在用哪份词库。",
            dict::describe_chain(&ds.meta.tokenizer.resolved_dicts())
        );
    }

    let tk = Tokenizer::from_dicts(&chain, opts)
        .with_context(|| format!("装载词库链失败（共 {} 份）", chain.len()))?;

    let input = if text.is_empty() {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        s
    } else {
        text.join(" ")
    };
    if input.trim().is_empty() {
        bail!("没有输入文本。请作为参数传入，或从标准输入管道输入。");
    }

    let infos = ds.analyze(&tk, &input);

    if json {
        let payload = serde_json::json!({
            "meta": {
                "generated_at": ds.meta.generated_at,
                "tokenizer": ds.meta.tokenizer,
                "tier_names": ds.meta.tier_names,
                "primary_scope": ds.primary_scope,
                "total_tokens": ds
                    .primary_table("word")
                    .map(|t| t.total_tokens)
                    .unwrap_or(0),
            },
            "tokens": infos,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    // 用 token 的字节区间把原文重新拼出来并着色
    print!("着色： ");
    for t in &infos {
        let label = &t.text;
        match t.tier {
            Some(i) => {
                if color {
                    let (r, g, b) = TIER_RGB[i.min(6)];
                    print!("\x1b[38;2;{r};{g};{b}m{label}\x1b[0m");
                } else {
                    print!("{label}");
                }
            }
            None => {
                if t.accepted {
                    if color {
                        let (r, g, b) = UNKNOWN_RGB;
                        print!("\x1b[38;2;{r};{g};{b}m{label}\x1b[0m");
                    } else {
                        print!("{label}");
                    }
                } else {
                    print!("{label}");
                }
            }
        }
    }
    println!("\n");

    let names = &ds.meta.tier_names;
    println!(
        "{:<14} {:>10} {:>10} {:>12} {:>9}  {:<6} 标记",
        "词", "频次", "排名", "前%", "占比", "分组"
    );
    println!("{}", "-".repeat(92));
    for t in &infos {
        if !t.accepted {
            continue;
        }
        let tier = match t.tier {
            Some(i) => names.get(i).cloned().unwrap_or_default(),
            None => "未收录".to_string(),
        };
        let mut marks = Vec::new();
        if t.in_dict == Some(false) {
            marks.push("词典外");
        }
        if t.from_user == Some(true) {
            marks.push("用户词典");
        }
        if t.single_cjk {
            marks.push("单字→字表");
        }
        if t.table == "word" && t.count.is_none() {
            marks.push("词表未收录");
        }
        // 「各表对比」一律用前%：排名绝对值跨表不可比，前%可以。
        let cmp: Vec<String> = t
            .table_ranks
            .iter()
            .filter(|r| domains.is_empty() || domains.iter().any(|d| d == &r.scope))
            .map(|r| match r.top_pct {
                Some(p) => format!("{} 前{}", r.scope, fmt_top_pct(p)),
                None => format!("{}-", r.scope),
            })
            .collect();
        println!(
            "{:<14} {:>10} {:>10} {:>12} {:>9}  {:<6} {}",
            t.text,
            t.count.map(|c| c.to_string()).unwrap_or_else(|| "-".into()),
            t.rank.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
            t.top_pct.map(fmt_top_pct).unwrap_or_else(|| "-".into()),
            t.pct
                .map(|p| format!("{p:.5}%"))
                .unwrap_or_else(|| "-".into()),
            tier,
            marks.join(" ")
        );
        if !cmp.is_empty() {
            println!("{:<14} 各表对比: {}", "", cmp.join("  "));
        }
    }

    // 图例
    let tiers = ds
        .primary_table("word")
        .and_then(|t| ds.meta.table(&t.scope, "word").map(|m| m.tiers.clone()))
        .unwrap_or_default();
    let pct_bounds = ds
        .meta
        .table(&ds.primary_scope, "word")
        .map(|m| m.effective_tier_pct())
        .unwrap_or_default();
    println!("\n主作用域：{}", ds.primary_scope);
    print!("图例：");
    for (i, t) in tiers.iter().enumerate() {
        // 前%口径才是主口径，图例直接给前%，排名绝对值只在没有前%数据时才用
        let last = i + 1 == tiers.len();
        let label = match pct_bounds.get(i).copied() {
            // 第 7 组没有上界，前%口径下就是「前 100%」
            _ if last && !pct_bounds.is_empty() => format!("{} 前 100%", t.name),
            Some(p) => format!("{} 前{}", t.name, fmt_top_pct(p)),
            None => {
                let bound = if t.max_rank == u64::MAX {
                    format!(
                        ">{}",
                        tiers
                            .get(i.saturating_sub(1))
                            .map(|x| x.max_rank)
                            .unwrap_or(0)
                    )
                } else {
                    format!("<={}", t.max_rank)
                };
                format!("{} {}", t.name, bound)
            }
        };
        if color {
            let (r, g, b) = TIER_RGB[i.min(6)];
            print!("\x1b[38;2;{r};{g};{b}m{label}\x1b[0m  ");
        } else {
            print!("{label}  ");
        }
    }
    if color {
        let (r, g, b) = UNKNOWN_RGB;
        print!("\x1b[38;2;{r};{g};{b}m未收录\x1b[0m");
    } else {
        print!("未收录");
    }
    println!();
    let _ = std::io::stdout().flush();
    Ok(())
}

/// 前%的终端格式化：头部排名是 0.0000263% 这种极小值，固定小数位会压成 0.00%。
fn fmt_top_pct(p: f64) -> String {
    let digits = if p >= 100.0 {
        0
    } else if p >= 10.0 {
        1
    } else if p >= 1.0 {
        2
    } else if p >= 0.01 {
        3
    } else if p >= 0.001 {
        4
    } else if p >= 0.0001 {
        5
    } else {
        7
    };
    format!("{p:.digits$}%")
}

#[cfg(test)]
mod tests;
