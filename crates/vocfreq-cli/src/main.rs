//! VocTier 语料库字词频率统计工具（纯 Rust，不依赖 Tauri）。

use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
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

    /// 全量统计并产出频率表
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
        /// 叠加用户词典
        #[arg(long)]
        user_dict: Option<PathBuf>,
        /// 用 ANSI 真彩色显示分组配色
        #[arg(long)]
        color: bool,
        /// 只看这些分域的排名
        #[arg(long, value_delimiter = ',')]
        domains: Vec<String>,
    },

    /// 从已生成的词表里重新导出「词典外候选词」，无需重跑统计
    Oov {
        /// vocfreq scan 的产物目录
        #[arg(long)]
        data: PathBuf,
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
    /// 叠加用户词典（jieba 格式：词 词频 词性，词频可省略）
    #[arg(long)]
    user_dict: Option<PathBuf>,
    /// 完全替换内置词典（而非叠加）
    #[arg(long)]
    dict: Option<PathBuf>,
    /// 自定义语料解析规则 JSON
    #[arg(long)]
    rules: Option<PathBuf>,
    /// 只统计这些域（逗号分隔）；默认全部
    #[arg(long, value_delimiter = ',')]
    domains: Vec<String>,
    /// 不产出各分域的子表，只要全库总表
    #[arg(long)]
    no_domains: bool,
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
            user_dict,
            color,
            domains,
        } => cmd_segment(data, text, json, hmm, user_dict, color, domains),
        Cmd::Oov {
            data,
            min_count,
            min_len,
            no_cjk_only,
            limit,
        } => cmd_oov(data, min_count, min_len, !no_cjk_only, limit),
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
fn cmd_curve(data: PathBuf, table: String, points: usize, targets: Vec<f64>) -> Result<()> {
    use vocfreq_core::query::{rank_for_coverage, KIND_CHAR};

    let ds = Dataset::open(&data).with_context(|| format!("打开产物目录 {} 失败", data.display()))?;
    let t = ds.table_by_path(&table).ok_or_else(|| {
        anyhow::anyhow!(
            "找不到表 {table}；可用取值形如 full/word、full/char、domains/news/word、domains/news/char"
        )
    })?;
    let h = t.header();

    let t0 = std::time::Instant::now();
    let curve = t.coverage_curve(points);
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
        let name = vocfreq_core::rank::TIER_NAMES.get(i).copied().unwrap_or("(第7组)");
        let warn = if r < prev { "  ⚠ 目标覆盖率未严格递增，会得到空组" } else { "" };
        println!("  {name:<4} 目标 {tg:>6.2}%  ->  rank <= {r}{warn}");
        prev = r.max(prev);
    }
    println!("\n把上界填进设置里的 tierWordBounds / tierCharBounds，并把 tierMethod 设为 coverage，");
    println!("分组就会按覆盖率而不是绝对排名来切。");
    Ok(())
}

// ---------------------------------------------------------------- oov

/// 从已有的 `full/word.tsv` 重新导出词典外候选词。
///
/// 这样调筛选条件（最小频次、词长、是否只留汉字）时不必重跑几十分钟的统计。
/// 词表本身按频次降序，所以输出天然也是按频次降序。
fn cmd_oov(
    data: PathBuf,
    min_count: u64,
    min_len: usize,
    cjk_only: bool,
    limit: usize,
) -> Result<()> {
    use std::io::BufRead;

    let tsv = data.join("full").join("word.tsv");
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
        println!("{:<62} {:<18} {:>9.1}MB", truncate(path, 60), rule, *bytes as f64 / 1e6);
        let e = by_rule.entry(rule.clone()).or_insert((0, 0));
        e.0 += 1;
        e.1 += bytes;
    }
    println!("{}", "-".repeat(92));
    println!("共 {} 个文件，合计 {:.2} GB", rows.len(), rows.iter().map(|r| r.2).sum::<u64>() as f64 / 1e9);
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
    format!("...{}", chars[chars.len() - n + 3..].iter().collect::<String>())
}

// ---------------------------------------------------------------- scan

fn cmd_scan(a: ScanArgs) -> Result<()> {
    let rules = match &a.rules {
        Some(_) => Some(load_rules(&a.rules)?),
        None => None,
    };

    let mut cfg = ScanConfig::new(a.corpus.clone(), a.out.clone());
    cfg.threads = a.threads;
    cfg.user_dict = a.user_dict.clone();
    cfg.dict = a.dict.clone();
    cfg.rules = rules;
    cfg.only_domains = a.domains.clone();
    cfg.skip_domain_tables = a.no_domains;
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
                Progress::Plan { files, bytes, domains, .. } => {
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
    println!("\n分组覆盖率（用于校准阈值）：");
    for t in meta.tables.iter().filter(|t| t.path == "full/word") {
        println!("  {:<6} {:<12} {:>10} 词  覆盖 {:>6.2}%  累计 {:>6.2}%",
            "词表", "分组", "词条数", "占比", "累计");
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
        if h.kind == vocfreq_core::query::KIND_CHAR { "字表" } else { "词表" }
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
            println!("{:>8}  {:>12}  {:>9.6}%  {}", e.rank, e.count, e.count as f64 * 100.0 / total as f64, e.word);
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

/// 分组配色，与 `docs/DESIGN.md` §5.4 的浅色前景色一致，便于在终端预览界面效果。
const TIER_RGB: [(u8, u8, u8); 7] = [
    (0xB4, 0x23, 0x18),
    (0xB5, 0x47, 0x08),
    (0x8A, 0x61, 0x00),
    (0x3B, 0x6E, 0x1E),
    (0x17, 0x5C, 0xD3),
    (0x69, 0x41, 0xC6),
    (0x47, 0x54, 0x67),
];
const UNKNOWN_RGB: (u8, u8, u8) = (0x98, 0xA2, 0xB3);

fn cmd_segment(
    data: PathBuf,
    text: Vec<String>,
    json: bool,
    hmm: bool,
    user_dict: Option<PathBuf>,
    color: bool,
    domains: Vec<String>,
) -> Result<()> {
    let ds = Dataset::open(&data).with_context(|| format!("打开产物目录 {} 失败", data.display()))?;
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
    let mut tk = Tokenizer::builtin(opts);
    if let Some(p) = &user_dict {
        tk.load_user_dict(p)?;
    } else if let Some(p) = &ds.meta.tokenizer.user_dict {
        // 建表时用过用户词典，这里必须带上，否则分词结果对不上
        let p = PathBuf::from(p);
        if p.exists() {
            tk.load_user_dict(&p).ok();
        }
    }

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
                "total_tokens": ds.word.header().total_tokens,
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
        "{:<14} {:>10} {:>10} {:>10}  {:<6} {}",
        "词", "频次", "排名", "占比", "分组", "标记"
    );
    println!("{}", "-".repeat(78));
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
        let dom: Vec<String> = t
            .domain_ranks
            .iter()
            .filter(|(n, _)| domains.is_empty() || domains.iter().any(|d| d == n))
            .map(|(n, r)| match r {
                Some(r) => format!("{n}#{r}"),
                None => format!("{n}-"),
            })
            .collect();
        println!(
            "{:<14} {:>10} {:>10} {:>9}  {:<6} {}",
            t.text,
            t.count.map(|c| c.to_string()).unwrap_or_else(|| "-".into()),
            t.rank.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
            t.pct.map(|p| format!("{p:.5}%")).unwrap_or_else(|| "-".into()),
            tier,
            marks.join(" ")
        );
        if !dom.is_empty() {
            println!("{:<14} 分域排名: {}", "", dom.join("  "));
        }
    }

    // 图例
    let total = ds.word.header().total_tokens.max(1);
    let _ = total;
    print!("\n图例：");
    let tiers = ds
        .meta
        .tables
        .iter()
        .find(|t| t.path == "full/word")
        .map(|t| t.tiers.clone())
        .unwrap_or_default();
    for (i, t) in tiers.iter().enumerate() {
        let bound = if t.max_rank == u64::MAX {
            format!(">{}", tiers.get(i.saturating_sub(1)).map(|x| x.max_rank).unwrap_or(0))
        } else {
            format!("<={}", t.max_rank)
        };
        if color {
            let (r, g, b) = TIER_RGB[i.min(6)];
            print!("\x1b[38;2;{r};{g};{b}m{} {}\x1b[0m  ", t.name, bound);
        } else {
            print!("{} {}  ", t.name, bound);
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
