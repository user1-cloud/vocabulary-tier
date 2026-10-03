//! 语料库扫描编排：表组、分块、并行、计数、落盘。
//!
//! 并行策略（依据 `docs/DESIGN.md` §0 的实测）：
//! * 把每个文件切成约 64MB 的**字节区间**，所有区间构成工作队列交给 `par_iter`；
//!   这样即使某个表组只有一个 1GB 大文件也能吃满 32 线程；
//! * 用 `fold` + `reduce` 而非 `collect`，避免几百个中间计数表同时驻留内存；
//! * 计数表**每线程独占**，绝不用互斥锁分片表（实测 32 线程会倒退）。
//!
//! 表组之间串行、表组内并行：每个表组的计数表用完即可释放，整体内存受限，且语料只读一遍。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use rayon::prelude::*;
use serde::Serialize;
use walkdir::WalkDir;

use crate::artifact::{self, Meta, SCHEMA_VERSION, SourceScope, TableMeta, TokenizerMeta};
use crate::clean;
use crate::count::{GlobalCounts, LocalCounts};
use crate::rank::{self, RankedEntry};
use crate::source::{self, SourceRule};
use crate::tokenize::{TokenizeOpts, Tokenizer};
use crate::{Error, Result, Totals, VERSION};

/// 单个工作区间的字节数。
///
/// 这个值决定**并发读流的数量**：区间越小，同一时刻打开的读流越多。实测这台
/// NVMe 的抗并发能力有限——单流顺序读能到 2600 MB/s，但 32 线程开几百个流时
/// 端到端只有约 400 MB/s。16MB 区块（全库 2000+ 区间）比 64MB 慢 60%，因此不要把
/// 它调小；调大又会牺牲负载均衡，64MB 是实测的平衡点。
const CHUNK_BYTES: u64 = 64 * 1024 * 1024;

/// 读缓冲区大小。
const READ_BUF: usize = 1 << 20;

/// 探测格式时最多读多长的首行。真实语料首行远小于此。
const MAX_SAMPLE: usize = 64 * 1024 * 1024;

// ---------------------------------------------------------------- 配置

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub corpus: PathBuf,
    pub out: PathBuf,
    /// 0 表示按可用核心数自动
    pub threads: usize,
    pub tok: TokenizeOpts,
    /// 词典链，**按装载顺序**：第一份是主词典，其后都是叠加词典；同名条目后者覆盖
    /// 前者的词频。
    ///
    /// 现在没有"内置词典"这回事了，所以这里不能为空 —— [`scan`] 会直接报错，
    /// 而不是拿一份空词典跑出一张只有单字的废表。
    pub dicts: Vec<PathBuf>,
    /// 自定义规则；`None` 表示用内置规则做自动探测
    pub rules: Option<Vec<SourceRule>>,
    /// 只处理这些表组；空表示全部
    pub only_domains: Vec<String>,
    /// 是否跳过各表组的子表（只要全库总表）
    pub skip_domain_tables: bool,
    /// 是否**同时**产出全量表组 `full`。
    ///
    /// **默认 `false`**：`scan` 只产各表组自己的表，`full` 一律由
    /// 「跨产物合流 [`crate::merge`] + 各表组 [`crate::compose`] 相加」得到。
    ///
    /// 为什么改成默认不产：全量语料（实测 140 GB）常常大到一块盘放不下，只能
    /// 一个表组一个表组地扫成**多份**产物 —— 那种工作流里每次 `scan` 都顺手写一遍
    /// `full` 是纯浪费（写几百 MB，而且因为每次都整体覆盖，前几次写的全被冲掉）。
    ///
    /// 置 `true` 只为一种用途：**验证**。一次性全量扫一遍、把 `full` 当对照基准，
    /// 与"表组扫 + 合流 + 相加"的结果逐条比对（见 `scan.rs` 与 `compose.rs` 的测试）。
    /// 所以它是内部能力，不对外设开关。
    pub write_full: bool,
    /// 只保留出现次数 >= 该值的词条（1 表示不过滤）
    pub min_count: u64,
    /// 是否同时写可读 TSV
    pub write_tsv: bool,
    /// 非空即导出；实测把阈值设高会只剩 URL 碎片与单字，交调用方决定
    pub oov_min_count: u64,
    pub oov_limit: usize,
    /// 词典外候选词的最小词长。
    ///
    /// 实测默认 1：关闭 HMM 时 jieba 只能输出「词典命中的词」或「未命中的单字」，
    /// 因此**多字中文候选必然为空**（实测最小词长 2 时筛出 0 条）。设为 1 能捞出
    /// 词典外的单字——在你的语料里就是繁体字（對/業/機/華…），它们可以真正回填
    /// 用户词典。多字新词要靠 HMM 或二期的新词发现。
    pub oov_min_len: usize,
    /// 取消标志。置为 true 后，正在扫描的区间会在当前行之后尽快退出。
    /// GUI 里全库统计要跑一分多钟，必须可取消。
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl ScanConfig {
    pub fn new(corpus: impl Into<PathBuf>, out: impl Into<PathBuf>) -> Self {
        ScanConfig {
            corpus: corpus.into(),
            out: out.into(),
            threads: 0,
            tok: TokenizeOpts::default(),
            dicts: Vec::new(),
            rules: None,
            only_domains: Vec::new(),
            skip_domain_tables: false,
            write_full: false,
            min_count: 1,
            write_tsv: true,
            oov_min_count: 500,
            oov_limit: 200_000,
            oov_min_len: 1,
            cancel: None,
        }
    }
}

// ---------------------------------------------------------------- 进度

/// 进度事件。序列化为带 `event` 标签的 JSON，逐行写 stderr，供 Tauri 子进程解析。
///
/// 所有诊断信息都走 [`Progress::Log`] 而不是直接 `eprintln!`，否则在 JSON 模式下
/// 人类可读的日志会混进 JSON 流，把前端的解析器搞坏。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Progress {
    Log {
        level: String,
        message: String,
    },
    Plan {
        files: u64,
        bytes: u64,
        domains: Vec<SourceScope>,
        warnings: Vec<String>,
    },
    /// `units_*` 是工作区间（约 64MB 一块）的数量，不是文件数：大文件会被切成多块。
    Phase {
        phase: String,
        domain: String,
        units_done: u64,
        units_total: u64,
        bytes_done: u64,
        bytes_total: u64,
        percent: f64,
    },
    Table {
        table: String,
        entries: u64,
        total_tokens: u64,
        tier_stats: Vec<rank::TierStat>,
    },
    Done {
        elapsed_ms: u64,
        out: String,
    },
}

pub type ProgressFn<'a> = &'a (dyn Fn(Progress) + Send + Sync);

// ---------------------------------------------------------------- 计划

#[derive(Debug, Clone)]
pub struct FilePlan {
    pub path: PathBuf,
    pub domain: String,
    pub bytes: u64,
    /// 在规则数组里的下标
    pub rule: usize,
}

#[derive(Debug, Clone)]
struct Chunk {
    path: PathBuf,
    rule: usize,
    start: u64,
    end: u64,
}

/// 扫描语料库目录，为每个文件确定表组与解析规则。
///
/// 只读每个文件的**首行**做探测，因此即使语料是 33GB 也很快。
/// 返回 `(计划, 警告)`；警告由调用方通过 [`Progress::Log`] 上报。
pub fn plan_corpus(
    corpus: &Path,
    rules: &[SourceRule],
    only: &[String],
) -> Result<(Vec<FilePlan>, Vec<String>)> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for e in WalkDir::new(corpus).follow_links(false) {
        let e = match e {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !e.file_type().is_file() {
            continue;
        }
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        paths.push(p.to_path_buf());
    }
    paths.sort();

    let mut out = Vec::with_capacity(paths.len());
    let mut errors: Vec<String> = Vec::new();
    for path in paths {
        let rel = path.strip_prefix(corpus).unwrap_or(&path);
        let domain = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or_else(|| "_root".to_string());
        if !only.is_empty() && !only.iter().any(|d| d == &domain) {
            continue;
        }
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        match detect_rule(&path, rules) {
            Ok(rule) => out.push(FilePlan {
                path,
                domain,
                bytes,
                rule,
            }),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    if out.is_empty() {
        let mut msg = format!("在 {} 下没有找到任何可识别的 .jsonl 文件", corpus.display());
        if !errors.is_empty() {
            msg.push_str(&format!(
                "；探测失败 {} 个: {:?}",
                errors.len(),
                &errors[..errors.len().min(5)]
            ));
        }
        return Err(Error::Schema(msg));
    }
    Ok((out, errors))
}

/// 读首行并做规则探测。优先匹配规则里的 `glob` 强制指定。
fn detect_rule(path: &Path, rules: &[SourceRule]) -> Result<usize> {
    let name = path.to_string_lossy();
    if let Some(i) = rules.iter().position(|r| {
        r.glob
            .as_deref()
            .is_some_and(|g| simple_glob_match(g, &name))
    }) {
        return Ok(i);
    }
    let sample = read_first_line(path, MAX_SAMPLE)?;
    if let Ok(rule) = source::detect(rules, &sample) {
        if let Some(i) = rules.iter().position(|r| r.name == rule.name) {
            return Ok(i);
        }
    }
    // 回退：首行可能被截断或含有无法解析的字节，直接在样本里找规则锚点。
    for (i, r) in rules.iter().enumerate() {
        for a in r.anchors() {
            let pat = format!("\"{a}\"");
            if find_bytes(&sample, pat.as_bytes()).is_some() {
                return Ok(i);
            }
        }
    }
    Err(Error::Schema(format!(
        "无法识别格式（样本前 120 字节: {:?}）",
        String::from_utf8_lossy(&sample[..sample.len().min(120)])
    )))
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// 读文件首行。为让 serde 能完整走完顶层字段，这里读**整行**而非截断前缀；
/// 仅用一个很大的上限防御「整个文件就是一行」的极端情况。
fn read_first_line(path: &Path, cap: usize) -> Result<Vec<u8>> {
    use std::io::{BufReader, Read};
    let f = std::fs::File::open(path)?;
    let mut rd = BufReader::with_capacity(1 << 16, f);
    let mut buf = Vec::with_capacity(1 << 16);
    let mut chunk = [0u8; 8192];
    loop {
        let n = rd.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        match chunk[..n].iter().position(|b| *b == b'\n') {
            Some(p) => {
                buf.extend_from_slice(&chunk[..p]);
                break;
            }
            None => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() >= cap {
                    break;
                }
            }
        }
    }
    Ok(buf)
}

/// 极简 glob：只支持 `*` 与 `?`，足够用来按路径强制指定规则。
fn simple_glob_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// ---------------------------------------------------------------- 扫描

/// 把文件列表切成字节区间工作单元。
fn build_chunks(files: &[&FilePlan]) -> Vec<Chunk> {
    let mut v = Vec::new();
    for f in files {
        if f.bytes == 0 {
            continue;
        }
        let n = f.bytes.div_ceil(CHUNK_BYTES).max(1);
        for i in 0..n {
            v.push(Chunk {
                path: f.path.clone(),
                rule: f.rule,
                start: f.bytes * i / n,
                end: f.bytes * (i + 1) / n,
            });
        }
    }
    v
}

/// 扫描一个字节区间，把结果累加进 `totals` 与 `counts`。
///
/// 这里的读法来自实测教训（`docs/DESIGN.md` §0）：
/// * 全程 `read_until(b'\n')` **按字节**读行 —— 语料里存在非法 UTF-8 的行，
///   用 `read_line` 会因解码失败而中断，一行坏数据就能废掉整个区间；
/// * 丢弃边界半行时**复用同一个 `BufReader`** —— 另建一个 reader 会在析构时
///   带走内部已缓冲的约 8KB，导致文件位置前跳、每个区间开头读到截断数据。
fn scan_chunk(
    tk: &Tokenizer,
    rules: &[SourceRule],
    c: &Chunk,
    totals: &mut Totals,
    counts: &mut LocalCounts,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) {
    use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};

    let rule = &rules[c.rule];
    let f = match std::fs::File::open(&c.path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut rd = BufReader::with_capacity(READ_BUF, f);
    let mut line_start = c.start;

    if c.start > 0 {
        if rd.seek(SeekFrom::Start(c.start - 1)).is_err() {
            return;
        }
        let mut one = [0u8; 1];
        if rd.read_exact(&mut one).is_err() {
            return;
        }
        if one[0] != b'\n' {
            let mut discard: Vec<u8> = Vec::new();
            match rd.read_until(b'\n', &mut discard) {
                Ok(n) => line_start += n as u64,
                Err(_) => return,
            }
        }
    }

    let mut raw: Vec<u8> = Vec::with_capacity(1 << 16);
    let mut cleaned = String::new();

    loop {
        if line_start >= c.end {
            break;
        }
        if let Some(flag) = cancel {
            if flag.load(Ordering::Relaxed) {
                break;
            }
        }
        raw.clear();
        let n = match rd.read_until(b'\n', &mut raw) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        line_start += n as u64;
        totals.lines += 1;
        totals.bytes += n as u64;
        if raw.last() == Some(&b'\n') {
            raw.pop();
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
        }

        match source::extract_row(rule, &raw) {
            Ok(Some(texts)) => {
                for t in texts {
                    totals.paras += 1;
                    let s: &str = if rule.strip_html {
                        cleaned.clear();
                        clean::clean_into(&t, true, &mut cleaned);
                        &cleaned
                    } else {
                        &t
                    };
                    if s.is_empty() {
                        continue;
                    }
                    let counted = tk.count_into(s, counts);
                    totals.tokens += counted.words;
                }
            }
            // 解析失败（含非法 UTF-8 且 lossy 也救不回来）—— 只计坏行，绝不中断
            Ok(None) | Err(_) => totals.bad_lines += 1,
        }
    }
}

/// 扫描一个表组：表组内所有区间并行，计数表每线程独占，最后 fold/reduce 归并。
fn run_domain(
    tk: &Tokenizer,
    rules: &[SourceRule],
    files: &[&FilePlan],
    domain: &str,
    progress: ProgressFn<'_>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> (Totals, LocalCounts) {
    let chunks = build_chunks(files);
    let units_total = chunks.len() as u64;
    let bytes_total: u64 = files.iter().map(|f| f.bytes).sum::<u64>().max(1);
    let units_done = AtomicU64::new(0);
    let bytes_done = AtomicU64::new(0);

    let (mut totals, counts) = chunks
        .par_iter()
        .fold(
            || (Totals::default(), LocalCounts::new()),
            |mut acc, c| {
                scan_chunk(tk, rules, c, &mut acc.0, &mut acc.1, cancel);
                let span = c.end - c.start;
                let done = units_done.fetch_add(1, Ordering::Relaxed) + 1;
                let bd = bytes_done.fetch_add(span, Ordering::Relaxed) + span;
                progress(Progress::Phase {
                    phase: "scan".into(),
                    domain: domain.to_string(),
                    units_done: done,
                    units_total,
                    bytes_done: bd,
                    bytes_total,
                    percent: bd as f64 / bytes_total as f64,
                });
                acc
            },
        )
        .reduce(
            || (Totals::default(), LocalCounts::new()),
            |mut a, b| {
                a.0.add(&b.0);
                a.1.merge(&b.1);
                a
            },
        );

    // 区间数不等于文件数，这里把文件数补回正确值
    totals.files = files.len() as u64;
    (totals, counts)
}

// ---------------------------------------------------------------- 产物写出

struct Written {
    word: TableMeta,
    ch: TableMeta,
}

/// 把一套（词频表 + 字表）写进 `<out_dir>`，返回它们在 `meta.json` 里的描述。
///
/// `scope` 是**表组 id**，同时是磁盘上的目录名（`<out>/<scope>/`）。每个表组
/// 平级、结构相同 —— 全库的 `full` 与表组的 `news` 没有任何区别，这是 schema v3
/// 的核心改动（见 `docs/DESIGN.md` §5.0）。
#[allow(clippy::too_many_arguments)]
fn write_tables(
    out_dir: &Path,
    scope: &str,
    word_entries: &[RankedEntry],
    char_entries: &[RankedEntry],
    word_total: u64,
    char_total: u64,
    min_count: u64,
    write_tsv: bool,
) -> Result<Written> {
    std::fs::create_dir_all(out_dir)?;

    // 阈值按**该表自己的条目数**由前%换算，而不是套一份全局常量：
    // 表组只有几万条，前 0.0026% 是 1 条；全库表几百万条，同一个前%是 98 条。
    let word_pct = rank::default_tier_pct_for("word");
    let char_pct = rank::default_tier_pct_for("char");
    let word_tiers = rank::tiers_from_pct(word_pct, word_entries.len() as u64);
    let char_tiers = rank::tiers_from_pct(char_pct, char_entries.len() as u64);

    let w = artifact::write_vfr(
        &out_dir.join("word.vfr"),
        artifact::KIND_WORD,
        word_entries,
        word_total,
        true,
    )?;
    if write_tsv {
        artifact::write_tsv(
            &out_dir.join("word.tsv"),
            &artifact::table_key(scope, "word"),
            "word",
            word_entries,
            word_total,
            &word_tiers,
        )?;
    }

    let c = artifact::write_vfr(
        &out_dir.join("char.vfr"),
        artifact::KIND_CHAR,
        char_entries,
        char_total,
        false,
    )?;
    if write_tsv {
        artifact::write_tsv(
            &out_dir.join("char.tsv"),
            &artifact::table_key(scope, "char"),
            "char",
            char_entries,
            char_total,
            &char_tiers,
        )?;
    }

    Ok(Written {
        word: TableMeta {
            path: scope.to_string(),
            kind: "word".into(),
            entries: word_entries.len() as u64,
            total_tokens: word_total,
            vfr_bytes: w.file_bytes,
            tier_stats: rank::tier_stats(word_entries, &word_tiers),
            tiers: word_tiers,
            min_count,
            tier_pct: word_pct.to_vec(),
            source_tables: Vec::new(),
        },
        ch: TableMeta {
            path: scope.to_string(),
            kind: "char".into(),
            entries: char_entries.len() as u64,
            total_tokens: char_total,
            vfr_bytes: c.file_bytes,
            tier_stats: rank::tier_stats(char_entries, &char_tiers),
            tiers: char_tiers,
            min_count,
            tier_pct: char_pct.to_vec(),
            source_tables: Vec::new(),
        },
    })
}

/// 过滤低频词并**重新编号**排名，保证 rank 从 1 连续。
fn apply_min_count(entries: Vec<RankedEntry>, min_count: u64) -> Vec<RankedEntry> {
    if min_count <= 1 {
        return entries;
    }
    entries
        .into_iter()
        .filter(|e| e.count >= min_count)
        .enumerate()
        .map(|(i, mut e)| {
            e.rank = (i + 1) as u32;
            e
        })
        .collect()
}

// ---------------------------------------------------------------- 主入口

/// 执行一次完整统计，返回写出的 `meta.json` 内容。
pub fn scan(cfg: &ScanConfig, progress: ProgressFn<'_>) -> Result<Meta> {
    let t_start = Instant::now();
    // 用**局部**线程池而不是 build_global：build_global 一个进程只能成功调用一次，
    // GUI 里第二次改线程数会静默失效。
    let pool = if cfg.threads > 0 {
        Some(
            rayon::ThreadPoolBuilder::new()
                .num_threads(cfg.threads)
                .build()
                .map_err(|e| Error::Other(format!("创建 {}-线程池失败: {e}", cfg.threads)))?,
        )
    } else {
        None
    };
    std::fs::create_dir_all(&cfg.out)?;
    // 「不要表组 + 不要 full」会得到一张表都没有的目录，而 `Dataset::open` 打不开
    // 这种产物（"tables 为空，这个目录是废的"）。前置拦掉，别让用户跑完几十分钟才发现。
    if cfg.skip_domain_tables && !cfg.write_full {
        return Err(Error::Other(
            "配置矛盾：既跳过了各表组的表，又没有要全量表组 `full`，这样产物里一张表都不会有。\
             `full` 现在默认不产出（它由「merge 合流 + compose 相加」得到），\
             想要它请显式打开 `write_full`（CLI：`--full`）。"
                .into(),
        ));
    }
    let log = |level: &str, message: String| {
        progress(Progress::Log {
            level: level.to_string(),
            message,
        })
    };

    // 1) 分词器：整条词典链一次装载
    let tk = Tokenizer::from_dicts(&cfg.dicts, cfg.tok.clone())?;
    for (d, rep) in tk.dicts.iter().zip(tk.dict_reports.iter()) {
        log(
            "info",
            format!(
                "词典 {}：{} 条有效词条（跳过注释 {} 行、空行 {} 行）",
                d.short(),
                rep.entries,
                rep.comments,
                rep.blanks
            ),
        );
        if rep.freq_omitted > 0 {
            log(
                "info",
                format!(
                    "  · 其中 {} 条没写词频，已按 jieba 的 suggest_freq 折算",
                    rep.freq_omitted
                ),
            );
        }
        if rep.freq_zero > 0 {
            log(
                "warn",
                format!(
                    "  · 其中 {} 条把词频显式写成了 0：它们会被登记进词典，但路径概率是 \
                     ln(0) = -inf，**永远不可能被切分出来**，几乎肯定是笔误",
                    rep.freq_zero
                ),
            );
        }
    }

    // 2) 计划
    let rules = cfg.rules.clone().unwrap_or_else(source::builtin_rules);
    let (plans, warnings) = plan_corpus(&cfg.corpus, &rules, &cfg.only_domains)?;

    let mut domain_names: Vec<String> = Vec::new();
    for p in &plans {
        if !domain_names.contains(&p.domain) {
            domain_names.push(p.domain.clone());
        }
    }
    let scope_metas: Vec<SourceScope> = domain_names
        .iter()
        .map(|d| {
            let fs: Vec<&FilePlan> = plans.iter().filter(|p| &p.domain == d).collect();
            SourceScope {
                name: d.clone(),
                files: fs.len() as u64,
                bytes: fs.iter().map(|f| f.bytes).sum(),
            }
        })
        .collect();

    let total_bytes: u64 = plans.iter().map(|p| p.bytes).sum();
    progress(Progress::Plan {
        files: plans.len() as u64,
        bytes: total_bytes,
        domains: scope_metas.clone(),
        warnings: warnings.clone(),
    });
    for w in &warnings {
        log("warn", w.clone());
    }
    log(
        "info",
        format!(
            "发现 {} 个文件 / {:.2} GB，{} 个表组：{}",
            plans.len(),
            total_bytes as f64 / 1e9,
            scope_metas.len(),
            scope_metas
                .iter()
                .map(|d| format!("{}={}", d.name, d.files))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
    log(
        "info",
        format!(
            "分词器：{}  HMM={}  线程={}",
            tk.dict_label(),
            cfg.tok.hmm,
            rayon::current_num_threads()
        ),
    );

    // 3) 逐表组扫描 + 落盘
    let mut overall = GlobalCounts::new();
    let mut overall_totals = Totals::default();
    let mut table_metas: Vec<TableMeta> = Vec::new();
    // 默认不产 `full`，词典外候选词就得从**某个表组**里筛。这里留着迄今条目最多的
    // 那一份（它是全库最有代表性的子集：候选词本来就是"值得回填词典的高频新词"，
    // 按一个足够大的表组筛比按几千条的表组筛更有意义）。
    let mut oov_source: Option<Vec<RankedEntry>> = None;

    for d in &scope_metas {
        if cancelled(cfg) {
            invalidate(&cfg.out);
            return Err(Error::Other("已取消".into()));
        }
        let fs: Vec<&FilePlan> = plans.iter().filter(|p| p.domain == d.name).collect();
        let t0 = Instant::now();
        let cancel_ref = cfg.cancel.as_deref();
        let job = || run_domain(&tk, &rules, &fs, &d.name, progress, cancel_ref);
        let (tot, mut counts) = match &pool {
            Some(p) => p.install(job),
            None => job(),
        };
        // 区间扫描可能是在中途被取消的，此时这份计数是残缺的，不能再落盘
        if cancelled(cfg) {
            invalidate(&cfg.out);
            return Err(Error::Other("已取消".into()));
        }

        let word_total: u64 = counts.words.values().sum();
        let char_total = counts.char_total();
        let uniq_words = counts.words.len();
        let uniq_chars = counts.char_entries().len();

        // 先把本表组并入全库总表（此时 counts 还能被借用），再排行落盘（会消费 counts）。
        // 顺序不能反：rank_entries_owned 会把词频表搬空。
        let t_merge = Instant::now();
        overall.merge(&counts);
        overall_totals.add(&tot);
        let merge_ms = t_merge.elapsed().as_millis();

        let mut rank_ms = 0u128;
        let mut write_ms = 0u128;
        if !cfg.skip_domain_tables {
            let t_rank = Instant::now();
            // 顺序要紧：char_entries() 借的是整个 counts，必须在 words 被 take 走之前算。
            let chars = rank::rank_chars(counts.char_entries());
            let words = apply_min_count(
                rank::rank_entries_owned(std::mem::take(&mut counts.words), &|w| tk.flags(w)),
                cfg.min_count,
            );
            rank_ms = t_rank.elapsed().as_millis();

            let t_write = Instant::now();
            // 表组就是目录名：`<out>/news/`，与 `<out>/full/` 完全平级
            let w = write_tables(
                &cfg.out.join(&d.name),
                &d.name,
                &words,
                &chars,
                word_total,
                char_total,
                cfg.min_count,
                cfg.write_tsv,
            )?;
            write_ms = t_write.elapsed().as_millis();
            for t in [w.word, w.ch] {
                progress(Progress::Table {
                    table: t.key(),
                    entries: t.entries,
                    total_tokens: t.total_tokens,
                    tier_stats: t.tier_stats.clone(),
                });
                table_metas.push(t);
            }
            // 留着当词典外候选词的来源（见上面 `oov_source` 的说明）
            if cfg.oov_min_count > 0 && oov_source.as_ref().is_none_or(|b| b.len() < words.len()) {
                oov_source = Some(words);
            }
        }

        log(
            "info",
            format!(
                "表组 {}: {:.2} GB / {} token / {} 词 / {} 字 | 扫描 {:.1}s 排行 {:.1}s 落盘 {:.1}s 归并 {:.1}s",
                d.name,
                tot.bytes as f64 / 1e9,
                tot.tokens,
                uniq_words,
                uniq_chars,
                t0.elapsed().as_secs_f64()
                    - (rank_ms + write_ms) as f64 / 1000.0
                    - merge_ms as f64 / 1000.0,
                rank_ms as f64 / 1000.0,
                write_ms as f64 / 1000.0,
                merge_ms as f64 / 1000.0,
            ),
        );
    }

    // 4) 全量表组（默认**不产**，见 [`ScanConfig::write_full`]）。
    //
    //    它现在只是一个**验证基准**：一次性全量扫一遍、和各表组相加的结果逐条比对。
    //    常规工作流是「表组扫成几份产物 → merge 合流 → compose 相加出 full」，
    //    在那种工作流里每次 scan 都写一遍 full 纯是浪费（而且因为整体覆盖，
    //    前几次写的会被后一次冲掉）。
    let oov_entries: Vec<RankedEntry> = if cfg.write_full {
        let full_tokens: u64 = overall.words.values().sum();
        let full_chars = overall.chars.char_total();
        let t_rank = Instant::now();
        let full_words = apply_min_count(
            rank::rank_entries_owned(std::mem::take(&mut overall.words), &|w| tk.flags(w)),
            cfg.min_count,
        );
        let full_char_entries = rank::rank_chars(overall.chars.char_entries());
        let full_rank_ms = t_rank.elapsed().as_millis();

        let t_write = Instant::now();
        let w = write_tables(
            &cfg.out.join(artifact::SCOPE_FULL),
            artifact::SCOPE_FULL,
            &full_words,
            &full_char_entries,
            full_tokens,
            full_chars,
            cfg.min_count,
            cfg.write_tsv,
        )?;
        let full_write_ms = t_write.elapsed().as_millis();
        log(
            "info",
            format!(
                "全量表组（full）：{} 词 / {} 字 | 排行 {:.1}s 落盘 {:.1}s",
                full_words.len(),
                full_char_entries.len(),
                full_rank_ms as f64 / 1000.0,
                full_write_ms as f64 / 1000.0
            ),
        );
        for t in [w.word, w.ch] {
            progress(Progress::Table {
                table: t.key(),
                entries: t.entries,
                total_tokens: t.total_tokens,
                tier_stats: t.tier_stats.clone(),
            });
            table_metas.push(t);
        }
        full_words
    } else {
        log(
            "info",
            "按默认口径跳过全量表组 full：它由「合流各表组产物 + 把表组相加」得到\
             （`vocfreq merge` 然后 `vocfreq compose --scope full`）"
                .to_string(),
        );
        oov_source.take().unwrap_or_default()
    };

    // 5) 词典外候选词（关闭 HMM 时它们会被切成碎片，回填 user-dict 后即可成词）
    //
    // 必须限定「含汉字」：论坛语料里的 HTML 与 URL 会切出 https / com / www / chksm
    // 这类 ASCII 碎片，它们的频次极高，不加过滤会把整张候选榜淹掉，真正有价值的
    // 中文新词一个都露不出来。
    let cands: Vec<(Box<str>, u64)> = oov_entries
        .iter()
        .filter(|e| {
            e.count >= cfg.oov_min_count
                && e.word.chars().count() >= cfg.oov_min_len
                && !e.in_dict()
                && e.word.chars().any(crate::clean::is_cjk_ideograph)
        })
        .take(cfg.oov_limit)
        .map(|e| (e.word.clone(), e.count))
        .collect();
    if !cands.is_empty() {
        artifact::write_oov(&cfg.out.join("oov_candidates.tsv"), &cands)?;
        log("info", format!("导出词典外候选词 {} 条", cands.len()));
    }

    // 6) meta.json
    let elapsed_ms = t_start.elapsed().as_millis() as u64;
    let meta = Meta {
        schema_version: SCHEMA_VERSION,
        generated_at: artifact::iso8601_now(),
        tool_version: VERSION.to_string(),
        corpus_root: cfg.corpus.display().to_string(),
        elapsed_ms,
        tokenizer: TokenizerMeta {
            engine: "jieba-rs".into(),
            version: "0.11".into(),
            hmm: cfg.tok.hmm,
            dicts: tk.dicts.clone(),
            // v1 的两个兼容字段只读不写
            legacy_dict: None,
            legacy_user_dict: None,
            min_len: cfg.tok.min_len,
            max_len: cfg.tok.max_len,
            keep_latin: cfg.tok.keep_latin,
            keep_digit: cfg.tok.keep_digit,
            skip_single_char: cfg.tok.skip_single_char,
        },
        totals: overall_totals,
        domains: scope_metas,
        tables: table_metas,
        tier_names: rank::TIER_NAMES.iter().map(|s| s.to_string()).collect(),
        tier_keys: rank::TIER_KEYS.iter().map(|s| s.to_string()).collect(),
    };
    artifact::write_meta(&cfg.out.join("meta.json"), &meta)?;

    progress(Progress::Done {
        elapsed_ms,
        out: cfg.out.display().to_string(),
    });
    log(
        "info",
        format!(
            "完成：用时 {:.1}s，产物目录 {}",
            elapsed_ms as f64 / 1000.0,
            cfg.out.display()
        ),
    );
    Ok(meta)
}

/// 是否已被请求取消。
#[inline]
fn cancelled(cfg: &ScanConfig) -> bool {
    cfg.cancel
        .as_ref()
        .is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed))
}

/// 取消时删掉 meta.json，让产物目录明确处于「不完整」状态。
///
/// 否则半成品目录配上一次成功运行残留的 meta.json，`Dataset::open` 会成功打开一张
/// 与实际数据不符的表——那比直接打不开更危险。
fn invalidate(out: &Path) {
    let _ = std::fs::remove_file(out.join("meta.json"));
}

/// 供 `detect` 子命令使用：只探测每个文件的格式，不做任何统计。
pub fn detect_report(corpus: &Path, rules: &[SourceRule]) -> Result<Vec<(String, String, u64)>> {
    let (plans, _warnings) = plan_corpus(corpus, rules, &[])?;
    Ok(plans
        .into_iter()
        .map(|p| {
            (
                p.path
                    .strip_prefix(corpus)
                    .unwrap_or(&p.path)
                    .to_string_lossy()
                    .to_string(),
                rules[p.rule].name.clone(),
                p.bytes,
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches() {
        assert!(simple_glob_match("*/forum/*", "E:/c/forum/a.jsonl"));
        assert!(simple_glob_match("*.jsonl", "a/b/c.jsonl"));
        assert!(simple_glob_match("a?c", "abc"));
        assert!(!simple_glob_match("a?c", "abbc"));
        assert!(!simple_glob_match("*.json", "a.jsonl"));
        assert!(simple_glob_match("*", "anything"));
    }

    #[test]
    fn min_count_filter_renumbers_ranks() {
        let entries: Vec<RankedEntry> = [10u64, 7, 4, 1]
            .iter()
            .enumerate()
            .map(|(i, c)| RankedEntry {
                word: format!("w{i}").into(),
                count: *c,
                rank: (i + 1) as u32,
                flags: 0,
            })
            .collect();
        let got = apply_min_count(entries, 5);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].rank, 1);
        assert_eq!(got[1].rank, 2);
        assert_eq!(got[0].count, 10);
        assert_eq!(got[1].count, 7);
    }

    #[test]
    fn rejects_skip_domain_tables_without_full() {
        // 「不要表组 + 不要 full」= 一张表都没有。必须在动手扫描前就报错，
        // 而不是产出一份 `Dataset::open` 打不开的废目录。
        let out = std::env::temp_dir().join(format!("vocfreq_scan_reject_{}", std::process::id()));
        let mut cfg = ScanConfig::new("corpus-不存在也无所谓", &out);
        cfg.skip_domain_tables = true;
        cfg.write_full = false;
        let e = scan(&cfg, &|_| {}).expect_err("这种组合必须被拒绝");
        assert!(e.to_string().contains("配置矛盾"), "{e}");
    }

    #[test]
    fn chunks_cover_whole_file_without_gaps() {
        let plans = [FilePlan {
            path: PathBuf::from("x.jsonl"),
            domain: "d".into(),
            bytes: 100 * 1024 * 1024 + 7,
            rule: 0,
        }];
        let refs: Vec<&FilePlan> = plans.iter().collect();
        let chunks = build_chunks(&refs);
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks.last().unwrap().end, plans[0].bytes);
        for w in chunks.windows(2) {
            assert_eq!(w[0].end, w[1].start, "区间之间不能有空隙或重叠");
        }
    }

    #[test]
    fn find_bytes_locates_anchor() {
        assert_eq!(
            find_bytes(
                r#"{"段落":[{"内容":"x"}]}"#.as_bytes(),
                r#""段落""#.as_bytes()
            ),
            Some(1)
        );
        assert_eq!(find_bytes(b"abc", b"zz"), None);
    }
}
