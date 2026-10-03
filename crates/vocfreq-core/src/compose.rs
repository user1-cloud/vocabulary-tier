//! 词频表**相加**：把若干张表合成一张新表。
//!
//! ## 为什么这在数学上是精确的
//!
//! `scan` 本身就是**逐表组扫完、再累加进全量表组**（见 `scan.rs` 的
//! `overall.merge(&counts)`）。每个 token 只属于一个表组，所以
//!
//! ```text
//! full.count(词) == Σ 各表组.count(词)          （逐条相等，不是近似）
//! full.total_tokens == Σ 各表组.total_tokens
//! ```
//!
//! `compose_equal_to_full_scan` 这条测试就是拿扫描产出的表组相加、与扫描产出的
//! `full` 表逐条比对来钉这件事的。
//!
//! 前提是**低频过滤阈值要跟着走**：源表各自按 `min_count` 丢过一批词，相加方必须
//! 丢掉同一批（取各源的最大值），重编号后的排名才会与"全量重扫一次"的结果一致。
//!
//! ## 源表可以来自多份产物
//!
//! 全量语料大到一块盘放不下时，只能一份产物一个表组地扫，最后再合起来（见
//! [`crate::merge`]）。所以相加的源表**允许跨产物**，代价是必须先过一道硬门槛：
//! 所有源产物的词典链与分词口径必须**完全一致**（[`check_mergeable`]）。
//!
//! 反过来，相加**永远不合并词典链** —— 不一致就拒绝，绝不静默凑一份。
//!
//! ## 落地方式
//!
//! 物化成一份真实的 `.vfr`（几秒），而不是"只记一条 A+B+C 的标记、读时跨表归并"。
//! 理由：排行榜要按排名翻页、覆盖率曲线要顺序读遍全表，虚拟合成会让这两件事没法
//! 靠排名索引 O(1) 完成。磁盘代价是**一份表大小**（源表不需要了可以删掉回收），
//! 而 `TableMeta::source_tables` 仍然把来源记下来 —— 将来真要改成虚拟合成，
//! 那个字段就是现成的接口（也是 [`crate::query::TableReader`] 存在的理由）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::artifact::{self, Meta, SCHEMA_VERSION, SourceScope, TableMeta, TokenizerMeta};
use crate::dict::DictRef;
use crate::query::{Dataset, KIND_CHAR, KIND_WORD, TableRef};
use crate::rank::{self, RankedEntry};
use crate::{Error, Result};

/// 一次相加请求。
#[derive(Debug, Clone)]
pub struct ComposeSpec {
    /// 源产物目录（新表组默认也写回这里）
    pub from: std::path::PathBuf,
    /// **额外的**源产物目录。
    ///
    /// 全量语料大到一块盘放不下时，只能一份产物一个表组地扫，最后再把几份产物里的
    /// 表组加在一起 —— 所以相加允许跨产物。代价是必须先过 [`check_mergeable`]
    /// 那道硬门槛：全部产物（含 [`Self::from`]）的词典链与分词口径必须完全一致。
    pub products: Vec<std::path::PathBuf>,
    /// 源表身份，形如 `full/word`、`news/char`
    pub sources: Vec<String>,
    /// 新表组名（会变成目录名，内部会洗一遍）
    pub scope: String,
    /// 只要这一类；`None` = 源表里出现过的每一类都相加
    pub kind: Option<String>,
    /// 输出目录。`None` = 写回 `from`（新表组成为同一张产物里的另一张表）
    pub out: Option<std::path::PathBuf>,
}

/// 相加结果，供 CLI 打印与桌面端反馈。
#[derive(Debug, Clone, Serialize)]
pub struct ComposedTable {
    pub out: String,
    pub scope: String,
    pub kind: String,
    pub entries: u64,
    pub total_tokens: u64,
    pub bytes: u64,
    /// 这些表是从哪几张表加出来的
    pub sources: Vec<String>,
    pub elapsed_ms: u64,
}

/// 相加时**只搬运、不解码**的阈值。
///
/// 留作将来优化：源表比这个还小的时候，整文件拷贝比"解码一遍再编码一遍"更快，
/// 而且不会因为编解码往返引入任何差异（当前所有相加都走解码路径，
/// 380 万词的表实测也在秒级）。
pub const COPY_AS_IS_LIMIT: u64 = 8 * 1024 * 1024;

/// 把若干词频表按词相加。
///
/// 标记位（`in_dict` / `from_user`）取**或**：它们本来是分词器的性质而不是语料的
/// 性质，跨产物相加时也已经校验过词典链一致，或起来只是为了不丢位。
fn merge_words(
    table_sources: &BTreeMap<String, (usize, &TableRef)>,
    sources: &[String],
    min_count: u64,
) -> Result<Vec<RankedEntry>> {
    // 一趟扫完：同一个词在多个表里出现时累加频次、或上标记位。
    let mut acc: FxHashMap<Box<str>, (u64, u8)> = FxHashMap::default();
    for key in sources {
        let (_, t) = table_of(table_sources, key)?;
        if t.kind != "word" {
            return Err(Error::Other(format!(
                "{key} 是 {} 表，不能和词频表相加",
                t.kind
            )));
        }
        for h in t.vfr.positions() {
            let e = acc.entry(h.word.into_boxed_str()).or_insert((0, 0));
            // 用 saturating_add：同一批语料被扫两遍之后相加是真能溢出的，
            // 回绕会静默产生一个"极少"的假象，宁可顶在上限上。
            e.0 = e.0.saturating_add(h.count);
            e.1 |= h.flags;
        }
    }
    let mut v: Vec<(Box<str>, u64, u8)> = acc.into_iter().map(|(w, (c, f))| (w, c, f)).collect();
    v.sort_unstable_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });
    Ok(v.into_iter()
        .filter(|(_, c, _)| *c >= min_count)
        .enumerate()
        .map(|(i, (word, count, flags))| RankedEntry {
            word,
            count,
            rank: (i + 1) as u32,
            flags,
        })
        .collect())
}

/// 把若干字表按字相加。
fn merge_chars(
    table_sources: &BTreeMap<String, (usize, &TableRef)>,
    sources: &[String],
    min_count: u64,
) -> Result<Vec<RankedEntry>> {
    let mut acc: FxHashMap<Box<str>, u64> = FxHashMap::default();
    for key in sources {
        let (_, t) = table_of(table_sources, key)?;
        if t.kind != "char" {
            return Err(Error::Other(format!(
                "{key} 是 {} 表，不能和字表相加",
                t.kind
            )));
        }
        for h in t.vfr.positions() {
            let e = acc.entry(h.word.into_boxed_str()).or_insert(0);
            *e = e.saturating_add(h.count);
        }
    }
    let mut v: Vec<(Box<str>, u64)> = acc.into_iter().collect();
    // 字表同样按频次降序、同频次按字节序，与 `scan::rank_chars` 的口径一致
    v.sort_unstable_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });
    Ok(v.into_iter()
        .filter(|(_, c)| *c >= min_count)
        .enumerate()
        .map(|(i, (word, count))| RankedEntry {
            word,
            count,
            rank: (i + 1) as u32,
            flags: 0,
        })
        .collect())
}

/// 按表身份取「它属于哪份产物 + 那张表」。
fn table_of<'a>(
    table_sources: &BTreeMap<String, (usize, &'a TableRef)>,
    key: &str,
) -> Result<(usize, &'a TableRef)> {
    table_sources
        .get(key)
        .copied()
        .ok_or_else(|| Error::Other(format!("找不到表 {key}")))
}

// ===========================================================================
// 跨产物合流的前置门槛（`merge` 与「多源相加」共用同一套判定）
// ===========================================================================

/// **硬门槛**：这些产物能不能放在一起。
///
/// 只做一件事 —— 逐字段校验 [`TokenizerMeta`]（词典链指纹 + 分词口径）在所有产物里
/// **完全一致**，外加 `engine` / `version`。不一致就返回一条**指出哪一份产物、
/// 哪个字段**不同的错误，绝不"挑一份当基准"。
///
/// 为什么这是硬门槛：频次是**同一套切分规则下的计数**。跨产物相加/合流时若词典或
/// 分词口径不同，同一个词在不同源表里的频次来自不同的切分，加出来的排名自相矛盾，
/// 而且没有任何办法事后看出来。宁可拒绝。
///
/// 反过来，这些**不是**门槛（不影响正确性，只影响展示）：
/// `min_count`（每张表自己记着，相加时取最大）、`tiers` / `tier_stats` / `tier_pct`
/// （各表条目数不同，本来就该不同）、`tier_names` / `tier_keys`（展示名，由本程序的
/// `rank::TIER_*` 常量决定，新产物必然一致）、`corpus_root` / `generated_at`（合流
/// 本来就是把几块语料并起来）。
pub fn check_mergeable(products: &[&Dataset], labels: &[String]) -> Result<()> {
    debug_assert_eq!(products.len(), labels.len());
    // 先单独判"没有指纹"这一种：`dicts` 为空（schema v1 老产物）时逐份比对会得到
    // "两份都是空的、看起来一致"，而那是**假一致** —— 没有指纹就无从知道这两份产物
    // 当年用的是不是同一份词典。合流的正确性完全依赖这件事，所以必须拒绝。
    let unverifiable: Vec<&str> = products
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            let ds = d.meta.tokenizer.resolved_dicts();
            ds.is_empty() || !ds.iter().any(DictRef::is_verifiable)
        })
        .map(|(i, _)| labels[i].as_str())
        .collect();
    if !unverifiable.is_empty() {
        return Err(Error::Other(format!(
            "拒绝合流：这些产物**没有可校验的词典指纹**，无从确认它们用的是不是同一份词典。\n  {}\n\n\
             `meta.json` 的 `tokenizer.dicts[]` 必须逐份带 `sha256`。没有指纹时「两份都是空的」\
             看着一致，其实什么也没说明 —— 合流的正确性完全依赖「同一份词典、同一套分词口径」。\n\
             请用带指纹的词典重扫这些语料，再合流。",
            unverifiable.join("\n  ")
        )));
    }
    let base = products[0].meta.tokenizer.clone();
    let mut diffs: Vec<String> = Vec::new();
    for (i, ds) in products.iter().enumerate().skip(1) {
        diffs.extend(tokenizer_diffs(&base, i, &ds.meta.tokenizer));
    }
    if diffs.is_empty() {
        return Ok(());
    }
    Err(Error::Other(format!(
        "拒绝合流：这些产物的词典链 / 分词口径不一致。\n  {}\n\n\
         频次是同一套切分规则下的计数。跨产物合计时若词典或分词口径不同，\
         同一个词的频次会来自不同的切分，排名自相矛盾且事后无从察觉 —— 所以这里直接拒绝，\
         不会挑一份当基准、也不会静默合并。\n\
         请用同一份词典、同一套分词选项重扫，或者只合流一致的那几份。",
        diffs.join("\n  ")
    )))
}

/// 一份分词口径与基准之间**全部**不同之处（显示用；空 = 完全一致）。
fn tokenizer_diffs(base: &TokenizerMeta, i: usize, other: &TokenizerMeta) -> Vec<String> {
    let mut d = tok_engine_diffs(base, i, other);
    d.extend(tok_switch_diffs(base, i, other));
    d.extend(dict_chain_diffs(base, i, other));
    d
}

/// 分词口径里一个字段的名字与当前值。显示用，不是稳定标识。
fn tok_field(
    base: &TokenizerMeta,
    i: usize,
    other: &TokenizerMeta,
    name: &str,
    f: impl Fn(&TokenizerMeta) -> String,
) -> String {
    let (a, b) = (f(base), f(other));
    format!("分词口径 {name}: 第 1 份是 {a}；第 {} 份是 {b}", i + 1)
}

/// 比较两份分词口径里那几个**开关与长度**。
fn tok_switch_diffs(base: &TokenizerMeta, i: usize, other: &TokenizerMeta) -> Vec<String> {
    let mut d = Vec::new();
    macro_rules! cmp {
        ($($name:literal => $field:ident),+ $(,)?) => {
            $(if base.$field != other.$field {
                d.push(tok_field(base, i, other, $name, |t| t.$field.to_string()));
            })+
        };
    }
    cmp!(
        "hmm" => hmm,
        "min_len" => min_len,
        "max_len" => max_len,
        "keep_latin" => keep_latin,
        "keep_digit" => keep_digit,
        "skip_single_char" => skip_single_char,
    );
    d
}

/// 比较分词引擎：引擎不同就根本不是同一套切分规则，版本不同则按明确的版本差拒绝。
fn tok_engine_diffs(base: &TokenizerMeta, i: usize, other: &TokenizerMeta) -> Vec<String> {
    let mut d = Vec::new();
    if base.engine != other.engine {
        d.push(tok_field(base, i, other, "engine", |t| t.engine.clone()));
    }
    if base.version != other.version {
        d.push(tok_field(base, i, other, "version", |t| t.version.clone()));
    }
    d
}

/// 比较整条词典链：**逐份、按内容指纹**。
///
/// 指纹是唯一可靠的判据 —— `path` 换台机器就失效（出厂预置表刻意不记路径），
/// `name` 是用户随手起的。之所以要逐份比而不是比一个"链的总指纹"：两份不同内容的
/// 词典有可能被用户摆成同一个链，那样总指纹更不容易看出来（实测里就发生过）。
fn dict_chain_diffs(base: &TokenizerMeta, i: usize, other: &TokenizerMeta) -> Vec<String> {
    let (a, b) = (base.resolved_dicts(), other.resolved_dicts());
    if a.len() != b.len() {
        return vec![format!(
            "词典链长度: 第 1 份是 {} 份（{}）；第 {} 份是 {} 份（{}）",
            a.len(),
            dict_chain_label(&a),
            i + 1,
            b.len(),
            dict_chain_label(&b)
        )];
    }
    let mut d = Vec::new();
    for (k, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if let Some(why) = dict_pair_mismatch(x, y) {
            d.push(format!(
                "词典链第 {} 份（第 1 份「{}」/ 第 {} 份「{}」）: {why}",
                k + 1,
                x.name,
                i + 1,
                y.name
            ));
        }
    }
    d
}

/// 两份词典记录是否指向同一份内容。`Some(原因)` = 对不上或不可判定。
fn dict_pair_mismatch(x: &DictRef, y: &DictRef) -> Option<String> {
    match x.same_content(y) {
        // 指纹（sha256）逐字节相等 —— 这才是"同一份词典"
        Some(true) => None,
        Some(false) => Some(format!(
            "内容指纹不同：{} vs {}",
            short_sha(&x.sha256),
            short_sha(&y.sha256)
        )),
        // 没有指纹 = 无从校验。这不是"一致"，必须显式拒绝：
        // 拿两份不知道是否相同的词典合流，等于把正确性赌在运气上。
        None => Some(format!(
            "没有内容指纹，无法校验两份词典是不是同一份（{} vs {}）。\
             请用同一份词典重扫一遍再合流",
            sha_label(x),
            sha_label(y)
        )),
    }
}

fn sha_label(d: &DictRef) -> String {
    if d.is_verifiable() {
        short_sha(&d.sha256)
    } else {
        "无指纹".into()
    }
}

fn dict_chain_label(ds: &[DictRef]) -> String {
    if ds.is_empty() {
        return "空（没有记录任何词典）".into();
    }
    ds.iter()
        .map(|d| d.name.clone())
        .collect::<Vec<_>>()
        .join(" + ")
}

fn short_sha(s: &str) -> String {
    if s.is_empty() {
        return "无指纹".into();
    }
    format!("{}…", s.chars().take(12).collect::<String>())
}

/// 全部源产物的分词口径是否一致（不一致就返回那条详细错误）。
///
/// `from` 里可以有**多份**产物：跨产物相加必须先过这一关。只有一份时恒定通过。
fn ensure_same_tokenizer(products: &[&Dataset], labels: &[String]) -> Result<()> {
    check_mergeable(products, labels)
}

/// 给一条错误消息用的产物短描述：目录名 + 工具版本 + 表数。
pub fn describe_product(name: &str, ds: &Dataset) -> String {
    format!(
        "{}（vocfreq {} / {} 张表 / 生成于 {}）",
        name,
        ds.meta.tool_version,
        ds.meta.tables.len(),
        ds.meta.generated_at
    )
}

// ===========================================================================
// 相加
// ===========================================================================

/// 执行相加。返回写出的每一类表。
pub fn compose(spec: &ComposeSpec) -> Result<Vec<ComposedTable>> {
    let t0 = Instant::now();
    if spec.sources.is_empty() {
        return Err(Error::Other("没有选择要相加的表".into()));
    }
    let scope = artifact::sanitize_scope(&spec.scope);
    if scope.is_empty() {
        return Err(Error::Other("表组名不能为空".into()));
    }

    // 源产物：`from` 是那个"源产物目录"，`products` 是**额外**的产物目录(可空)。
    // 全量语料大到一块盘放不下时只能一份产物一个表组地扫，所以跨产物相加是常规路径，
    // 不是例外 —— 但必须先过词典链/分词口径那一关。
    let mut roots: Vec<PathBuf> = vec![spec.from.clone()];
    for p in &spec.products {
        if !roots.contains(p) {
            roots.push(p.clone());
        }
    }
    let mut opened: Vec<Dataset> = Vec::with_capacity(roots.len());
    for r in &roots {
        opened.push(
            Dataset::open(r)
                .map_err(|e| Error::Format(format!("打不开源产物 {}：{e}", r.display())))?,
        );
    }
    let products: Vec<&Dataset> = opened.iter().collect();
    let labels: Vec<String> = roots
        .iter()
        .zip(products.iter())
        .map(|(r, d)| describe_product(&r.display().to_string(), d))
        .collect();
    // **硬门槛**：词典链 + 分词口径必须完全一致。不一致就到此为止，绝不出半截产物。
    ensure_same_tokenizer(&products, &labels)?;
    let table_sources = table_sources(&products)?;
    let ds = first_ds(&products);

    // 目标目录：`out` 不给就写回第一份源产物（新表组成为同一份产物里的另一张表）
    let out_root = spec.out.clone().unwrap_or_else(|| roots[0].clone());
    let out_dir = out_root.join(&scope);
    if out_dir.exists() {
        // 不覆盖：与 `library::import_dict` 的"永不覆盖"同一条原则。
        // 相加是一次可能跑几分钟、写几百 MB 的操作，悄悄盖掉用户已有的表代价太大。
        return Err(Error::Other(format!(
            "{} 已经存在了。换一个表组名，或先把旧的那张删掉。",
            out_dir.display()
        )));
    }
    // `Dataset::open` 已经校验过源产物自身的完整性；这里再挡一次"目标其实就是某份源
    // 产物"的写法 —— 那种情况下先把源表整目录铺过去会撞上自己的读端。
    if roots.iter().any(|r| r == &out_root) && out_root != roots[0] {
        return Err(Error::Other(format!(
            "目标目录 {} 是源产物之一，不能往里铺源表。请换一个输出目录。",
            out_root.display()
        )));
    }
    if products
        .iter()
        .any(|p| p.meta.table(&scope, "word").is_some() || p.meta.table(&scope, "char").is_some())
    {
        return Err(Error::Other(format!(
            "表组「{scope}」在这些产物里已经有一张表了，换一个名字"
        )));
    }

    // 选中的表：按 kind 分组，同时做四件事 —— 校验存在、拒绝重复、拒绝撞名、算出要产出几类。
    let mut by_kind: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut seen: Vec<String> = Vec::new();
    for s in &spec.sources {
        let (src_scope, src_kind) = artifact::split_table_key(s);
        let key = artifact::table_key(&src_scope, &src_kind);
        let (origin, _) = source_of(&table_sources, &src_scope, &src_kind).ok_or_else(|| {
            Error::Other(format!(
                "找不到表 {s}；这些产物里可用的是：{}",
                available(&products)
            ))
        })?;
        if let Some(want) = &spec.kind {
            if &src_kind != want {
                continue;
            }
        }
        let key = ds
            .meta
            .table(&src_scope, &src_kind)
            .map(|t| t.key())
            .unwrap_or(key);
        if seen.contains(&key) {
            return Err(Error::Other(format!(
                "表 {key} 被选了两次（它在 {} 里）",
                labels[origin]
            )));
        }
        seen.push(key.clone());
        by_kind.entry(src_kind).or_default().push(key);
    }
    if seen.is_empty() {
        return Err(Error::Other(match &spec.kind {
            Some(k) => format!("选中的表里没有一张是 {k} 类"),
            None => "没有可相加的表".into(),
        }));
    }

    // 低频过滤阈值：取各源表的最大值。源表各自按自己的 `min_count` 丢过一批词，
    // 相加方必须丢掉同一批，重编号后的排名才会与"全量重扫一次"逐条一致。
    let min_count = seen
        .iter()
        .map(|k| artifact::split_table_key(k))
        .filter_map(|(s, kind)| ds.meta.table(&s, &kind).map(|t| t.min_count))
        .max()
        .unwrap_or(1);

    // 写去别的目录时，先把源产物（meta + 全部表）搬过去：只搬新加出来的那一张，
    // 得到的是一个"有表却查不到词"的半截产物（它引用的其它表都不在）。
    //
    // ⚠ 顺序是「先搬、再建新表组目录」。反过来会在新表组目录里塞进源表的
    // `.vfr`（`copy_product` 是整目录铺的），虽然紧接着就会被覆写，但那几秒里
    // 目录处于自相矛盾的状态，进程被中断时留下的就是一份坏产物。
    let copying = roots.iter().any(|r| r != &out_root);
    if copying {
        std::fs::create_dir_all(&out_root)?;
        // 先把「并起来」的 meta 写下去：`copy_product` 会把全部源表铺进目标目录，
        // 这一步写完之前它不是一份完整产物；写完再补上相加出来的那一张。
        artifact::write_meta(&out_root.join("meta.json"), &merge_meta(&opened))?;
        copy_product(&opened, &table_sources, &out_root)?;
    }
    std::fs::create_dir_all(&out_dir)?;
    let mut written: Vec<ComposedTable> = Vec::new();

    for (kind, sources) in &by_kind {
        let (entries, total_tokens, bytes) = match kind.as_str() {
            "word" => {
                let merged = merge_words(&table_sources, sources, min_count)?;
                let total: u64 = merged.iter().map(|e| e.count).sum();
                let w = artifact::write_vfr(
                    &out_dir.join("word.vfr"),
                    KIND_WORD,
                    &merged,
                    total,
                    true,
                )?;
                (w.entries, total, w.file_bytes)
            }
            "char" => {
                let merged = merge_chars(&table_sources, sources, min_count)?;
                let total: u64 = merged.iter().map(|e| e.count).sum();
                let w = artifact::write_vfr(
                    &out_dir.join("char.vfr"),
                    KIND_CHAR,
                    &merged,
                    total,
                    false,
                )?;
                (w.entries, total, w.file_bytes)
            }
            other => {
                return Err(Error::Other(format!("不认识的表类型 {other}")));
            }
        };
        written.push(ComposedTable {
            out: out_dir.display().to_string(),
            scope: scope.clone(),
            kind: kind.clone(),
            entries,
            total_tokens,
            bytes,
            sources: sources.clone(),
            elapsed_ms: t0.elapsed().as_millis() as u64,
        });
    }

    // 更新 meta.json：把新表组的表挂进去。
    //
    // `domains` / `totals` / `tables` 必须**先合并全部源产物**：跨产物相加时目标目录
    // 里放着几份产物的表，meta 只写第一份的话，别的表就成了"磁盘上有、meta 里没有"
    // 的孤儿（`Dataset::open` 打不开）。已知一致的是词典链与分词口径（上面校验过），
    // 语料切片清单则是**并起来**才算如实描述。
    let mut meta = merge_meta(&opened);
    for c in &written {
        let (tiers, tier_stats) = build_tier_meta(&out_dir, c, &tier_pct_of(&by_kind, &c.kind))?;
        meta.tables
            .retain(|t| !(t.path == c.scope && t.kind == c.kind));
        meta.tables.push(TableMeta {
            path: c.scope.clone(),
            kind: c.kind.clone(),
            entries: c.entries,
            total_tokens: c.total_tokens,
            vfr_bytes: c.bytes,
            tiers,
            tier_stats,
            min_count,
            tier_pct: tier_pct_of(&by_kind, &c.kind),
            source_tables: c.sources.clone(),
        });
    }
    meta.schema_version = SCHEMA_VERSION;
    artifact::write_meta(&out_root.join("meta.json"), &meta)?;

    Ok(written)
}

/// 第一份源产物（兼容旧签名里的那个"源产物"）。
fn first_ds<'a>(products: &[&'a Dataset]) -> &'a Dataset {
    products[0]
}

/// 表身份 → (它属于第几份产物, 它在磁盘上的表)。
///
/// 一份表只能属于一份产物：`Dataset::open` 的校验保证 `meta.tables` 与磁盘上的
/// `.vfr` 一一对应，所以这里不会出现"同一个 key 在两份产物里都有"的歧义
/// ——它正是跨产物合流要挡的那个**表组撞名**，在 [`crate::merge`] 里被拒绝。
fn table_sources<'a>(products: &[&'a Dataset]) -> Result<BTreeMap<String, (usize, &'a TableRef)>> {
    let mut out: BTreeMap<String, (usize, &'a TableRef)> = BTreeMap::new();
    for (i, ds) in products.iter().enumerate() {
        for t in &ds.tables {
            out.entry(t.key()).or_insert((i, t));
        }
    }
    Ok(out)
}

/// 从表身份表里取「它属于哪份产物」。
fn source_of<'a>(
    table_sources: &BTreeMap<String, (usize, &'a TableRef)>,
    scope: &str,
    kind: &str,
) -> Option<(usize, &'a TableRef)> {
    table_sources
        .get(&artifact::table_key(scope, kind))
        .copied()
}

fn available(products: &[&Dataset]) -> String {
    let mut keys: Vec<String> = products
        .iter()
        .flat_map(|d| d.tables.iter().map(|t| t.key()))
        .collect();
    keys.sort();
    keys.dedup();
    keys.join("、")
}
/// 把全部源产物并成一份 meta：`tokenizer` 沿用第一份（已校验一致）、`domains` 按名字
/// 合并计数、`totals` 相加、`tables` 并起来。
///
/// 跨产物相加/合流时目标目录里放着好几份产物的表，meta 必须如实描述它们全部；
/// 只抄第一份的话，别的表就成了"磁盘上有、meta 里没有"的孤儿。语料切片清单
/// （`domains`）描述的是**语料**，几块语料并起来才是实情。
fn merge_meta(opened: &[Dataset]) -> Meta {
    let first = &opened[0].meta;
    let mut domains: Vec<SourceScope> = Vec::new();
    for ds in opened {
        for d in &ds.meta.domains {
            match domains.iter_mut().find(|x| x.name == d.name) {
                Some(x) => {
                    x.files += d.files;
                    x.bytes += d.bytes;
                }
                None => domains.push(d.clone()),
            }
        }
    }
    domains.sort_by(|a, b| a.name.cmp(&b.name));

    let mut totals = crate::Totals::default();
    for ds in opened {
        totals.add(&ds.meta.totals);
    }

    let same_corpus = opened
        .iter()
        .all(|d| d.meta.corpus_root == first.corpus_root);

    Meta {
        schema_version: SCHEMA_VERSION,
        generated_at: artifact::iso8601_now(),
        tool_version: crate::VERSION.to_string(),
        corpus_root: if same_corpus {
            first.corpus_root.clone()
        } else {
            format!("跨 {} 份产物相加/合流", opened.len())
        },
        elapsed_ms: 0,
        tokenizer: first.tokenizer.clone(),
        totals,
        domains,
        tables: opened
            .iter()
            .flat_map(|d| d.meta.tables.iter().cloned())
            .collect(),
        tier_names: rank::TIER_NAMES.iter().map(|s| s.to_string()).collect(),
        tier_keys: rank::TIER_KEYS.iter().map(|s| s.to_string()).collect(),
    }
}

/// 把源产物里**已经存在的**表全部复制到目标目录。
///
/// 相加写去别处时必须做这一步：新表只是同一份产物里的又一个表组，产物本身
/// （`meta.json` + 各表组的 `.vfr`）得先完整存在，否则新表引用的其它表都不在，
/// 那个目录是打不开的。
///
/// `scope/` 已经由调用方判过"不存在"，这里不会碰它。
fn copy_product(
    opened: &[Dataset],
    table_sources: &BTreeMap<String, (usize, &TableRef)>,
    to: &Path,
) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for (key, (pi, t)) in table_sources {
        let (_, kind) = artifact::split_table_key(key);
        let dir = to.join(&t.scope);
        std::fs::create_dir_all(&dir)?;
        for ext in ["vfr", "tsv"] {
            let src = opened[*pi]
                .root
                .join(&t.scope)
                .join(format!("{kind}.{ext}"));
            if src.exists() {
                std::fs::copy(&src, dir.join(format!("{kind}.{ext}")))?;
            }
        }
    }
    Ok(())
}

/// 从 `by_kind` 里取某一类的前%上界。
fn tier_pct_of(by_kind: &BTreeMap<String, Vec<String>>, kind: &str) -> Vec<f64> {
    // 有这一类就按这一类的默认值；没有（不该发生）退回词频表默认值
    if by_kind.contains_key(kind) {
        rank::default_tier_pct_for(kind).to_vec()
    } else {
        rank::default_tier_pct().to_vec()
    }
}

/// 重新算一遍阈值与分档统计（新表的条目数、token 数都变了，必须重算）。
fn build_tier_meta(
    out_dir: &Path,
    c: &ComposedTable,
    pct: &[f64],
) -> Result<(Vec<rank::Tier>, Vec<rank::TierStat>)> {
    let vfr_path = out_dir.join(format!("{}.vfr", c.kind));
    let entries = read_all_entries(&vfr_path)?;
    let tiers = rank::tiers_from_pct(pct, entries.len() as u64);
    let stats = rank::tier_stats(&entries, &tiers);
    Ok((tiers, stats))
}

/// 读出一张已写好的 `.vfr` 的全部条目（按排名序）。
fn read_all_entries(path: &Path) -> Result<Vec<RankedEntry>> {
    let vfr = crate::query::VfrTable::open(path)?;
    let mut hits: Vec<crate::query::Hit> = vfr.positions().collect();
    // `positions()` 给的是词序，排名要按 rank 排回来
    hits.sort_unstable_by_key(|h| h.rank);
    Ok(hits
        .into_iter()
        .map(|h| RankedEntry {
            word: h.word.into_boxed_str(),
            count: h.count,
            rank: h.rank,
            flags: h.flags,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::SourceScope;
    use crate::merge::{MergeSpec, merge};
    use std::path::PathBuf;
    fn tmpdir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("vocfreq_compose_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    // ------------------------------------------------------------ 造数据
    /// 测试用的词典链。指纹是编出来的：合流只比较指纹，不真的去读词典文件。
    fn fixture_dicts() -> Vec<DictRef> {
        vec![
            DictRef {
                id: "主词典".into(),
                name: "主词典".into(),
                path: String::new(),
                entries: 12,
                sha256: "a".repeat(64),
            },
            DictRef {
                id: "叠加词典".into(),
                name: "叠加词典".into(),
                path: String::new(),
                entries: 3,
                sha256: "b".repeat(64),
            },
        ]
    }
    /// 测试用的分词口径。每一处都可以改，用来构造"对不上"的两份产物。
    fn fixture_tokenizer() -> TokenizerMeta {
        serde_json::from_value(serde_json::json!({
            "engine": "jieba-rs", "version": "0.11", "hmm": false,
            "dicts": fixture_dicts(),
            "min_len": 1, "max_len": 64,
            "keep_latin": true, "keep_digit": false, "skip_single_char": false,
        }))
        .unwrap()
    }
    /// 与 `scan::apply_min_count` 同口径：丢掉低频词并**重新编号**，保证 rank 从 1 连续。
    fn rank_filter(entries: Vec<RankedEntry>, min_count: u64) -> Vec<RankedEntry> {
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
    /// 写一个表组的词频表 + 字表，返回 `(词频表条目, 字表条目)`。
    ///
    /// `prefix` 让不同产物里的词**不重叠**（产物 A 的「的」与产物 B 的「的」是两个
    /// 不同的词），这样"分开扫 → 合流 → 相加"如果不是逐条正确地累加，比对必然失败，
    /// 而不是被重叠词掩盖过去。
    fn write_scope(
        root: &Path,
        scope: &str,
        prefix: &str,
        words: &[(&str, u64)],
        min_count: u64,
    ) -> (Vec<RankedEntry>, Vec<RankedEntry>) {
        std::fs::create_dir_all(root.join(scope)).unwrap();
        let m: FxHashMap<Box<str>, u64> = words
            .iter()
            .map(|(w, c)| (format!("{prefix}{w}").into(), *c))
            .collect();
        let entries = rank_filter(rank::rank_entries_owned(m, &|_| 0), min_count);
        let total: u64 = entries.iter().map(|e| e.count).sum();
        artifact::write_vfr(
            &root.join(scope).join("word.vfr"),
            KIND_WORD,
            &entries,
            total,
            true,
        )
        .unwrap();
        // 字表：词的频次平摊到字上（够用就行，测的是相加逻辑不是字频）
        let mut cm: FxHashMap<Box<str>, u64> = FxHashMap::default();
        for (w, c) in words {
            for ch in format!("{prefix}{w}").chars() {
                *cm.entry(ch.to_string().into_boxed_str()).or_insert(0) += c;
            }
        }
        let centries = rank_filter(rank::rank_entries_owned(cm, &|_| 0), min_count);
        let ctotal: u64 = centries.iter().map(|e| e.count).sum();
        artifact::write_vfr(
            &root.join(scope).join("char.vfr"),
            KIND_CHAR,
            &centries,
            ctotal,
            false,
        )
        .unwrap();
        (entries, centries)
    }

    /// 把一份已经算好的排行写进 `<root>/<scope>/<kind>.vfr`。
    fn write_scope_entries(
        root: &Path,
        scope: &str,
        kind: &str,
        entries: &[RankedEntry],
        with_flags: bool,
    ) {
        std::fs::create_dir_all(root.join(scope)).unwrap();
        let total: u64 = entries.iter().map(|e| e.count).sum();
        let vfr_kind = if kind == "char" { KIND_CHAR } else { KIND_WORD };
        artifact::write_vfr(
            &root.join(scope).join(format!("{kind}.vfr")),
            vfr_kind,
            entries,
            total,
            with_flags,
        )
        .unwrap();
    }
    /// 把一张已写好的表登记进 `meta.tables`。
    fn push_table(
        tables: &mut Vec<TableMeta>,
        scope: &str,
        kind: &str,
        entries: &[RankedEntry],
        total_tokens: u64,
        min_count: u64,
    ) {
        tables.push(TableMeta {
            path: scope.to_string(),
            kind: kind.to_string(),
            entries: entries.len() as u64,
            total_tokens,
            vfr_bytes: entries.iter().map(|e| e.word.len() as u64 + 3).sum(),
            tiers: rank::tiers_from_pct(rank::default_tier_pct_for(kind), entries.len() as u64),
            tier_stats: Vec::new(),
            min_count,
            tier_pct: rank::default_tier_pct_for(kind).to_vec(),
            source_tables: Vec::new(),
        });
    }
    /// 造一份"扫描产物"：每个表组各一张词频表 + 字表。
    ///
    /// **不含 `full`** —— 这正是现在 `scan` 的默认行为（`ScanConfig::write_full`
    /// 默认为 false），也是跨产物合流要支持的那种"一份产物只有自己那个表组"的形状。
    fn write_scanned_product(
        root: &Path,
        prefix: &str,
        scopes: &[(&str, &[(&str, u64)])],
        min_count: u64,
    ) -> Meta {
        write_product_with(root, prefix, scopes, min_count, fixture_tokenizer())
    }

    /// 同 [`write_scanned_product`]，但可以换一份分词口径（用来造不一致的产物）。
    fn write_product_with(
        root: &Path,
        prefix: &str,
        scopes: &[(&str, &[(&str, u64)])],
        min_count: u64,
        tokenizer: TokenizerMeta,
    ) -> Meta {
        let mut tables: Vec<TableMeta> = Vec::new();
        for (scope, words) in scopes {
            let (entries, centries) = write_scope(root, scope, prefix, words, min_count);
            let total: u64 = entries.iter().map(|e| e.count).sum();
            let ctotal: u64 = centries.iter().map(|e| e.count).sum();
            push_table(&mut tables, scope, "word", &entries, total, min_count);
            push_table(&mut tables, scope, "char", &centries, ctotal, min_count);
        }
        write_product_meta(root, scopes, tokenizer, tables)
    }

    /// 造一份**带 `full`** 的产物 —— 也就是 `vocfreq scan --full` 那种。
    ///
    /// 它是验收标准 1 的**对照基准**：拿"分开扫 + 合流 + 相加"的结果和它逐条比对。
    fn write_full_product(root: &Path, prefix: &str, scopes: &[(&str, &[(&str, u64)])]) -> Meta {
        let mut meta = write_scanned_product(root, prefix, scopes, 1);
        let ds = Dataset::open(root).unwrap();
        let mut all_words: FxHashMap<Box<str>, u64> = FxHashMap::default();
        let mut all_chars: FxHashMap<Box<str>, u64> = FxHashMap::default();
        for (scope, _) in scopes {
            for (kind, acc) in [("word", &mut all_words), ("char", &mut all_chars)] {
                let t = ds.table(scope, kind).unwrap();
                for h in t.vfr.positions() {
                    *acc.entry(h.word.into_boxed_str()).or_insert(0) += h.count;
                }
            }
        }
        drop(ds);
        let fentries = rank::rank_entries_owned(all_words, &|_| 0);
        let fcentries = rank::rank_entries_owned(all_chars, &|_| 0);
        write_scope_entries(root, "full", "word", &fentries, true);
        write_scope_entries(root, "full", "char", &fcentries, false);
        let ftotal: u64 = fentries.iter().map(|e| e.count).sum();
        let fctotal: u64 = fcentries.iter().map(|e| e.count).sum();
        push_table(&mut meta.tables, "full", "word", &fentries, ftotal, 1);
        push_table(&mut meta.tables, "full", "char", &fcentries, fctotal, 1);
        artifact::write_meta(&root.join("meta.json"), &meta).unwrap();
        meta
    }

    /// 写出一份产物的 `meta.json`。`tables` 必须与磁盘上已有的 `.vfr` 一一对应，
    /// 否则 `Dataset::open` 会拒收（那正是它要挡的"半截产物"）。
    fn write_product_meta(
        root: &Path,
        scopes: &[(&str, &[(&str, u64)])],
        tokenizer: TokenizerMeta,
        tables: Vec<TableMeta>,
    ) -> Meta {
        let meta = Meta {
            schema_version: SCHEMA_VERSION,
            generated_at: String::new(),
            tool_version: String::new(),
            corpus_root: String::new(),
            elapsed_ms: 0,
            tokenizer,
            totals: crate::Totals::default(),
            domains: scopes
                .iter()
                .map(|(s, _)| SourceScope {
                    name: (*s).to_string(),
                    files: 1,
                    bytes: 1,
                })
                .collect(),
            tables,
            tier_names: rank::TIER_NAMES.iter().map(|s| s.to_string()).collect(),
            tier_keys: rank::TIER_KEYS.iter().map(|s| s.to_string()).collect(),
        };
        artifact::write_meta(&root.join("meta.json"), &meta).unwrap();
        meta
    }
    /// 三个表组的形状（既有测试都用它）。
    const THREE_SCOPES: [(&str, &[(&str, u64)]); 3] = [
        ("news", &[("的", 100), ("中国", 40), ("人工智能", 7)]),
        ("wiki", &[("的", 80), ("宇宙", 30), ("人工智能", 3)]),
        ("book", &[("的", 60), ("中国", 10), ("文学", 5)]),
    ];
    /// 一份三表组、**带 `full`** 的产物（`vocfreq scan --full` 那种）。
    fn make_full_product(root: &Path, min_count: u64) -> Meta {
        let _ = min_count;
        write_full_product(root, "t", &THREE_SCOPES)
    }
    /// 两份**互相独立**的扫描产物：A 只有 `news`+`wiki`，B 只有 `book`。
    ///
    /// 这正是"全量语料放不下、只能一个表组一个表组扫"的形状 —— 两份产物在互不相邻的目录里，
    /// 谁也不知道对方存在（想嵌在一起也不行：合流拒绝互相嵌套的源）。两份都**不含
    /// `full`**，与现在 `scan` 的默认行为一致。
    fn two_products(tag: &str) -> (PathBuf, PathBuf) {
        let a = tmpdir(&format!("{tag}-a"));
        let b = tmpdir(&format!("{tag}-b"));
        write_scanned_product(&a, "a", &THREE_SCOPES[..2], 1);
        write_scanned_product(&b, "b", &THREE_SCOPES[2..], 1);
        (a, b)
    }
    /// 造一个**新的**临时目录当产物目标。
    ///
    /// 与 [`tmpdir`] 不同：每次调用都用不同的名字，因此可以重复造目标 ——
    /// 合流与相加都是"目标已存在就拒绝"，一条测试里要试好几次就得有个新目录。
    fn fresh_out(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "vocfreq_compose_{}_{tag}-out{n}",
            std::process::id()
        ));
        // 上一次跑测试留下来的同名目录必须先清掉：合流/相加都是"目标已存在就拒绝"，
        // 否则第二次跑测试会撞上上一次的残留。
        let _ = std::fs::remove_dir_all(&p);
        p
    }
    /// 一份产物内部相加时用的 spec（`products` 留空 = 只有 `from` 这一份）。
    fn spec(
        from: &Path,
        sources: &[&str],
        scope: &str,
        kind: Option<&str>,
        out: Option<PathBuf>,
    ) -> ComposeSpec {
        ComposeSpec {
            from: from.to_path_buf(),
            products: Vec::new(),
            sources: sources.iter().map(|s| (*s).to_string()).collect(),
            scope: scope.to_string(),
            kind: kind.map(|k| k.to_string()),
            out,
        }
    }
    fn words_of(root: &Path, scope: &str) -> Vec<(String, u64, u32)> {
        let ds = Dataset::open(root).unwrap();
        let t = ds.table(scope, "word").unwrap();
        let mut v: Vec<(String, u64, u32)> = t
            .vfr
            .positions()
            .map(|h| (h.word, h.count, h.rank))
            .collect();
        v.sort();
        v
    }
    // ------------------------------------------------------------ 单产物相加
    #[test]
    fn compose_equal_to_full_scan() {
        // 这是相加的**核心正确性**：把各表组表相加，必须与全量扫描出来的 full 表
        // 逐条相等（频次、排名、条目数、总量）。不符就说明相加漏了词或重复计数。
        let root = tmpdir("equal");
        make_full_product(&root, 1);
        let out = compose(&spec(
            &root,
            &["news/word", "wiki/word", "book/word"],
            "相加",
            Some("word"),
            None,
        ))
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, "word");
        let composed = words_of(&root, "相加");
        let full = words_of(&root, "full");
        assert_eq!(composed, full, "相加出来的词频表必须与全量扫描逐条一致");
        let ds = Dataset::open(&root).unwrap();
        assert_eq!(
            ds.table("相加", "word").unwrap().total_tokens,
            ds.table("full", "word").unwrap().total_tokens,
            "token 总量也要一致（它决定 pct）"
        );
        // meta 里出现了新表组，且记下了来源
        let t = ds.meta.table("相加", "word").unwrap();
        assert_eq!(t.source_tables.len(), 3);
        assert!(t.source_tables.contains(&"news/word".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn compose_chars_too() {
        let root = tmpdir("chars");
        make_full_product(&root, 1);
        compose(&spec(
            &root,
            &["news/char", "wiki/char", "book/char"],
            "相加字",
            Some("char"),
            None,
        ))
        .unwrap();
        let ds = Dataset::open(&root).unwrap();
        let c = ds.table("相加字", "char").unwrap();
        let f = ds.table("full", "char").unwrap();
        assert_eq!(c.entries, f.entries);
        assert_eq!(c.total_tokens, f.total_tokens);
        for h in f.vfr.positions() {
            let got = c
                .lookup(&h.word)
                .unwrap_or_else(|| panic!("相加丢了字 {}", h.word));
            assert_eq!(got.count, h.count, "字 {} 的频次不符", h.word);
            assert_eq!(got.rank, h.rank);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn compose_both_kinds_when_no_kind_given() {
        let root = tmpdir("both");
        make_full_product(&root, 1);
        let out = compose(&spec(
            &root,
            &["news/word", "news/char", "wiki/word", "wiki/char"],
            "两个都加",
            None,
            None,
        ))
        .unwrap();
        let kinds: Vec<&str> = out.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(kinds, vec!["char", "word"], "两类都要产出");
        // word 只该由两张词频表加出来，不能把字表也算进去
        let w = out.iter().find(|c| c.kind == "word").unwrap();
        assert_eq!(
            w.sources,
            vec!["news/word".to_string(), "wiki/word".to_string()]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn compose_respects_min_count() {
        // 源表建表时用了 min_count=4，相加方必须丢掉同一批低频词，
        // 否则相加结果会比全量扫描多出一批词、排名整体前移。
        let root = tmpdir("mincount");
        make_full_product(&root, 4);
        compose(&spec(
            &root,
            &["news/word", "wiki/word", "book/word"],
            "阈值相加",
            Some("word"),
            None,
        ))
        .unwrap();
        let composed = words_of(&root, "阈值相加");
        assert!(
            composed.iter().all(|(_, c, _)| *c >= 4),
            "低于阈值的词不该出现在相加结果里：{composed:?}"
        );
        assert_eq!(composed, words_of(&root, "full"));
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn compose_keeps_flags_and_recomputes_tiers() {
        let root = tmpdir("tiers");
        make_full_product(&root, 1);
        compose(&spec(
            &root,
            &["news/word", "wiki/word"],
            "两张",
            Some("word"),
            None,
        ))
        .unwrap();
        let ds = Dataset::open(&root).unwrap();
        let t = ds.meta.table("两张", "word").unwrap();
        assert_eq!(t.tier_pct.len(), 6, "前%上界必须是 6 个");
        assert_eq!(t.tiers.len(), 7, "阈值必须是 7 档（最后一档是以上全部）");
        assert_eq!(t.tiers[6].max_rank, u64::MAX);
        assert!(
            t.tiers[..6]
                .windows(2)
                .all(|w| w[0].max_rank < w[1].max_rank)
        );
        // 分档统计要按新表的条目数重算，且覆盖率加总为 1
        let sum: f64 = t.tier_stats.iter().map(|s| s.coverage).sum();
        assert!((sum - 1.0).abs() < 1e-9, "覆盖率之和应为 1，实际 {sum}");
        // 条目数变了，阈值也该跟着变（前%口径下与条目数成正比）
        assert_eq!(
            t.entries, 4,
            "news 3 个词 + wiki 3 个词，其中「的」「人工智能」重叠 → 4 个"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn compose_writes_to_a_separate_directory() {
        let root = tmpdir("outdir");
        make_full_product(&root, 1);
        let out_root = tmpdir("outdir_dst");
        let out = compose(&spec(
            &root,
            &["news/word", "wiki/word", "book/word"],
            "搬到别处",
            Some("word"),
            Some(out_root.clone()),
        ))
        .unwrap();
        assert_eq!(out[0].scope, "搬到别处");
        // 目录树自检：每个表组都该有自己那一份表
        let mut tree: Vec<String> = Vec::new();
        for scope in std::fs::read_dir(&out_root).unwrap().flatten() {
            if !scope.path().is_dir() {
                continue;
            }
            for f in std::fs::read_dir(scope.path()).unwrap().flatten() {
                tree.push(format!(
                    "{}/{}",
                    scope.file_name().to_string_lossy(),
                    f.file_name().to_string_lossy()
                ));
            }
        }
        tree.sort();
        assert!(
            tree.iter().any(|p| p == "搬到别处/word.vfr"),
            "新表组的表要落在自己的目录里：{tree:?}"
        );
        assert!(
            !tree.iter().any(|p| p.starts_with("news/full")),
            "搬运不该把源表塞进新表组的目录：{tree:?}"
        );
        // 目标目录里应当是一份完整可打开的产物（meta 由源产物并出来）
        let ds = Dataset::open(&out_root)
            .unwrap_or_else(|e| panic!("搬过去的产物必须能打开：{e}\n目录内容：{tree:?}"));
        assert!(ds.table("搬到别处", "word").is_some());
        assert_eq!(
            ds.table("搬到别处", "word").unwrap().entries,
            ds.table("full", "word").unwrap().entries
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&out_root);
    }
    #[test]
    fn compose_rejects_missing_duplicate_and_unwritable_targets() {
        let root = tmpdir("reject");
        make_full_product(&root, 1);
        // 找不到的表
        let e = compose(&spec(&root, &["不存在/word"], "x", Some("word"), None)).unwrap_err();
        assert!(e.to_string().contains("找不到表"), "{e}");
        // 重复选同一张
        let e = compose(&spec(
            &root,
            &["news/word", "news/word"],
            "y",
            Some("word"),
            None,
        ))
        .unwrap_err();
        assert!(e.to_string().contains("两次"), "{e}");
        // 表组名撞车（`full` 已经存在）—— 绝不能覆盖
        let e = compose(&spec(&root, &["news/word"], "full", Some("word"), None)).unwrap_err();
        assert!(e.to_string().contains("已经"), "{e}");
        // 一张都没选
        let e = compose(&spec(&root, &[], "z", None, None)).unwrap_err();
        assert!(e.to_string().contains("没有选择"), "{e}");
        // 表组名会被洗干净，不会跑出目录
        let c = compose(&spec(
            &root,
            &["news/word"],
            "上/级/目录",
            Some("word"),
            None,
        ))
        .unwrap();
        assert_eq!(c[0].scope, "上_级_目录");
        assert!(root.join("上_级_目录").is_dir(), "必须落在产物目录内部");
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn composed_table_is_a_first_class_scope() {
        // 相加出来的表要和别的表完全平级：能当主表组、能被再次相加。
        let root = tmpdir("firstclass");
        make_full_product(&root, 1);
        compose(&spec(
            &root,
            &["news/word", "wiki/word"],
            "一阶",
            Some("word"),
            None,
        ))
        .unwrap();
        compose(&spec(
            &root,
            &["一阶/word", "book/word"],
            "二阶",
            Some("word"),
            None,
        ))
        .unwrap();
        let ds = Dataset::open_with_primary(&root, Some("二阶")).unwrap();
        assert_eq!(ds.primary_scope, "二阶");
        assert_eq!(ds.primary_table("word").unwrap().scope, "二阶");
        // 二阶 == full（一阶已经等于 news+wiki）
        let a = ds.table("二阶", "word").unwrap();
        let b = ds.table("full", "word").unwrap();
        assert_eq!(a.entries, b.entries);
        assert_eq!(a.total_tokens, b.total_tokens);
        for h in b.vfr.positions() {
            assert_eq!(
                a.lookup(&h.word).map(|x| x.count),
                Some(h.count),
                "词 {} 在二阶表里对不上",
                h.word
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
    // ------------------------------------------------------------ 跨产物合流
    /// `scan` 默认不产 `full`，`full` 由「跨产物合流 + 相加」得到。
    ///
    /// 这条测试钉的是**验收标准 1**：表组独立扫描（各产物只有自己的表组）→ 合流成
    /// 一份 → 把表组相加出 `full`，结果与"各表组全量累加"逐条一致（条目数、token
    /// 总量、每个词的频次与排名）。
    #[test]
    fn merge_products_then_compose_full_matches_accumulated_scopes() {
        let (a, b) = two_products("merge-ok");
        // 参照基准：把两份产物所有表组的词/字计数累加起来，按同一口径排行。
        // （等价于"一次全量扫描出来的 full"，见 `write_full_product`。）
        let mut ref_words = accumulated(&[&a, &b]);
        ref_words.sort();
        let ref_chars = accumulated_chars(&[&a, &b]);
        let out = fresh_out("merge-ok");
        let report = merge(&MergeSpec {
            from: vec![a.clone(), b.clone()],
            out: out.clone(),
            scope: None,
        })
        .expect("词典链一致时合流应当成功");
        assert_eq!(report.tables, 6, "3 个表组 × 2 类表");
        assert_eq!(
            report.scopes,
            vec!["book", "news", "wiki"],
            "合流产物里的表组就是各源的表组，一个不多一个不少"
        );
        // 合流产物是一份**标准 v3 单产物**：能直接打开，表组各自一张表
        let ds = Dataset::open(&out).expect("合流产物必须能打开");
        assert_eq!(ds.meta.schema_version, SCHEMA_VERSION);
        for scope in ["news", "wiki", "book"] {
            assert!(ds.table(scope, "word").is_some(), "{scope}/word 必须在");
            assert!(ds.table(scope, "char").is_some(), "{scope}/char 必须在");
        }
        // `domains`（语料切片清单）是把两份并起来的
        let mut names: Vec<&str> = ds.meta.domains.iter().map(|d| d.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["book", "news", "wiki"]);
        // 相加出全库表组。**一次调用把两类都加出来** —— 一个表组只有一份目录，
        // 分两次 compose 会在第二次撞上"目录已存在"。
        let composed = compose(&spec(
            &out,
            &[
                "news/word",
                "wiki/word",
                "book/word",
                "news/char",
                "wiki/char",
                "book/char",
            ],
            "full",
            None,
            None,
        ))
        .expect("合流之后相加应当成功");
        assert_eq!(composed.len(), 2, "词频表与字表都要加出来");
        let word = composed.iter().find(|c| c.kind == "word").unwrap();
        assert_eq!(word.entries, ref_words.len() as u64);
        assert_eq!(
            words_of(&out, "full"),
            ref_words,
            "合流后相加必须与各表组累加逐条一致"
        );
        let ds = Dataset::open(&out).unwrap();
        let f = ds.table("full", "char").unwrap();
        assert_eq!(f.entries, ref_chars.len() as u64);
        for (word, count, rank) in &ref_chars {
            let got = f
                .lookup(word)
                .unwrap_or_else(|| panic!("相加丢了字 {word}"));
            assert_eq!(got.count, *count, "字 {word} 的频次不符");
            assert_eq!(got.rank, *rank, "字 {word} 的排名不符");
        }
        for d in [&a, &b, &out] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    /// 直接跨产物相加（不先合流）也要与累加一致 —— `from` + `products` 那条路。
    #[test]
    fn compose_accepts_extra_products() {
        let (a, b) = two_products("cross");
        let mut ref_words = accumulated(&[&a, &b]);
        // `words_of` 按词排序，这里也对齐，好逐条比
        ref_words.sort();
        let out = fresh_out("cross");
        let written = compose(&ComposeSpec {
            from: a.clone(),
            products: vec![b.clone()],
            sources: vec!["news/word".into(), "wiki/word".into(), "book/word".into()],
            scope: "全库".into(),
            kind: Some("word".into()),
            out: Some(out.clone()),
        })
        .expect("词典链一致时跨产物相加应当成功");
        assert_eq!(written[0].entries, ref_words.len() as u64);
        // 目标目录里必须是**完整**产物：源产物的全部表都在，外加新加出来的那一张
        let ds = Dataset::open(&out).expect("跨产物相加的产物要能打开");
        for key in [
            "news/word",
            "news/char",
            "wiki/word",
            "wiki/char",
            "book/word",
            "book/char",
        ] {
            assert!(ds.table_by_key(key).is_some(), "{key} 应当被搬进目标产物");
        }
        assert_eq!(words_of(&out, "全库"), ref_words);
        for d in [&a, &b, &out] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    /// 合流 / 跨产物相加时，词典链不一致必须**明确报错并拒绝**，绝不静默合并。
    #[test]
    fn merge_rejects_mismatched_dict_chain() {
        let (a, b) = two_products("dict-mismatch");
        // B 换了词典（同一份文件名、内容不同 → 指纹不同）
        let mut tok = fixture_tokenizer();
        tok.dicts[0].sha256 = "c".repeat(64);
        rewrite_tokenizer(&b, tok);
        let out = fresh_out("dict-mismatch");
        let e = merge(&MergeSpec {
            from: vec![a.clone(), b.clone()],
            out: out.clone(),
            scope: None,
        })
        .unwrap_err()
        .to_string();
        assert!(e.contains("词典链"), "错误里要指出是词典链的问题：{e}");
        assert!(
            e.contains("内容指纹不同"),
            "错误里要说清是哪个字段不同：{e}"
        );
        assert!(
            e.contains("第 2 份"),
            "错误里要指出是哪一份产物（两份产物的描述应当都出现）：{e}"
        );
        assert!(!out.exists(), "被拒绝时绝不能留下半个产物目录");
        // 跨产物相加走同一道门槛
        let e = compose(&ComposeSpec {
            from: a.clone(),
            products: vec![b.clone()],
            sources: vec!["news/word".into(), "wiki/word".into()],
            scope: "全库".into(),
            kind: Some("word".into()),
            out: Some(out.clone()),
        })
        .unwrap_err()
        .to_string();
        assert!(e.contains("拒绝合流"), "{e}");
        assert!(!out.exists(), "被拒绝时绝不能留下半个产物目录");
        for d in [&a, &b, &out] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    /// 分词口径不一致（这里是 `hmm`）同样拒绝，且错误里点名到字段。
    #[test]
    fn merge_rejects_mismatched_tokenize_options() {
        let (a, b) = two_products("opt-mismatch");
        let mut tok = fixture_tokenizer();
        tok.hmm = true;
        rewrite_tokenizer(&b, tok);
        let out = fresh_out("opt-mismatch");
        let e = merge(&MergeSpec {
            from: vec![a.clone(), b.clone()],
            out,
            scope: None,
        })
        .unwrap_err()
        .to_string();
        assert!(e.contains("hmm"), "错误里要点名到字段：{e}");
        assert!(
            e.contains("false") && e.contains("true"),
            "错误里要给两边的值：{e}"
        );
        for d in [&a, &b] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    /// 没有指纹的老产物（`dicts` 为空）**不可校验**，因此也拒绝 ——
    /// "不知道是不是同一份"不等于"是同一份"。
    #[test]
    fn merge_rejects_products_without_dict_fingerprints() {
        let (a, b) = two_products("legacy");
        // 两份**都**没有指纹。"两份都是空的"看着一致，其实什么也没说明 ——
        // 合流的正确性完全依赖"同一份词典、同一套分词口径"。
        for root in [&a, &b] {
            let mut tok = fixture_tokenizer();
            tok.dicts.clear();
            rewrite_tokenizer(root, tok);
        }
        let out = fresh_out("legacy");
        let e = merge(&MergeSpec {
            from: vec![a.clone(), b.clone()],
            out,
            scope: None,
        })
        .unwrap_err()
        .to_string();
        assert!(e.contains("指纹"), "错误里要说明是缺指纹：{e}");
        for d in [&a, &b] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// 两份产物里出现同名表组时拒绝合流：静默丢数据比报错糟得多。
    #[test]
    fn merge_rejects_scope_name_clash() {
        let a = tmpdir("clash-a");
        let b = tmpdir("clash-b");
        // A 有 news + book，B 也有 book（内容不同：前缀是 b）—— 三份表组里恰好只有
        // `book` 撞名，其余两个必须照样能认出来。
        write_scanned_product(&a, "a", &[THREE_SCOPES[0], THREE_SCOPES[2]], 1);
        write_scanned_product(&b, "b", &[("book", &[("甲", 5)])], 1);
        let out = fresh_out("clash");
        let e = merge(&MergeSpec {
            from: vec![a.clone(), b.clone()],
            out: out.clone(),
            scope: None,
        })
        .unwrap_err()
        .to_string();
        assert!(e.contains("表组名撞车"), "{e}");
        assert!(e.contains("book"), "要说清撞的是哪个名字：{e}");
        assert!(!out.exists());
        for d in [&a, &b, &out] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    /// 只给一份产物不叫合流，应当明确指向 `compose`。
    #[test]
    fn merge_needs_at_least_two_products() {
        let a = tmpdir("one");
        write_scanned_product(&a, "a", &THREE_SCOPES[..1], 1);
        let e = merge(&MergeSpec {
            from: vec![a.clone()],
            out: a.join("out"),
            scope: None,
        })
        .unwrap_err()
        .to_string();
        assert!(e.contains("至少要两份"), "{e}");
        let _ = std::fs::remove_dir_all(&a);
    }
    #[test]
    fn merge_writes_into_a_named_subdirectory() {
        let (a, b) = two_products("nested");
        let parent = fresh_out("nested");
        let report = merge(&MergeSpec {
            from: vec![a.clone(), b.clone()],
            out: parent.clone(),
            scope: Some("全库表".into()),
        })
        .unwrap();
        assert!(parent.join("全库表").join("meta.json").is_file());
        assert!(
            !parent.join("meta.json").exists(),
            "父目录本身不该被当成产物"
        );
        assert!(report.out.ends_with("全库表"));
        // 目标目录里不能留下临时目录
        let leftovers: Vec<String> = std::fs::read_dir(&parent)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("merging"))
            .collect();
        assert!(leftovers.is_empty(), "临时目录要清理干净：{leftovers:?}");
        for d in [&a, &b, &parent] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    // ------------------------------------------------------------ 辅助
    /// 把一份产物改掉 `tokenizer`（模拟"用户换了词典 / 改了分词选项"）。
    fn rewrite_tokenizer(root: &Path, tokenizer: TokenizerMeta) {
        let mut meta: Meta =
            serde_json::from_slice(&std::fs::read(root.join("meta.json")).unwrap()).unwrap();
        meta.tokenizer = tokenizer;
        artifact::write_meta(&root.join("meta.json"), &meta).unwrap();
    }
    /// 把若干份产物的**全部表组**累加起来，按同一口径排行。
    ///
    /// 这是"一次全量扫描出来的 `full`"的等价物，也是验收标准 1 里的对照基准。
    fn accumulated(roots: &[&Path]) -> Vec<(String, u64, u32)> {
        let mut acc: FxHashMap<String, u64> = FxHashMap::default();
        for root in roots {
            let ds = Dataset::open(root).unwrap();
            for t in &ds.tables {
                // `full` 是产物自己的加总，累加时跳过，否则会重复计入
                if t.kind != "word" || t.scope == "full" {
                    continue;
                }
                for h in t.vfr.positions() {
                    *acc.entry(h.word).or_insert(0) += h.count;
                }
            }
        }
        ranked(acc)
    }
    fn accumulated_chars(roots: &[&Path]) -> Vec<(String, u64, u32)> {
        let mut acc: FxHashMap<String, u64> = FxHashMap::default();
        for root in roots {
            let ds = Dataset::open(root).unwrap();
            for t in &ds.tables {
                if t.kind != "char" || t.scope == "full" {
                    continue;
                }
                for h in t.vfr.positions() {
                    *acc.entry(h.word).or_insert(0) += h.count;
                }
            }
        }
        ranked(acc)
    }
    /// 按频次降序、同频次按字节序排行，与 `rank::rank_entries_owned` 同口径。
    fn ranked(acc: FxHashMap<String, u64>) -> Vec<(String, u64, u32)> {
        let mut v: Vec<(String, u64)> = acc.into_iter().collect();
        v.sort_unstable_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
        });
        v.into_iter()
            .enumerate()
            .map(|(i, (w, c))| (w, c, (i + 1) as u32))
            .collect()
    }
}
