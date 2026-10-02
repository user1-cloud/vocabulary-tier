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

use crate::rank::{RankedEntry, Tier, TierStat};
use crate::{Error, Result, Totals, VERSION};

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
        entries[a as usize].word.as_bytes().cmp(entries[b as usize].word.as_bytes())
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
    header.resize(VFR_HEADER_LEN, 0);

    f.write_all(&header)?;
    f.write_all(&block_index)?;
    f.write_all(&heads)?;
    f.write_all(&rank_index)?;
    f.write_all(&records)?;
    f.flush()?;

    Ok(VfrWritten {
        entries: n as u64,
        total_tokens,
        file_bytes: records_base + records.len() as u64,
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
    writeln!(f, "# 词频列填的是语料中的出现次数，会被 jieba 当作路径概率权重使用。")?;
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
    pub dict: String,
    pub user_dict: Option<String>,
    pub min_len: usize,
    pub max_len: usize,
    pub keep_latin: bool,
    pub keep_digit: bool,
    pub skip_single_char: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainMeta {
    pub name: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableMeta {
    /// 相对输出目录的路径，不含扩展名，例如 `full/word`
    pub path: String,
    pub kind: String,
    pub entries: u64,
    pub total_tokens: u64,
    pub vfr_bytes: u64,
    pub tiers: Vec<Tier>,
    pub tier_stats: Vec<TierStat>,
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
    pub domains: Vec<DomainMeta>,
    pub tables: Vec<TableMeta>,
    /// 未收录于语料库的 token 应显示为该索引（顶层数组下标，供前端参考）
    pub tier_names: Vec<String>,
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
        for v in [0u64, 1, 127, 128, 300, 16_383, 16_384, u32::MAX as u64, u64::MAX] {
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
}
