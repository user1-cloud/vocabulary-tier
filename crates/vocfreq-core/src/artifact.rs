//! 产物写出：可读的 TSV 排行 + 可 mmap 查询的 `.vfr` 二进制索引 + `meta.json`。
//!
//! `.vfr` 格式规范见 `docs/DESIGN.md` §5.1。设计目标：打开即用、O(log n) 查词、
//! 常驻内存小。
//!
//! 文件布局：
//! `header(96) | 块索引(block_count*16) | 块首词区 | 排名索引(entry_count*4) | 记录区`
//!
//! 块索引里存的是**完整**块首词（不是截断前缀），因此对块做二分是精确的，
//! 查词只需线性扫一个块。块首词区在 500 万词条时也只有约 600 KB，打开时整块载入内存。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::dict::DictRef;
use crate::rank::{RankedEntry, Tier, TierStat};
use crate::{Error, Result, Totals, VERSION};

/// `meta.json` 的格式版本。
///
/// * **1**：`tokenizer.dict` 是一句自由文本标签，`user_dict` 是单个路径字符串。
///   没有指纹，所以「这张表基于哪份词库生成的」**无法校验** —— 词库当年是编在
///   exe 里的，没有替换的可能，也就没记的必要。
/// * **2**：词库外置，`tokenizer.dicts` 是完整词库链（每份都带 `sha256`），
///   读取方据此判断词库有没有被换过。表用 `full/word`、`domains/<域>/word`
///   这套「路径」指代，全库表与分域表是两个不同的概念。
/// * **3**：**作用域（scope）铺平**。不再区分「全库表」与「分域表」——每个作用域
///   就是一张词表加一张字表，`full` 只是其中一个作用域。`TableMeta.path` 的语义
///   从「路径」变成**作用域 id**（`full`、`news`、`相加：财经`），表身份统一用
///   `作用域/类型` 表示。相加产生的新表是同级的另一个作用域，靠
///   [`TableMeta::source_tables`] 记来源。
///
/// 读取方遇到 **< 3** 的产物必须**明确报错并让用户重扫**，不能静默按新布局去找文件
/// —— 那只会得到一句莫名其妙的「文件不存在」。
pub const SCHEMA_VERSION: u32 = 3;

/// 全量语料对应的作用域 id。
///
/// 它**不是特权作用域**：与 `news`、`wiki` 完全同级，只是"所有域加一起"这一份。
pub const SCOPE_FULL: &str = "full";

/// 作用域 id 里不允许出现的字符（目录名）。
///
/// 相加出来的作用域名是用户起的，会直接变成磁盘目录名，必须洗一遍。
/// 与桌面端的 `library::sanitize_table_name` 同源，但这里更严（连 `..` 都不允许）。
pub fn sanitize_scope(raw: &str) -> String {
    let mut s: String = raw
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() || s == "." || s == ".." {
        s = "未命名".into();
    }
    if s.chars().count() > 64 {
        s = s.chars().take(64).collect();
    }
    s
}

pub const VFR_MAGIC: &[u8; 8] = b"VOCFREQ1";
pub const VFR_VERSION: u32 = 1;
pub const VFR_HEADER_LEN: usize = 96;
pub const VFR_BLOCK_SIZE: usize = 64;
/// 块索引项：u64 记录偏移 + u32 块首词偏移 + u16 块首词长度 + u16 保留
pub const VFR_BLOCK_ENTRY_LEN: usize = 16;
/// header.flags 的 bit0：记录里带一个 flags 字节
pub const VFR_FLAG_HAS_FLAGS: u32 = 1;

pub const KIND_WORD: u32 = 0;
pub const KIND_CHAR: u32 = 1;

// ---------------------------------------------------------------- varint

pub fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// 从 `buf[*pos..]` 读一个 varint，越界或超长返回 None。
pub fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    loop {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

// ---------------------------------------------------------------- 时间戳

/// 由 1970-01-01 起的天数算出公历年月日（Howard Hinnant 的 civil_from_days）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 无依赖的 ISO-8601 UTC 时间戳，避免为此引入 chrono。
pub fn iso8601_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// ---------------------------------------------------------------- .vfr 写出

#[derive(Debug, Clone, Copy, Serialize)]
pub struct VfrWritten {
    pub entries: u64,
    pub total_tokens: u64,
    pub file_bytes: u64,
    pub block_count: u32,
}

/// 写出 `.vfr`。
///
/// `entries` 需按**排名升序**（即频次降序）传入，函数内部会另建一份按词字节序的
/// 顺序表用于块索引与记录区。
pub fn write_vfr(
    path: &Path,
    kind: u32,
    entries: &[RankedEntry],
    total_tokens: u64,
    with_flags: bool,
) -> Result<VfrWritten> {
    let n = entries.len();
    let block_size = VFR_BLOCK_SIZE;
    let block_count = n.div_ceil(block_size);

    // 记录区按词字节序排列，查词才能二分
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_unstable_by(|&a, &b| {
        entries[a as usize]
            .word
            .as_bytes()
            .cmp(entries[b as usize].word.as_bytes())
    });

    let index_bytes = block_count * VFR_BLOCK_ENTRY_LEN;
    let rank_index_bytes = n * 4;

    // 第一遍：拼块首词区（必须在算 records_base 之前完成）
    let mut heads: Vec<u8> = Vec::with_capacity(block_count * 8);
    let mut head_meta: Vec<(u32, u16)> = Vec::with_capacity(block_count);
    for chunk in order.chunks(block_size) {
        let w = entries[chunk[0] as usize].word.as_bytes();
        let off = heads.len();
        debug_assert!(off <= u32::MAX as usize, "块首词区超过 4GB");
        head_meta.push((off as u32, w.len() as u16));
        heads.extend_from_slice(w);
    }

    let heads_offset = (VFR_HEADER_LEN + index_bytes) as u64;
    let rank_index_offset = heads_offset + heads.len() as u64;
    let records_base = rank_index_offset + rank_index_bytes as u64;

    // 第二遍：记录区 / 块索引 / 排名索引
    let mut records: Vec<u8> = Vec::with_capacity(n * 14 + 64);
    let mut block_index: Vec<u8> = Vec::with_capacity(index_bytes);
    // 预分配成定长，按 rank 下标直接写入。不能用 resize(rank*4) 逐条增长：
    // 记录区是按词序生成的，rank 是乱序的，resize 到更短的长度会把已写的槽截掉。
    let mut rank_index: Vec<u8> = vec![0u8; rank_index_bytes];

    for (bi, chunk) in order.chunks(block_size).enumerate() {
        let off = records_base + records.len() as u64;
        let (h_off, h_len) = head_meta[bi];
        block_index.extend_from_slice(&off.to_le_bytes());
        block_index.extend_from_slice(&h_off.to_le_bytes());
        block_index.extend_from_slice(&h_len.to_le_bytes());
        block_index.extend_from_slice(&0u16.to_le_bytes());

        for &ei in chunk {
            let e = &entries[ei as usize];
            let start = records.len();
            write_varint(&mut records, e.word.len() as u64);
            records.extend_from_slice(e.word.as_bytes());
            write_varint(&mut records, e.count);
            write_varint(&mut records, e.rank as u64);
            if with_flags {
                records.push(e.flags);
            }
            let abs = records_base + start as u64;
            if abs > u32::MAX as u64 {
                return Err(Error::Format(format!(
                    "{} 的记录区超过 4GB（{abs} 字节），当前排名索引格式无法表示",
                    path.display()
                )));
            }
            let slot = (e.rank as usize - 1) * 4;
            rank_index[slot..slot + 4].copy_from_slice(&(abs as u32).to_le_bytes());
        }
    }

    let mut flags = 0u32;
    if with_flags {
        flags |= VFR_FLAG_HAS_FLAGS;
    }

    let records_len = records.len() as u64;

    let mut f = BufWriter::with_capacity(1 << 22, File::create(path)?);
    let mut header = Vec::with_capacity(VFR_HEADER_LEN);
    header.extend_from_slice(VFR_MAGIC);
    header.extend_from_slice(&VFR_VERSION.to_le_bytes());
    header.extend_from_slice(&kind.to_le_bytes());
    header.extend_from_slice(&(n as u64).to_le_bytes());
    header.extend_from_slice(&(block_size as u32).to_le_bytes());
    header.extend_from_slice(&(block_count as u32).to_le_bytes());
    header.extend_from_slice(&total_tokens.to_le_bytes());
    header.extend_from_slice(&(VFR_HEADER_LEN as u64).to_le_bytes());
    header.extend_from_slice(&heads_offset.to_le_bytes());
    // n == 0 时 rank 索引区为空，用 0 表示"无排名索引"
    let rio = if n == 0 { 0 } else { rank_index_offset };
    header.extend_from_slice(&rio.to_le_bytes());
    header.extend_from_slice(&flags.to_le_bytes());
    // 记录区范围（见 `docs/DESIGN.md` §5.1 的 68/76 字节）。
    //
    // ⚠ **不能靠文件长度推断记录区末尾**。最后一块的结束位置原本取的是 mmap 长度，
    // 而 `BufWriter` 会不会在 `records` 之后再落几个字节是实现细节 —— 实测那条路
    // 会让整表遍历把尾巴上的字节当记录解析，516 条的样本只遍历出 513 条
    // （`Positions` 就是踩了这个坑才加的这两个字段）。
    header.extend_from_slice(&records_base.to_le_bytes());
    header.extend_from_slice(&records_len.to_le_bytes());
    header.resize(VFR_HEADER_LEN, 0);

    f.write_all(&header)?;
    f.write_all(&block_index)?;
    f.write_all(&heads)?;
    f.write_all(&rank_index)?;
    f.write_all(&records)?;
    f.flush()?;

    // 把文件截到记录区末尾：让「文件长度」与「记录区末尾」在这种正常情形下也一致，
    // 老读取端（只认 mmap.len()）因此不会读到多余的字节。
    let total_bytes = records_base + records_len;
    drop(f);
    if let Ok(fh) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = fh.set_len(total_bytes);
    }

    Ok(VfrWritten {
        entries: n as u64,
        total_tokens,
        file_bytes: total_bytes,
        block_count: block_count as u32,
    })
}

// ---------------------------------------------------------------- TSV 写出

/// 写出可读排行。`entries` 需按排名升序。
pub fn write_tsv(
    path: &Path,
    table_name: &str,
    kind: &str,
    entries: &[RankedEntry],
    total_tokens: u64,
    tiers: &[Tier],
) -> Result<()> {
    let mut f = BufWriter::with_capacity(1 << 22, File::create(path)?);
    let total = total_tokens.max(1);
    writeln!(
        f,
        "# VocTier 频率排行  table={table_name}  kind={kind}  entries={}  total_tokens={total_tokens}",
        entries.len()
    )?;
    let bounds: Vec<String> = tiers
        .iter()
        .map(|t| {
            if t.max_rank == u64::MAX {
                format!("{}:>{}", t.name, tiers[tiers.len() - 2].max_rank)
            } else {
                format!("{}:<={}", t.name, t.max_rank)
            }
        })
        .collect();
    writeln!(f, "# 分组(按排名)  {}", bounds.join("  "))?;
    writeln!(f, "# flags: bit0=在jieba词典内 bit1=来自用户词典")?;
    writeln!(f, "rank\tcount\tpct\tword\tflags")?;
    for e in entries {
        writeln!(
            f,
            "{}\t{}\t{:.8}\t{}\t{}",
            e.rank,
            e.count,
            e.count as f64 * 100.0 / total as f64,
            e.word,
            e.flags
        )?;
    }
    f.flush()?;
    Ok(())
}

/// 写词典外高频候选词，可直接喂给 `--user-dict`。
pub fn write_oov(path: &Path, cands: &[(Box<str>, u64)]) -> Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    writeln!(f, "# VocTier 词典外高频条目")?;
    writeln!(
        f,
        "# 这些条目不在 jieba 内置词典中，因此关闭 HMM 时会被切成更小的碎片。"
    )?;
    writeln!(
        f,
        "# 把本文件（或筛过的子集）用 --user-dict 传入重新统计，它们就会成为独立词条。"
    )?;
    writeln!(
        f,
        "# 词频列填的是语料中的出现次数，会被 jieba 当作路径概率权重使用。"
    )?;
    writeln!(f, "#")?;
    writeln!(
        f,
        "# 实测提醒：关闭 HMM 时 jieba 只能输出「词典命中的词」或「未命中的单字」，"
    )?;
    writeln!(
        f,
        "# 所以多字中文新词（如「元宇宙」「直播带货」）**不可能**出现在这里。"
    )?;
    writeln!(
        f,
        "# 含汉字的条目几乎都是繁体单字（對/業/機/華…），因为 jieba 词典只有简体——"
    )?;
    writeln!(
        f,
        "# 它们确实值得回填。不含汉字的条目则是 URL/代码碎片（com/https/App…），"
    )?;
    writeln!(f, "# 属于语料噪声，建议用来排查清洗规则而不是加进词典。")?;
    for (w, c) in cands {
        writeln!(f, "{w} {c}")?;
    }
    f.flush()?;
    Ok(())
}

// ---------------------------------------------------------------- meta.json

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenizerMeta {
    pub engine: String,
    pub version: String,
    pub hmm: bool,
    /// 词库链，**按装载顺序**：`dicts[0]` 是主词库，其后都是叠加词库。
    ///
    /// 每份都带 `sha256`，所以读取方能回答「这张表是不是还配得上当前这份词库」。
    #[serde(default)]
    pub dicts: Vec<DictRef>,
    /// **v1 兼容**：老产物把主词库记成一句自由文本标签（`"dict": "builtin(...)"`）。
    ///
    /// 只在读老产物时才有值，写新产物时永不写出（词库链已经在 `dicts` 里了）。
    /// 注意 `DictRef` 的反序列化接受纯字符串，因此这里能直接接住 v1 的写法。
    #[serde(default, rename = "dict", skip_serializing_if = "Option::is_none")]
    pub legacy_dict: Option<DictRef>,
    /// **v1 兼容**：老产物把（唯一的）叠加词库记成单个路径字符串。
    ///
    /// 同样只读不写。读取方不能因为 `dicts` 为空就断定"这份产物没有词库" ——
    /// 老产物可能整个信息都在 `legacy_dict` / `legacy_user_dict` 里（约定同
    /// [`Meta::tier_keys`]：空数组 = 无从判断，不要据此报错）。
    #[serde(default, rename = "user_dict", skip_serializing_if = "Option::is_none")]
    pub legacy_user_dict: Option<String>,
    pub min_len: usize,
    pub max_len: usize,
    pub keep_latin: bool,
    pub keep_digit: bool,
    pub skip_single_char: bool,
}

impl TokenizerMeta {
    /// 实际生效的词库链。
    ///
    /// 优先用 v2 的 [`Self::dicts`]；只有它为空（老产物）时才回退到 v1 的两个字段。
    pub fn resolved_dicts(&self) -> Vec<DictRef> {
        if !self.dicts.is_empty() {
            return self.dicts.clone();
        }
        let mut out = Vec::new();
        if let Some(d) = &self.legacy_dict {
            out.push(d.clone());
        }
        if let Some(u) = &self.legacy_user_dict {
            out.push(DictRef::legacy(u));
        }
        out
    }

    /// 是不是 v1 老格式 —— 也就是「没有指纹、词库一致性无从校验」。
    pub fn is_legacy(&self) -> bool {
        self.dicts.is_empty()
    }

    /// 词库链里有没有**可校验**的成员。
    pub fn has_verifiable_dict(&self) -> bool {
        self.resolved_dicts().iter().any(DictRef::is_verifiable)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceScope {
    pub name: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableMeta {
    /// **作用域 id**，同时是 `<产物目录>/<path>/` 这个子目录名，例如 `full`、`news`。
    ///
    /// ⚠ 语义在 schema v3 变过一次：从前它是 `full/word`、`domains/news/char`
    /// 这种「路径」，把作用域与类型糊在一个字符串里。现在它只是作用域，
    /// 类型在 [`Self::kind`] 里；两者合起来才是表身份（见 [`table_key`]）。
    pub path: String,
    pub kind: String,
    pub entries: u64,
    pub total_tokens: u64,
    pub vfr_bytes: u64,
    pub tiers: Vec<Tier>,
    pub tier_stats: Vec<TierStat>,
    /// 建这张表时用的低频过滤阈值。
    ///
    /// 缺了它，相加出来的表就与「全量重扫一次」的结果对不上：源表已经按各自的
    /// 阈值丢过一批低频词，相加方必须丢掉同一批，重编号后的排名才与全量扫描一致。
    #[serde(default = "default_min_count")]
    pub min_count: u64,
    /// 七组的**前%上界**（0..100），第 7 组为「以上全部」，因此恒为 6 个数。
    ///
    /// 这是稀有度的**默认口径**：`前% = 排名 ÷ 总条目数 × 100`。绝对排名阈值仍在
    /// [`Self::tiers`] 里（老读取路径与 CLI 靠它），两者由同一套默认值算出，
    /// 但**按前%分组时不读 `tiers`** —— 换一张条目数不同的表，绝对阈值会失真。
    ///
    /// `#[serde(default)]` 兜住没有这个字段的产物；空数组 = 无从判断，用
    /// [`crate::rank::default_tier_pct`]。
    #[serde(default)]
    pub tier_pct: Vec<f64>,
    /// 这张表是把哪些表**相加**出来的；空数组 = 这是扫描出来的原始表。
    ///
    /// 存的是 `作用域/类型` 形式（与用户的「相加」选择一一对应），用于界面显示
    /// 来源，也是将来做「零拷贝虚拟合成」的接口。
    #[serde(default)]
    pub source_tables: Vec<String>,
}

fn default_min_count() -> u64 {
    1
}

impl TableMeta {
    /// 这张表的身份：`作用域/类型`，例如 `full/word`、`相加：财经/char`。
    pub fn key(&self) -> String {
        table_key(&self.path, &self.kind)
    }

    /// 实际生效的前%上界（缺字段时用默认值）。
    pub fn effective_tier_pct(&self) -> Vec<f64> {
        if self.tier_pct.len() == crate::rank::TIER_NAMES.len() - 1 {
            self.tier_pct.clone()
        } else {
            crate::rank::default_tier_pct_for(&self.kind).to_vec()
        }
    }
}

/// 表身份字符串：`作用域/类型`。
///
/// 全仓库**只有这一处**拼这个字符串（Rust 侧）；前端的等价实现是
/// `apps/desktop/src/lib/format.ts::tableKey`。设置里的 `primaryScope` 与之无关
/// （那存的是纯作用域），老设置里的 `enabledTables` 存的是这个形式，因此仍然能对上。
pub fn table_key(scope: &str, kind: &str) -> String {
    format!("{scope}/{kind}")
}

/// 拆开表身份字符串。作用域本身可能含 `/`（相加表的上一级名字 + 子名），
/// 所以**从右边**切最后一个 `/`。
pub fn split_table_key(key: &str) -> (String, String) {
    match key.rsplit_once('/') {
        Some((scope, kind)) => (scope.to_string(), kind.to_string()),
        None => (key.to_string(), String::new()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub schema_version: u32,
    pub generated_at: String,
    pub tool_version: String,
    pub corpus_root: String,
    pub elapsed_ms: u64,
    pub tokenizer: TokenizerMeta,
    pub totals: Totals,
    /// 建这张表时语料库里**实际扫到的作用域**（含文件数、字节数）。
    ///
    /// ⚠ 这是「**语料切片**的清单」，不是「表清单」—— 表清单看 [`Self::tables`]。
    /// 两者以前是同一件事（扫描按一级子目录分域、每域产出一张表），现在不是了：
    ///
    /// * 「生成词频表」页的扫描计划显示的是**语料切片**（磁盘上的一级子目录）；
    /// * 「划句分析」「排行榜」「表管理」的作用域则是**表**（`meta.scopes()`）。
    ///
    /// 相加产生的表里这个字段可能为空（相加不再读语料）。
    #[serde(default)]
    pub domains: Vec<SourceScope>,
    pub tables: Vec<TableMeta>,
    /// 七组的**展示名**（见 `rank::TIER_NAMES`），与 `tier_keys` / `tiers` / `tier_stats` 同序。
    ///
    /// 仅供展示与兜底，**不要当身份用**。
    pub tier_names: Vec<String>,
    /// 七组的**稳定标识**（见 `rank::TIER_KEYS`），与 `tier_names` 同序、与语言无关。
    ///
    /// 界面用它把「组号」映射到配色与本地化文案：产物调整分组顺序也不会串色，
    /// 遇到不认识的 key 会退化成中性灰。
    ///
    /// `#[serde(default)]` 是为了兼容本次改动之前产出的老产物 —— 那种产物没有这个字段，
    /// 反序列化得到空数组，读取方需自行兜底（**空数组 = 无从判断，不要据此报错**）。
    #[serde(default)]
    pub tier_keys: Vec<String>,
}

impl Meta {
    /// 按「作用域 + 类型」找一张表。**这是全仓库唯一的查表入口** ——
    /// schema v3 之前到处写 `tables.iter().find(|t| t.path == "full/word")`，
    /// 布局一改就全散架了。
    pub fn table(&self, scope: &str, kind: &str) -> Option<&TableMeta> {
        self.tables
            .iter()
            .find(|t| t.path == scope && t.kind == kind)
    }

    /// 按表身份（`作用域/类型`）找一张表。
    pub fn table_by_key(&self, key: &str) -> Option<&TableMeta> {
        let (scope, kind) = split_table_key(key);
        self.table(&scope, &kind)
    }

    /// 全部作用域 id，按「`full` 优先，其余按名字」排序。
    ///
    /// 顺序确定很重要：界面列表、回落链、对比列都按它排，否则同一份产物在不同
    /// 机器上（目录枚举顺序不同）会长得不一样。
    pub fn scopes(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for t in &self.tables {
            if !names.contains(&t.path) {
                names.push(t.path.clone());
            }
        }
        names.sort_by(|a, b| {
            let rank = |s: &str| if s == SCOPE_FULL { 0 } else { 1 };
            rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
        });
        names
    }
}

pub fn write_meta(path: &Path, meta: &Meta) -> Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut f, meta)?;
    f.write_all(b"\n")?;
    f.flush()?;
    Ok(())
}

pub fn write_string(path: &Path, s: &str) -> Result<()> {
    std::fs::write(path, s)?;
    Ok(())
}

/// 供 CLI 打印的简短描述。
pub fn describe(meta: &Meta) -> String {
    format!(
        "VocTier {VERSION} | {} | {} 张表 | 语料 {} 文件 {:.1} GB | token {}",
        meta.generated_at,
        meta.tables.len(),
        meta.totals.files,
        meta.totals.bytes as f64 / 1e9,
        meta.totals.tokens
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for v in [
            0u64,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u32::MAX as u64,
            u64::MAX,
        ] {
            let mut buf = Vec::new();
            write_varint(&mut buf, v);
            let mut pos = 0;
            assert_eq!(read_varint(&buf, &mut pos), Some(v), "v={v}");
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn read_varint_rejects_truncated() {
        let mut pos = 0;
        assert_eq!(read_varint(&[0x80], &mut pos), None);
        pos = 0;
        assert_eq!(read_varint(&[], &mut pos), None);
    }

    #[test]
    fn iso8601_known_epochs() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(iso8601_now().len(), 20);
        assert!(iso8601_now().ends_with('Z'));
    }

    /// 加入 `tier_keys` **之前**产出的 meta.json：没有这个字段。
    ///
    /// 老产物必须仍然能读进来 —— 这是 `#[serde(default)]` 存在的唯一理由，
    /// 也是「加字段不废掉用户已有产物」这条承诺的回归测试。
    const LEGACY_META_JSON: &str = r#"{
        "schema_version": 1,
        "generated_at": "2026-01-01T00:00:00Z",
        "tool_version": "0.1.0",
        "corpus_root": "D:\\corpus",
        "elapsed_ms": 1,
        "tokenizer": {
            "engine": "jieba-rs", "version": "0.11", "hmm": false, "dict": "builtin",
            "user_dict": null, "min_len": 1, "max_len": 10,
            "keep_latin": false, "keep_digit": false, "skip_single_char": false
        },
        "totals": { "files": 1, "bytes": 2, "lines": 3, "paras": 4, "tokens": 5, "bad_lines": 0 },
        "domains": [{ "name": "wiki", "files": 1, "bytes": 2 }],
        "tables": [{
            "path": "full/word", "kind": "word", "entries": 1, "total_tokens": 1, "vfr_bytes": 1,
            "tiers": [{ "name": "极多", "max_rank": 100 }],
            "tier_stats": [{ "name": "极多", "max_rank": 100, "entries": 1, "tokens": 1,
                             "coverage": 1.0, "cumulative": 1.0 }]
        }],
        "tier_names": ["极多"]
    }"#;

    #[test]
    fn legacy_meta_without_tier_keys_still_deserializes() {
        let meta: Meta = serde_json::from_str(LEGACY_META_JSON).expect("老产物必须仍然可读");
        assert!(meta.tier_keys.is_empty(), "缺失的 tier_keys 应回落为空数组");
        assert_eq!(meta.tier_names, vec!["极多".to_string()]);
        assert_eq!(meta.tables[0].tiers[0].max_rank, 100);
        // v2 的 `domains` 与 v3 同义同名同类型（语料切片清单），照常读进来。
        // 判断"这是不是新布局"靠 `tables[].path` 里有没有 `/`，不靠这个字段。
        assert_eq!(meta.domains.len(), 1, "老产物的语料切片清单要能读进来");
        assert_eq!(meta.domains[0].name, "wiki");
        // `min_count` / `tier_pct` / `source_tables` 都是新字段，老产物里没有
        assert_eq!(meta.tables[0].min_count, 1, "缺失的 min_count 应回落为 1");
        assert!(meta.tables[0].source_tables.is_empty());
    }

    #[test]
    fn table_lookup_and_scope_ordering_are_deterministic() {
        let mut meta: Meta = serde_json::from_str(LEGACY_META_JSON).unwrap();
        meta.tables = ["wiki", "full", "news"]
            .iter()
            .map(|s| TableMeta {
                path: (*s).to_string(),
                kind: "word".into(),
                entries: 1,
                total_tokens: 1,
                vfr_bytes: 1,
                tiers: Vec::new(),
                tier_stats: Vec::new(),
                min_count: 1,
                tier_pct: Vec::new(),
                source_tables: Vec::new(),
            })
            .collect();
        assert_eq!(
            meta.scopes(),
            vec!["full", "news", "wiki"],
            "full 必须排在最前"
        );
        assert_eq!(meta.table("news", "word").map(|t| t.entries), Some(1));
        assert!(
            meta.table("news", "char").is_none(),
            "作用域对、类型不对就不该命中"
        );
        // 表身份是「作用域/类型」；作用域自己可以带斜杠（相加表的父名）
        assert_eq!(
            meta.table_by_key("full/word").map(|t| &t.path),
            Some(&"full".to_string())
        );
        let mut m2 = meta.clone();
        m2.tables.push(TableMeta {
            path: "相加：财经".into(),
            ..m2.tables[0].clone()
        });
        assert_eq!(
            m2.table_by_key("相加：财经/word").map(|t| &t.path),
            Some(&"相加：财经".to_string())
        );
    }

    #[test]
    fn scope_and_table_key_helpers_round_trip() {
        assert_eq!(table_key("full", "word"), "full/word");
        assert_eq!(split_table_key("full/word"), ("full".into(), "word".into()));
        // 作用域含斜杠时从右边切
        assert_eq!(split_table_key("a/b/char"), ("a/b".into(), "char".into()));
        assert_eq!(sanitize_scope("新闻/财经"), "新闻_财经");
        assert_eq!(sanitize_scope("  ..  "), "未命名");
        assert_eq!(sanitize_scope("a:b*c?"), "a_b_c_");
        assert_eq!(sanitize_scope(&"长".repeat(100)).chars().count(), 64);
    }

    #[test]
    fn effective_tier_pct_falls_back_when_missing_or_wrong_length() {
        let mut meta: Meta = serde_json::from_str(LEGACY_META_JSON).unwrap();
        let t = &mut meta.tables[0];
        assert_eq!(
            t.effective_tier_pct(),
            crate::rank::default_tier_pct().to_vec()
        );
        // 长度不对（半截数据）也必须回落，而不是拿 3 个上界去切 7 组
        t.tier_pct = vec![1.0, 2.0, 3.0];
        assert_eq!(
            t.effective_tier_pct(),
            crate::rank::default_tier_pct().to_vec()
        );
        t.tier_pct = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(t.effective_tier_pct(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn tier_keys_round_trip_through_json() {
        let mut meta: Meta = serde_json::from_str(LEGACY_META_JSON).unwrap();
        meta.tier_keys = crate::rank::TIER_KEYS
            .iter()
            .map(|s| s.to_string())
            .collect();
        let text = serde_json::to_string(&meta).unwrap();
        assert!(
            text.contains("\"tier_keys\""),
            "tier_keys 必须真的写进 JSON"
        );
        let back: Meta = serde_json::from_str(&text).unwrap();
        assert_eq!(back.tier_keys, meta.tier_keys);
    }
}
