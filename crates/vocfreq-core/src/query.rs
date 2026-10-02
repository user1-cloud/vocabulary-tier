//! `.vfr` 查询：mmap 打开、精确二分查词、按排名取条目。
//!
//! 还提供 [`Dataset`]：一次性打开全库表 + 全部分域表，并对一句话做完整分析
//! （分词 → 查频次 → 定分组 → 附分域排名）。前端的划句分析直接调它，
//! 命令行 `segment` 也走同一条代码路径，因此两条链路的行为必然一致。

use std::fs::File;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use serde::Serialize;

use crate::artifact::{Meta, VFR_BLOCK_ENTRY_LEN, VFR_FLAG_HAS_FLAGS, VFR_HEADER_LEN, VFR_MAGIC, VFR_VERSION};
use crate::rank::tier_of;
use crate::tokenize::Tokenizer;
use crate::{Error, Result};

pub const KIND_WORD: u32 = 0;
pub const KIND_CHAR: u32 = 1;

#[inline]
fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
#[inline]
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
#[inline]
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct VfrHeader {
    pub version: u32,
    pub kind: u32,
    pub entry_count: u64,
    pub block_size: u32,
    pub block_count: u32,
    pub total_tokens: u64,
    pub index_offset: u64,
    pub heads_offset: u64,
    pub rank_index_offset: u64,
    pub flags: u32,
}

/// 一次命中。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub word: String,
    pub count: u64,
    pub rank: u32,
    pub flags: u8,
}

pub struct VfrTable {
    mmap: Mmap,
    header: VfrHeader,
    /// 块索引区，打开时载入内存（5M 词条约 1.2 MB）
    index: Vec<u8>,
    /// 块首词区，打开时载入内存（5M 词条约 0.6 MB）
    heads: Vec<u8>,
    path: String,
}

impl VfrTable {
    pub fn open(path: &Path) -> Result<Self> {
        let f = File::open(path)?;
        // 安全性：产物是只读的，只要文件在映射期间不被外部截断/改写即可。
        let mmap = unsafe { Mmap::map(&f)? };
        let header = Self::parse_header(&mmap, path)?;

        let idx_start = header.index_offset as usize;
        let idx_len = header.block_count as usize * VFR_BLOCK_ENTRY_LEN;
        let index = mmap
            .get(idx_start..idx_start + idx_len)
            .ok_or_else(|| Error::Format(format!("{} 块索引区越界", path.display())))?
            .to_vec();

        // 块首词区 = [heads_offset, rank_index_offset)；n == 0 时无排名索引，则到文件尾
        let heads_end = if header.rank_index_offset == 0 {
            mmap.len()
        } else {
            header.rank_index_offset as usize
        };
        if heads_end < header.heads_offset as usize || heads_end > mmap.len() {
            return Err(Error::Format(format!("{} 块首词区范围非法", path.display())));
        }
        let heads = mmap[header.heads_offset as usize..heads_end].to_vec();

        Ok(VfrTable { mmap, header, index, heads, path: path.display().to_string() })
    }

    fn parse_header(mmap: &[u8], path: &Path) -> Result<VfrHeader> {
        let bad = |m: &str| Error::Format(format!("{}: {m}", path.display()));
        if mmap.len() < VFR_HEADER_LEN {
            return Err(bad("文件过短，不是有效的 .vfr"));
        }
        if &mmap[..8] != VFR_MAGIC {
            return Err(bad("magic 不匹配，不是 VocTier 频率表"));
        }
        let version = u32_at(mmap, 8).unwrap();
        if version != VFR_VERSION {
            return Err(bad(&format!("版本 {version} 与当前支持的 {VFR_VERSION} 不匹配")));
        }
        Ok(VfrHeader {
            version,
            kind: u32_at(mmap, 12).unwrap(),
            entry_count: u64_at(mmap, 16).unwrap(),
            block_size: u32_at(mmap, 24).unwrap(),
            block_count: u32_at(mmap, 28).unwrap(),
            total_tokens: u64_at(mmap, 32).unwrap(),
            index_offset: u64_at(mmap, 40).unwrap(),
            heads_offset: u64_at(mmap, 48).unwrap(),
            rank_index_offset: u64_at(mmap, 56).unwrap(),
            flags: u32_at(mmap, 64).unwrap(),
        })
    }

    pub fn header(&self) -> VfrHeader {
        self.header
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn len(&self) -> u64 {
        self.header.entry_count
    }
    pub fn is_empty(&self) -> bool {
        self.header.entry_count == 0
    }
    pub fn kind(&self) -> u32 {
        self.header.kind
    }
    fn has_flags(&self) -> bool {
        self.header.flags & VFR_FLAG_HAS_FLAGS != 0
    }

    fn block_offset(&self, i: usize) -> Option<u64> {
        u64_at(&self.index, i * VFR_BLOCK_ENTRY_LEN)
    }

    fn block_head(&self, i: usize) -> Option<&str> {
        let base = i * VFR_BLOCK_ENTRY_LEN;
        let off = u32_at(&self.index, base + 8)? as usize;
        let len = u16_at(&self.index, base + 12)? as usize;
        std::str::from_utf8(self.heads.get(off..off + len)?).ok()
    }

    /// 第 `i` 个块的记录区间 `[start, end)`。
    fn block_range(&self, i: usize) -> Option<(usize, usize)> {
        let start = self.block_offset(i)? as usize;
        let end = if i + 1 < self.header.block_count as usize {
            self.block_offset(i + 1)? as usize
        } else {
            self.mmap.len()
        };
        if start > end || end > self.mmap.len() {
            return None;
        }
        Some((start, end))
    }

    /// 解析 `off` 处的记录，同时返回下一条记录的偏移。
    fn parse_record(&self, off: usize) -> Option<(Hit, usize)> {
        let mut p = off;
        let wlen = crate::artifact::read_varint(&self.mmap, &mut p)? as usize;
        let wbytes = self.mmap.get(p..p + wlen)?;
        p += wlen;
        let word = std::str::from_utf8(wbytes).ok()?.to_string();
        let count = crate::artifact::read_varint(&self.mmap, &mut p)?;
        let rank = crate::artifact::read_varint(&self.mmap, &mut p)? as u32;
        let flags = if self.has_flags() {
            let b = *self.mmap.get(p)?;
            p += 1;
            b
        } else {
            0
        };
        Some((Hit { word, count, rank, flags }, p))
    }

    /// 精确查词。未收录返回 `None`。
    pub fn lookup(&self, word: &str) -> Option<Hit> {
        let bc = self.header.block_count as usize;
        if bc == 0 {
            return None;
        }
        let target = word.as_bytes();
        // 二分找第一个「块首词 > target」的块，则目标只可能落在它前一块
        let (mut lo, mut hi) = (0usize, bc);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.block_head(mid) {
                Some(h) if h.as_bytes() <= target => lo = mid + 1,
                Some(_) => hi = mid,
                None => return None,
            }
        }
        if lo == 0 {
            return None;
        }
        let (mut p, end) = self.block_range(lo - 1)?;
        while p < end {
            let (hit, next) = self.parse_record(p)?;
            match hit.word.as_str().cmp(word) {
                std::cmp::Ordering::Equal => return Some(hit),
                // 块内按词序排列，一旦超过就不可能再出现
                std::cmp::Ordering::Greater => return None,
                std::cmp::Ordering::Less => {}
            }
            if next <= p {
                return None;
            }
            p = next;
        }
        None
    }

    pub fn contains(&self, word: &str) -> bool {
        self.lookup(word).is_some()
    }

    /// 按排名取条目（1 起）。需要产物带有排名索引。
    pub fn entry_at_rank(&self, rank: u32) -> Option<Hit> {
        if rank == 0 || rank as u64 > self.header.entry_count {
            return None;
        }
        let rio = self.header.rank_index_offset;
        if rio == 0 {
            return None;
        }
        let off = u32_at(&self.mmap, rio as usize + (rank as usize - 1) * 4)? as usize;
        self.parse_record(off).map(|(h, _)| h)
    }

    /// 取排名 `[from, from+limit)` 的一段，供排行榜翻页。
    pub fn range_by_rank(&self, from: u32, limit: u32) -> Vec<Hit> {
        let mut out = Vec::with_capacity(limit as usize);
        for r in from..from.saturating_add(limit) {
            match self.entry_at_rank(r) {
                Some(h) => out.push(h),
                None => break,
            }
        }
        out
    }

    /// 只取记录里的 `count`，跳过词字符串。
    ///
    /// 画覆盖率曲线要顺序读几十万到几百万条记录，用 [`Self::parse_record`] 会为每条
    /// 记录分配一个 `String`——那是纯粹的白烧。
    fn record_count_at(&self, off: usize) -> Option<(u64, usize)> {
        let mut p = off;
        let wlen = crate::artifact::read_varint(&self.mmap, &mut p)? as usize;
        p = p.checked_add(wlen)?;
        let count = crate::artifact::read_varint(&self.mmap, &mut p)?;
        Some((count, p))
    }

    /// 累计覆盖率曲线：按对数间隔采样 `(rank, 累计覆盖率)`。
    ///
    /// 前端用它把「覆盖率目标」反解成排名阈值，从而支持**按覆盖率分组**（而不是
    /// 只能按绝对排名分组）。采样点刻意在头部密、尾部疏——Zipf 分布的有用信息
    /// 几乎全在头部，等距采样会把前 100 名挤成一个点。
    pub fn coverage_curve(&self, max_points: usize) -> Vec<(u32, f64)> {
        let n = self.header.entry_count;
        if n == 0 || self.header.rank_index_offset == 0 {
            return Vec::new();
        }
        let total = self.header.total_tokens.max(1) as f64;
        let max_points = max_points.clamp(8, 4000);

        // 采样目标：每个数量级约 max_points/decades 个点，即 1,2,3…10,12,15,19,24…
        let decades = (n as f64).log10().max(1.0);
        let per_decade = (max_points as f64 / decades).max(2.0);
        let mut targets: Vec<u32> = Vec::new();
        let mut k = 0f64;
        while targets.len() <= max_points {
            let r = (10f64.powf(k / per_decade).round() as u64).max(1);
            if r > n {
                break;
            }
            if targets.last() != Some(&(r as u32)) {
                targets.push(r as u32);
            }
            k += 1.0;
        }
        if targets.last() != Some(&(n as u32)) {
            targets.push(n as u32);
        }

        let rio = self.header.rank_index_offset as usize;
        let mut cum: u64 = 0;
        let mut out: Vec<(u32, f64)> = Vec::with_capacity(targets.len());
        let mut ti = 0usize;
        for rank in 1..=n as u32 {
            let off = match u32_at(&self.mmap, rio + (rank as usize - 1) * 4) {
                Some(o) => o as usize,
                None => break,
            };
            match self.record_count_at(off) {
                Some((c, _)) => cum += c,
                None => break,
            }
            while ti < targets.len() && targets[ti] == rank {
                out.push((rank, cum as f64 / total));
                ti += 1;
            }
            if ti >= targets.len() {
                break;
            }
        }
        out
    }

    /// 按**词首前缀**顺序遍历，返回最多 `limit` 个命中。
    ///
    /// 记录区本来就按词的字节序排列，所以前缀匹配的词在文件里是连续的一段：
    /// 先二分定位到首个块首词 >= prefix 的块，再顺序扫到不再匹配前缀为止。
    /// 排行榜的搜索框走这条路径，不会为了找几个词去遍历整张表。
    pub fn prefix_scan(&self, prefix: &str, limit: usize) -> Vec<Hit> {
        let mut out = Vec::new();
        if prefix.is_empty() || limit == 0 {
            return out;
        }
        let bc = self.header.block_count as usize;
        if bc == 0 {
            return out;
        }
        // 第一个块首词 >= prefix 的块
        let (mut lo, mut hi) = (0usize, bc);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.block_head(mid) {
                Some(h) if h.as_bytes() < prefix.as_bytes() => lo = mid + 1,
                Some(_) => hi = mid,
                None => return out,
            }
        }
        // 前一块里也可能有以 prefix 开头、但比该块首词小的词
        let start_block = lo.saturating_sub(1);
        'blocks: for b in start_block..bc {
            let (mut p, end) = match self.block_range(b) {
                Some(r) => r,
                None => break,
            };
            while p < end {
                let (hit, next) = match self.parse_record(p) {
                    Some(v) => v,
                    None => break 'blocks,
                };
                if hit.word.starts_with(prefix) {
                    out.push(hit);
                    if out.len() >= limit {
                        return out;
                    }
                } else if hit.word.as_str() > prefix {
                    // 已越过前缀区间
                    return out;
                }
                if next <= p {
                    break 'blocks;
                }
                p = next;
            }
        }
        out
    }
}

// ---------------------------------------------------------------- 数据集

/// 一句话里某个 token 的完整频率信息。
#[derive(Debug, Clone, Serialize)]
pub struct TokenInfo {
    pub text: String,
    pub byte_start: usize,
    pub byte_end: usize,
    /// 是否为计入统计的内容 token
    pub accepted: bool,
    /// 是否单个汉字（此时查字表）
    pub single_cjk: bool,
    /// 实际查的表：`word` / `char`，未收录或非内容为 `""`
    pub table: String,
    pub count: Option<u64>,
    pub rank: Option<u32>,
    /// 占全部 token 的百分比
    pub pct: Option<f64>,
    /// 分组下标 0..7；`None` 表示语料库未收录
    pub tier: Option<usize>,
    pub tier_name: Option<String>,
    pub in_dict: Option<bool>,
    pub from_user: Option<bool>,
    /// 各分域里的排名（`None` 表示该域未收录）
    pub domain_ranks: Vec<(String, Option<u32>)>,
}

/// 全库表 + 分域表的集合。
pub struct Dataset {
    pub root: PathBuf,
    pub meta: Meta,
    pub word: VfrTable,
    pub char: VfrTable,
    /// (域名, 词表, 字表)
    pub domains: Vec<(String, VfrTable, VfrTable)>,
}

impl Dataset {
    /// 按 `meta.tables[].path` 形式定位表：`full/word`、`full/char`、
    /// `domains/<域>/word`、`domains/<域>/char`。
    ///
    /// 前端表管理器与 `tier_curve` 命令都用这个字符串指代表，因此这里是唯一解析点。
    pub fn table_by_path(&self, path: &str) -> Option<&VfrTable> {
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            ["full", kind] => Some(if *kind == "char" { &self.char } else { &self.word }),
            ["domains", name, kind] => self
                .domains
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|(_, w, c)| if *kind == "char" { c } else { w }),
            _ => None,
        }
    }

    /// 打开输出目录（`vocfreq scan --out` 的产物目录）。
    pub fn open(root: &Path) -> Result<Self> {
        let meta_path = root.join("meta.json");
        let bytes = std::fs::read(&meta_path).map_err(|e| {
            Error::Format(format!(
                "读取 {} 失败（该目录不是 vocfreq 产物目录？）: {e}",
                meta_path.display()
            ))
        })?;
        let meta: Meta = serde_json::from_slice(&bytes)?;
        let word = VfrTable::open(&root.join("full").join("word.vfr"))?;
        let char = VfrTable::open(&root.join("full").join("char.vfr"))?;

        let mut domains = Vec::new();
        for d in &meta.domains {
            let p = root.join("domains").join(&d.name);
            let wv = p.join("word.vfr");
            let cv = p.join("char.vfr");
            if wv.exists() && cv.exists() {
                domains.push((d.name.clone(), VfrTable::open(&wv)?, VfrTable::open(&cv)?));
            }
        }
        Ok(Dataset { root: root.to_path_buf(), meta, word, char, domains })
    }

    fn tiers_of(&self, table: &str) -> Vec<crate::rank::Tier> {
        let key = if table == "char" { "full/char" } else { "full/word" };
        self.meta
            .tables
            .iter()
            .find(|t| t.path == key)
            .map(|t| t.tiers.clone())
            .unwrap_or_else(|| {
                if table == "char" {
                    crate::rank::default_char_tiers()
                } else {
                    crate::rank::default_word_tiers()
                }
            })
    }

    /// 分析一句话：分词 → 查频次 → 定分组 → 附分域排名。
    ///
    /// 单字 token 查字表，其余查词表；都查不到则 `tier = None`，界面应显示为
    /// 「语料库未收录」，与「极少」区分开。
    pub fn analyze(&self, tk: &Tokenizer, text: &str) -> Vec<TokenInfo> {
        let segs = tk.segment(text);
        let word_tiers = self.tiers_of("word");
        let char_tiers = self.tiers_of("char");
        let mut out = Vec::with_capacity(segs.len());

        for s in segs {
            let mut info = TokenInfo {
                text: s.text.clone(),
                byte_start: s.byte_start,
                byte_end: s.byte_end,
                accepted: s.accepted,
                single_cjk: s.single_cjk,
                table: String::new(),
                count: None,
                rank: None,
                pct: None,
                tier: None,
                tier_name: None,
                in_dict: None,
                from_user: None,
                domain_ranks: Vec::new(),
            };

            if s.accepted {
                // 单字查字表，其余查词表。字表不写 flags（记录里没有 flags 字节），
                // 所以「是否在 jieba 词典内」必须直接问分词器，不能从字表记录里读，
                // 否则每个单字都会被误标成「词典外」。
                let (hit, table, tiers, total) = if s.single_cjk {
                    (
                        self.char.lookup(&s.text),
                        "char",
                        &char_tiers,
                        self.char.header().total_tokens,
                    )
                } else {
                    (
                        self.word.lookup(&s.text),
                        "word",
                        &word_tiers,
                        self.word.header().total_tokens,
                    )
                };
                info.table = table.to_string();

                if let Some(h) = &hit {
                    info.count = Some(h.count);
                    info.rank = Some(h.rank);
                    info.pct = Some(if total > 0 {
                        h.count as f64 * 100.0 / total as f64
                    } else {
                        0.0
                    });
                    let ti = tier_of(h.rank, tiers);
                    info.tier = Some(ti);
                    info.tier_name = Some(tiers[ti].name.clone());
                }

                if table == "word" {
                    if let Some(h) = &hit {
                        info.in_dict = Some(h.flags & crate::tokenize::FLAG_IN_DICT != 0);
                        info.from_user = Some(h.flags & crate::tokenize::FLAG_FROM_USER != 0);
                    }
                } else {
                    info.in_dict = Some(tk.has_word(&s.text));
                    info.from_user = Some(tk.is_user_word(&s.text));
                }

                for (name, dw, dc) in &self.domains {
                    let r = if s.single_cjk { dc.lookup(&s.text) } else { dw.lookup(&s.text) };
                    info.domain_ranks.push((name.clone(), r.map(|h| h.rank)));
                }
            }
            out.push(info);
        }
        out
    }
}

/// 由覆盖率曲线反解某个累计覆盖率目标对应的排名阈值。
///
/// 在 `log10(rank)` 上做线性插值：曲线本身就是对数采样的，按排名线性插值会在
/// 头部产生很大误差（1→2 名与 100000→100001 名的覆盖率跨度完全不同）。
///
/// 前端要用「按覆盖率分组」时需要同一套换算，TS 侧有一份等价实现。
pub fn rank_for_coverage(curve: &[(u32, f64)], target: f64) -> Option<u32> {
    if curve.is_empty() {
        return None;
    }
    if target <= curve[0].1 {
        return Some(curve[0].0);
    }
    for w in curve.windows(2) {
        let (r0, c0) = w[0];
        let (r1, c1) = w[1];
        if target <= c1 {
            if (c1 - c0).abs() < 1e-12 {
                return Some(r1);
            }
            let t = (target - c0) / (c1 - c0);
            let lr = (r0 as f64).log10() + t * ((r1 as f64).log10() - (r0 as f64).log10());
            return Some(10f64.powf(lr).round().max(1.0) as u32);
        }
    }
    Some(curve.last().unwrap().0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rank::{rank_entries, RankedEntry};
    use rustc_hash::FxHashMap;

    fn tmpfile(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("vocfreq_test_{}_{}", std::process::id(), name));
        p
    }

    fn sample_entries() -> Vec<RankedEntry> {
        // 故意造出长短不一、含多字节的键，覆盖块边界与二分
        let mut m: FxHashMap<Box<str>, u64> = FxHashMap::default();
        let words = [
            "的", "是", "在", "中国", "人工智能", "中华人民共和国", "元宇宙", "zzz", "abc",
            "龘", "㐀", "𠀀", "a", "ab", "abcde", "abcdefghijklmnopqrstu",
        ];
        for (i, w) in words.iter().enumerate() {
            m.insert((*w).into(), (words.len() - i) as u64 * 7);
        }
        for i in 0..500 {
            m.insert(format!("测试词{i:04}").into(), (i + 1) as u64);
        }
        rank_entries(&m, &|_| 0)
    }

    fn build(name: &str, entries: &[RankedEntry], with_flags: bool) -> VfrTable {
        let p = tmpfile(name);
        crate::artifact::write_vfr(&p, KIND_WORD, entries, 123_456, with_flags).unwrap();
        let t = VfrTable::open(&p).unwrap();
        std::fs::remove_file(&p).ok();
        t
    }

    #[test]
    fn lookup_finds_every_entry_and_rejects_absent() {
        let entries = sample_entries();
        let t = build("lookup", &entries, true);
        assert_eq!(t.len(), entries.len() as u64);
        for e in &entries {
            let hit = t.lookup(&e.word).unwrap_or_else(|| panic!("查不到 {:?}", e.word));
            assert_eq!(hit.count, e.count, "词 {:?} 频次不符", e.word);
            assert_eq!(hit.rank, e.rank, "词 {:?} 排名不符", e.word);
        }
        for miss in ["不存在的词", "", "zzzz", "人工智能x", "龘龘"] {
            assert!(t.lookup(miss).is_none(), "{miss:?} 不应被查到");
        }
    }

    #[test]
    fn lookup_handles_block_boundaries_exhaustively() {
        // 对每个词做前缀探测：只有「本身也是词条」的前缀才应命中，其余前缀必须查不到。
        // 这条测试专门覆盖二分定位到错误块的情况（前缀恰好等于某块的块首词）。
        let entries = sample_entries();
        let t = build("boundary", &entries, true);
        let is_entry = |s: &str| entries.iter().any(|e| &*e.word == s);
        for e in &entries {
            let mut s = String::new();
            for ch in e.word.chars() {
                s.push(ch);
                if s.as_str() == &*e.word {
                    continue;
                }
                let hit = t.lookup(&s);
                if is_entry(&s) {
                    assert!(hit.is_some(), "前缀 {s:?} 本身是词条，应当查得到");
                } else {
                    assert!(hit.is_none(), "前缀 {s:?} 不应被查到");
                }
            }
        }
    }

    #[test]
    fn rank_index_returns_entries_in_rank_order() {
        let entries = sample_entries();
        let t = build("rank", &entries, true);
        assert_eq!(t.entry_at_rank(0), None);
        for (i, e) in entries.iter().enumerate() {
            let hit = t.entry_at_rank(e.rank).unwrap();
            assert_eq!(&hit.word, &*e.word, "排名 {} 的条目不符", e.rank);
            assert_eq!(hit.rank, (i + 1) as u32);
        }
        assert_eq!(t.entry_at_rank(entries.len() as u32 + 1), None);
        let page = t.range_by_rank(1, 5);
        assert_eq!(page.len(), 5);
        assert_eq!(page[0].rank, 1);
    }

    #[test]
    fn flags_roundtrip() {
        let mut m: FxHashMap<Box<str>, u64> = FxHashMap::default();
        m.insert("元宇宙".into(), 10);
        let entries = rank_entries(&m, &|_| crate::tokenize::FLAG_FROM_USER);
        let t = build("flags", &entries, true);
        let hit = t.lookup("元宇宙").unwrap();
        assert_eq!(hit.flags, crate::tokenize::FLAG_FROM_USER);
    }

    #[test]
    fn empty_table_is_valid() {
        let p = tmpfile("empty");
        crate::artifact::write_vfr(&p, KIND_WORD, &[], 0, true).unwrap();
        let t = VfrTable::open(&p).unwrap();
        assert_eq!(t.len(), 0);
        assert!(t.lookup("任何").is_none());
        assert_eq!(t.entry_at_rank(1), None);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        let p = tmpfile("bad");
        std::fs::write(&p, b"NOTAVFR!").unwrap();
        assert!(VfrTable::open(&p).is_err());
        std::fs::write(&p, vec![0u8; 40]).unwrap();
        assert!(VfrTable::open(&p).is_err());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn token_info_field_names_are_a_frozen_contract() {
        // 前端按 snake_case 读取这些字段名（见 docs/DESIGN.md §8.1）。
        // 一旦改名字段，界面会静默拿到 undefined——所以这里把契约钉死。
        let t = TokenInfo {
            text: "中國".into(),
            byte_start: 0,
            byte_end: 6,
            accepted: true,
            single_cjk: false,
            table: "word".into(),
            count: Some(12),
            rank: Some(34),
            pct: Some(0.5),
            tier: Some(2),
            tier_name: Some("较多".into()),
            in_dict: Some(true),
            from_user: Some(false),
            domain_ranks: vec![("news".into(), Some(7))],
        };
        let v = serde_json::to_value(&t).unwrap();
        for k in [
            "text",
            "byte_start",
            "byte_end",
            "accepted",
            "single_cjk",
            "table",
            "count",
            "rank",
            "pct",
            "tier",
            "tier_name",
            "in_dict",
            "from_user",
            "domain_ranks",
        ] {
            assert!(v.get(k).is_some(), "TokenInfo 序列化缺少字段 {k}：{v}");
        }
    }

    #[test]
    fn vfr_table_reports_kind_and_totals() {
        let entries = sample_entries();
        let p = tmpfile("kind");
        crate::artifact::write_vfr(&p, KIND_CHAR, &entries, 999, false).unwrap();
        let t = VfrTable::open(&p).unwrap();
        assert_eq!(t.kind(), KIND_CHAR);
        assert_eq!(t.header().total_tokens, 999);
        // 不带 flags 的表：flags 一律读成 0，而不是从别处借位
        assert_eq!(t.lookup(&entries[0].word).unwrap().flags, 0);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn prefix_scan_returns_only_matching_words() {
        let entries = sample_entries();
        let t = build("prefix", &entries, true);
        let got = t.prefix_scan("ab", 50);
        assert!(!got.is_empty(), "应能搜到以 ab 开头的词");
        for h in &got {
            assert!(h.word.starts_with("ab"), "前缀搜索返回了不匹配的词 {:?}", h.word);
        }
        // 「测试词」有 500 条，限制条数要生效
        let many = t.prefix_scan("测试词", 10);
        assert_eq!(many.len(), 10);
        // 搜不到的前缀
        assert!(t.prefix_scan("zzzzzz", 10).is_empty());
        // 空前缀不返回任何东西（否则等于全表遍历）
        assert!(t.prefix_scan("", 10).is_empty());
    }

    #[test]
    fn prefix_scan_covers_words_before_first_matching_block() {
        // 造一批词让「首个块首词 >= 前缀」落在块中间，验证回溯前一块的逻辑
        let mut m: FxHashMap<Box<str>, u64> = FxHashMap::default();
        for i in 0..600u32 {
            m.insert(format!("aa{i:05}").into(), 1000 - i as u64);
        }
        for i in 0..5u32 {
            m.insert(format!("ab{i:05}").into(), 10 - i as u64);
        }
        let entries = rank_entries(&m, &|_| 0);
        let t = build("prefix_edge", &entries, true);
        let got = t.prefix_scan("ab", 50);
        assert_eq!(got.len(), 5, "应恰好搜到 5 个 ab 开头的词，实际 {got:?}");
    }

    #[test]
    fn coverage_curve_is_monotonic_and_reaches_one() {
        let entries = sample_entries();
        let total: u64 = entries.iter().map(|e| e.count).sum();
        let p = tmpfile("curve");
        crate::artifact::write_vfr(&p, KIND_WORD, &entries, total, true).unwrap();
        let t = VfrTable::open(&p).unwrap();

        let c = t.coverage_curve(128);
        assert!(!c.is_empty(), "曲线不应为空");
        assert_eq!(c[0].0, 1, "曲线应从第 1 名开始");
        assert_eq!(c.last().unwrap().0 as u64, t.len(), "曲线应覆盖到最后一名");

        let mut prev = 0.0f64;
        for (rank, cov) in &c {
            assert!(
                *cov >= prev - 1e-12,
                "累计覆盖率必须单调不减：rank={rank} 覆盖={cov} < 上一点 {prev}"
            );
            assert!((0.0..=1.0 + 1e-12).contains(cov), "覆盖率越界：{cov}");
            prev = *cov;
        }
        assert!((prev - 1.0).abs() < 1e-9, "末端覆盖率应为 1，实际 {prev}");

        // 头部采样必须比尾部密：等距采样会把前 100 名挤成一个点，那就没法按覆盖率调阈值了
        let head_gap = c[1].0 - c[0].0;
        let tail_gap = c[c.len() - 1].0 - c[c.len() - 2].0;
        assert!(
            head_gap <= tail_gap,
            "头部采样间隔 {head_gap} 应不大于尾部 {tail_gap}"
        );
    }

    #[test]
    fn coverage_curve_on_empty_table_is_empty() {
        let p = tmpfile("curve_empty");
        crate::artifact::write_vfr(&p, KIND_WORD, &[], 0, true).unwrap();
        let t = VfrTable::open(&p).unwrap();
        assert!(t.coverage_curve(64).is_empty());
        std::fs::remove_file(&p).ok();
    }
}
