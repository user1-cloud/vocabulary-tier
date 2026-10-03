//! `.vfr` 查询：mmap 打开、精确二分查词、按排名取条目。
//!
//! 还提供 [`Dataset`]：打开一个产物目录里的**全部**表（每个表组各自一张词频表 +
//! 一张字表），并对一句话做完整分析（分词 → 查频次 → 定分组 → 附各表排名）。
//! 前端的划句分析直接调它，命令行 `segment` 也走同一条代码路径，因此两条链路的
//! 行为必然一致。
//!
//! ## schema v3：表组铺平
//!
//! 从前产物是「一张全库表 + N 张表组」，全库表享有特权（"总体频率"只查它）。
//! 现在**每个表组都平等**：`full`（全量）只是其中一个，相加出来的新表也是同级
//! 的一个表组。谁负责回答"这个词有多常见"由用户指定的 **主表组**决定
//! （见 [`Dataset::primary_scope`]），其余表组只做对比。
//!
//! 读取端对**老布局（schema < 3）明确报错并让用户重扫**，不试图迁移：老产物把
//! 表组与类型糊在 `path` 里（`domains/news/word`），新读取端按"每个表组一个
//! 目录"去找文件，硬读只会得到一句莫名其妙的「文件不存在」。

use std::fs::File;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use serde::Serialize;

use crate::artifact::{
    Meta, VFR_BLOCK_ENTRY_LEN, VFR_FLAG_HAS_FLAGS, VFR_HEADER_LEN, VFR_MAGIC, VFR_VERSION,
    table_key,
};
use crate::rank::tier_of;
use crate::tokenize::Tokenizer;
use crate::{Error, Result};

pub const KIND_WORD: u32 = 0;
pub const KIND_CHAR: u32 = 1;

/// 单个 token 最多对多少个表组做对比查询。
///
/// 每一步都是一次二分查词（约 15 µs），表组多了以后单句成本会线性上涨。
/// 划句分析的对象是句子不是文章，512 已经远超正常输入；超长输入（整篇文章粘进
/// 小窗）直接不附对比信息，而不是让界面卡住 —— 着色与分组本来只看主表组。
const MAX_COMPARE_TOKENS: usize = 512;

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
    /// 记录区起始偏移（0 = 老写入端没记，按块索引推断）
    pub records_offset: u64,
    /// 记录区字节数（0 = 老写入端没记）
    pub records_bytes: u64,
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
            return Err(Error::Format(format!(
                "{} 块首词区范围非法",
                path.display()
            )));
        }
        let heads = mmap[header.heads_offset as usize..heads_end].to_vec();

        Ok(VfrTable {
            mmap,
            header,
            index,
            heads,
            path: path.display().to_string(),
        })
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
            return Err(bad(&format!(
                "版本 {version} 与当前支持的 {VFR_VERSION} 不匹配"
            )));
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
            // 68/76 覆盖了原来 reserved 区的前 16 字节。老写入端在那儿写的是 0，
            // 于是这里得到 0，[`Self::records_end`] 会退回到「按块索引推断」。
            records_offset: u64_at(mmap, 68).unwrap_or(0),
            records_bytes: u64_at(mmap, 76).unwrap_or(0),
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

    /// 记录区结束的偏移。
    ///
    /// 优先用头里记的 `records_offset + records_bytes`；老写入端没有这两个字段
    /// （读到 0），才退回「文件长度」这个不稳的推断。
    fn records_end(&self) -> usize {
        if self.header.records_bytes > 0 {
            let end = self
                .header
                .records_offset
                .saturating_add(self.header.records_bytes);
            return (end as usize).min(self.mmap.len());
        }
        self.mmap.len()
    }

    /// 第 `i` 个块的记录区间 `[start, end)`。
    fn block_range(&self, i: usize) -> Option<(usize, usize)> {
        let start = self.block_offset(i)? as usize;
        let end = if i + 1 < self.header.block_count as usize {
            self.block_offset(i + 1)? as usize
        } else {
            self.records_end()
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
        Some((
            Hit {
                word,
                count,
                rank,
                flags,
            },
            p,
        ))
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

    /// 顺序遍历整张表（按词序），主要用于**相加**与全表比对。
    ///
    /// 与 [`Self::prefix_scan`] 共用同一套解析：记录区本来就是按词序落盘的，所以
    /// 顺着块一路读下去就是全表。逐条 `String` 分配在这里是可以接受的——相加一张
    /// 380 万词的表实测在秒级，而它是一次性操作。
    pub fn positions(&self) -> Positions<'_> {
        Positions {
            table: self,
            block: 0,
            pos: 0,
            end: 0,
        }
    }

    /// 把这张表**原样复制**到另一个路径（同 kind、同 flags 布局）。
    ///
    /// 相加时用来搬运不需要合并的那一类表（例如只相加词频表时，字表直接拷）。
    /// 比"解码一遍再编码一遍"快得多，而且不会因为编解码往返引入任何差异。
    pub fn clone_into(&self, path: &Path) -> Result<u64> {
        std::fs::write(path, &self.mmap)?;
        Ok(self.mmap.len() as u64)
    }

    /// 这张表自带的行数/占比信息：排名 → 排名占该表全部条目的百分比（前%）。
    pub fn top_pct(&self, rank: u32) -> f64 {
        crate::rank::pct_for_rank(rank, self.header.entry_count)
    }
}

/// [`VfrTable::positions`] 的迭代器。
///
/// 不缓存任何 `Hit`，状态只有「当前块 + 块内位置」，因此对一张 380 万词的表做
/// 整表遍历是纯流式的，不会把所有词条攒进内存。
pub struct Positions<'a> {
    table: &'a VfrTable,
    block: usize,
    pos: usize,
    end: usize,
}

impl Iterator for Positions<'_> {
    type Item = Hit;

    fn next(&mut self) -> Option<Hit> {
        loop {
            if self.pos >= self.end {
                // 进入下一个块。**不能用 done 挡住这一步**：块号到界时说明"没有
                // 下一块了"，但最后一块本身还没被读 —— 早先的写法在这里直接返回
                // None，于是整表遍历会静默丢掉最后一个块（实测 500 条只出来 449 条）。
                if self.block >= self.table.header.block_count as usize {
                    return None;
                }
                let (p, e) = self.table.block_range(self.block)?;
                self.block += 1;
                self.pos = p;
                self.end = e;
                if self.pos >= self.end {
                    continue;
                }
            }
            let (hit, next) = self.table.parse_record(self.pos)?;
            if next <= self.pos {
                // 解析不出下一条 = 记录区损坏，就此打住而不是死循环
                return None;
            }
            self.pos = next;
            return Some(hit);
        }
    }
}

/// 「读得到条目、能报条目数」的抽象。
///
/// 目前只有 [`VfrTable`] 一个实现，且 [`TableRef`] 直接用具体类型。留出这层是因为
/// **相加有两种落地方式**：现在选的是"物化成真实 `.vfr`"（`compose` 模块走这条路），
/// 而"零拷贝虚拟合成"（读时跨表归并、不写任何文件）只需要再实现一次这个 trait ——
/// `TableMeta::source_tables` 就是为它留的接口。
pub trait TableReader {
    fn lookup(&self, word: &str) -> Option<Hit>;
    fn rank(&self, word: &str) -> Option<u32> {
        self.lookup(word).map(|h| h.rank)
    }
    fn entry_count(&self) -> u64;
    fn total_tokens(&self) -> u64;
}

impl TableReader for VfrTable {
    fn lookup(&self, word: &str) -> Option<Hit> {
        VfrTable::lookup(self, word)
    }
    fn entry_count(&self) -> u64 {
        self.len()
    }
    fn total_tokens(&self) -> u64 {
        self.header.total_tokens
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
    /// **前%**：`排名 ÷ 该表条目数 × 100`，也就是「这个词在这张表里排在前百分之几」。
    ///
    /// 这是稀有度的默认口径（`TierMethod::top_pct`）。与 [`Self::pct`] 完全是两回事：
    /// `pct` 是"这个词占了多少正文"，`top_pct` 是"它比多少词常见"。
    pub top_pct: Option<f64>,
    /// 主表的总条目数（算前% 的分母，界面自己反解阈值时也要用）
    pub entries: Option<u64>,
    /// 分组下标 0..7；`None` 表示语料库未收录
    pub tier: Option<usize>,
    pub tier_name: Option<String>,
    pub in_dict: Option<bool>,
    pub from_user: Option<bool>,
    /// 各表组（表）里的排名对比。表组多时只填前 [`MAX_COMPARE_TOKENS`] 个 token。
    pub table_ranks: Vec<TableRank>,
}

/// 某个 token 在某张表里的排名（划句分析里的「各表对比」一列）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableRank {
    /// 表组：`full`、`news`、`相加：财经`…
    pub scope: String,
    pub kind: String,
    /// 该表里的排名；`None` = 这张表没收录这个词
    pub rank: Option<u32>,
    /// 该表里的前%（排名 ÷ 条目数 × 100）
    pub top_pct: Option<f64>,
    /// 该表的条目数
    pub entries: u64,
    /// 该表里的频次
    pub count: Option<u64>,
}

/// 产物目录里的一张表 + 它在 `meta.json` 里的描述。
///
/// `vfr` 直接用具体类型 [`VfrTable`] 而不是 trait 对象：覆盖率曲线、按排名翻页、
/// 前缀搜索都只有它一个实现，包一层 `dyn` 只会让每个调用点都要多一次动态分发。
/// 将来真要做"零拷贝虚拟合成"，再把它换成 `Box<dyn TableReader>` 即可。
pub struct TableRef {
    pub scope: String,
    pub kind: String,
    pub entries: u64,
    pub total_tokens: u64,
    pub vfr: VfrTable,
}

impl TableRef {
    /// 表身份 `表组/类型`。
    pub fn key(&self) -> String {
        table_key(&self.scope, &self.kind)
    }

    /// 精确查词（转发给底下的表）。
    pub fn lookup(&self, word: &str) -> Option<Hit> {
        self.vfr.lookup(word)
    }
}

/// 一个产物目录里的全部表。
pub struct Dataset {
    pub root: PathBuf,
    pub meta: Meta,
    /// 全部表（每个表组各一张词频表 + 一张字表），顺序与 `meta.tables` 一致
    pub tables: Vec<TableRef>,
    /// 主表组：回答「这个词有多常见」的那一张（见 [`Self::primary_scope`]）
    pub primary_scope: String,
}

impl std::fmt::Debug for Dataset {
    /// 手写而不是 derive：`VfrTable` 里有一个 100 MB 级的 mmap，derive 出来的
    /// Debug 会把整张表按字节打印出来（曾经把一条断言失败的输出炸成几百 MB）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dataset")
            .field("root", &self.root)
            .field("primary_scope", &self.primary_scope)
            .field(
                "tables",
                &self.tables.iter().map(|t| t.key()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Dataset {
    /// 打开输出目录（`vocfreq scan --out` / `vocfreq compose --out` 的产物目录）。
    ///
    /// 主表组默认 `full`；`full` 不存在时退到 `meta.scopes()` 的第一个。
    /// 要指定别的主表组用 [`Self::open_with_primary`]。
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_primary(root, None)
    }

    /// 同 [`Self::open`]，但显式指定主表组。
    pub fn open_with_primary(root: &Path, primary: Option<&str>) -> Result<Self> {
        let meta_path = root.join("meta.json");
        let bytes = std::fs::read(&meta_path).map_err(|e| {
            Error::Format(format!(
                "读取 {} 失败（该目录不是 vocfreq 产物目录？）: {e}",
                meta_path.display()
            ))
        })?;
        let meta: Meta = serde_json::from_slice(&bytes)?;

        if meta.schema_version < crate::artifact::SCHEMA_VERSION {
            return Err(Error::Format(format!(
                "{} 是 schema v{} 的老产物（现在是 v{}）。\n\
                 从 v3 起「全库表」与「表组」被铺平成同级的**表组**：\
                 每个表组一个目录（full/、news/…），老产物把表组与类型糊在 \
                 `full/word`、`domains/news/word` 这样的路径里，读不了。\n\
                 请用同一套语料库与词典重新统计一次（老产物的数据无法就地迁移）。",
                meta_path.display(),
                meta.schema_version,
                crate::artifact::SCHEMA_VERSION
            )));
        }

        let mut tables: Vec<TableRef> = Vec::new();
        for t in &meta.tables {
            if t.path.is_empty() || t.path.contains('/') || t.path.contains('\\') {
                return Err(Error::Format(format!(
                    "meta.json 里的表 {} 的表组名非法（不允许包含路径分隔符）",
                    t.path
                )));
            }
            let file = root.join(&t.path).join(format!("{}.vfr", t.kind));
            if !file.exists() {
                // 半截产物（拷贝中断、被误删）要说清楚缺的是哪一张
                return Err(Error::Format(format!(
                    "{} 里记录的 {} 表组 {} 表在磁盘上不存在：{}",
                    meta_path.display(),
                    t.path,
                    t.kind,
                    file.display()
                )));
            }
            let vfr = VfrTable::open(&file)?;
            tables.push(TableRef {
                scope: t.path.clone(),
                kind: t.kind.clone(),
                entries: vfr.len(),
                total_tokens: vfr.header().total_tokens,
                vfr,
            });
        }
        if tables.is_empty() {
            return Err(Error::Format(format!(
                "{} 里一张表都没有（tables 为空），这个目录是废的",
                meta_path.display()
            )));
        }

        let available = meta.scopes();
        let primary_scope = primary
            .filter(|p| !p.is_empty() && available.iter().any(|s| s == p))
            .map(|p| p.to_string())
            .or_else(|| available.first().cloned())
            .unwrap_or_else(|| crate::artifact::SCOPE_FULL.to_string());

        Ok(Dataset {
            root: root.to_path_buf(),
            meta,
            tables,
            primary_scope,
        })
    }

    /// 全部**表组** id（去重，`full` 优先）。
    pub fn scopes(&self) -> Vec<String> {
        self.meta.scopes()
    }

    /// 找一张表：`表组 + 类型`。
    pub fn table(&self, scope: &str, kind: &str) -> Option<&TableRef> {
        self.tables
            .iter()
            .find(|t| t.scope == scope && t.kind == kind)
    }

    /// 按表身份（`表组/类型`）找一张表。
    ///
    /// **全仓库唯一的解析点**：前端表管理器、`tier_curve` 命令、CLI 的 `curve`
    /// 都靠这个字符串指代表。
    pub fn table_by_key(&self, key: &str) -> Option<&TableRef> {
        let (scope, kind) = crate::artifact::split_table_key(key);
        self.table(&scope, &kind)
    }

    /// 主表组里这一类（词/字）的表。
    ///
    /// 主表组缺这一类时（比如相加时只相加了词频表）回落到 `full` 的同类，
    /// 再回落到任意一张同类表 —— 宁可换个来源，也不能让整句变成「未收录」。
    pub fn primary_table(&self, kind: &str) -> Option<&TableRef> {
        self.table(&self.primary_scope, kind)
            .or_else(|| self.table(crate::artifact::SCOPE_FULL, kind))
            .or_else(|| self.tables.iter().find(|t| t.kind == kind))
    }

    /// 主表组里某一类的分组阈值。
    fn tiers_of_primary(&self, kind: &str) -> Vec<crate::rank::Tier> {
        let key = self
            .primary_table(kind)
            .map(|t| t.key())
            .unwrap_or_else(|| table_key(&self.primary_scope, kind));
        self.meta
            .table_by_key(&key)
            .map(|t| t.tiers.clone())
            .unwrap_or_else(|| {
                if kind == "char" {
                    crate::rank::default_char_tiers()
                } else {
                    crate::rank::default_word_tiers()
                }
            })
    }

    /// 分析一句话：分词 → 查频次 → 定分组 → 附各表排名。
    ///
    /// 单字 token 查主表组的字表，其余查主表组的词频表；都查不到则 `tier = None`，
    /// 界面应显示为「语料库未收录」，与「极少」区分开。
    pub fn analyze(&self, tk: &Tokenizer, text: &str) -> Vec<TokenInfo> {
        let segs = tk.segment(text);
        let word_tiers = self.tiers_of_primary("word");
        let char_tiers = self.tiers_of_primary("char");
        // 主表组的实际表（可能回落到 full）；阈值必须按**实际查到的那张表**取，
        // 否则"分组"是用 A 表的阈值去切 B 表的排名。
        let word_ref = self.primary_table("word");
        let char_ref = self.primary_table("char");
        let compare = segs.len() <= MAX_COMPARE_TOKENS;
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
                top_pct: None,
                entries: None,
                tier: None,
                tier_name: None,
                in_dict: None,
                from_user: None,
                table_ranks: Vec::new(),
            };

            if s.accepted {
                let kind = if s.single_cjk { "char" } else { "word" };
                // 单字查字表，其余查词频表。字表不写 flags（记录里没有 flags 字节），
                // 所以「是否在 jieba 词典内」必须直接问分词器，不能从字表记录里读，
                // 否则每个单字都会被误标成「词典外」。
                let (hit, tiers, tref) = if s.single_cjk {
                    (
                        char_ref.and_then(|t| t.lookup(&s.text)),
                        &char_tiers,
                        char_ref,
                    )
                } else {
                    (
                        word_ref.and_then(|t| t.lookup(&s.text)),
                        &word_tiers,
                        word_ref,
                    )
                };
                info.table = kind.to_string();

                if let Some(h) = &hit {
                    info.count = Some(h.count);
                    info.rank = Some(h.rank);
                    if let Some(t) = tref {
                        let total = t.total_tokens;
                        info.pct = Some(if total > 0 {
                            h.count as f64 * 100.0 / total as f64
                        } else {
                            0.0
                        });
                        info.entries = Some(t.entries);
                        info.top_pct = Some(crate::rank::pct_for_rank(h.rank, t.entries));
                    }
                    let ti = tier_of(h.rank, tiers);
                    info.tier = Some(ti);
                    info.tier_name = Some(tiers[ti].name.clone());
                }

                if kind == "word" {
                    if let Some(h) = &hit {
                        info.in_dict = Some(h.flags & crate::tokenize::FLAG_IN_DICT != 0);
                        info.from_user = Some(h.flags & crate::tokenize::FLAG_FROM_USER != 0);
                    }
                } else {
                    info.in_dict = Some(tk.has_word(&s.text));
                    info.from_user = Some(tk.is_user_word(&s.text));
                }

                if compare {
                    for t in self.tables.iter().filter(|t| t.kind == kind) {
                        let r = t.lookup(&s.text);
                        info.table_ranks.push(TableRank {
                            scope: t.scope.clone(),
                            kind: t.kind.clone(),
                            rank: r.as_ref().map(|h| h.rank),
                            top_pct: r
                                .as_ref()
                                .map(|h| crate::rank::pct_for_rank(h.rank, t.entries)),
                            entries: t.entries,
                            count: r.map(|h| h.count),
                        });
                    }
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
    use crate::rank::{RankedEntry, rank_entries};
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
            "的",
            "是",
            "在",
            "中国",
            "人工智能",
            "中华人民共和国",
            "元宇宙",
            "zzz",
            "abc",
            "龘",
            "㐀",
            "𠀀",
            "a",
            "ab",
            "abcde",
            "abcdefghijklmnopqrstu",
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
            let hit = t
                .lookup(&e.word)
                .unwrap_or_else(|| panic!("查不到 {:?}", e.word));
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
            top_pct: Some(0.03),
            entries: Some(100_000),
            tier: Some(2),
            tier_name: Some("较多".into()),
            in_dict: Some(true),
            from_user: Some(false),
            table_ranks: vec![TableRank {
                scope: "news".into(),
                kind: "word".into(),
                rank: Some(7),
                top_pct: Some(0.01),
                entries: 70_000,
                count: Some(9),
            }],
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
            "top_pct",
            "entries",
            "tier",
            "tier_name",
            "in_dict",
            "from_user",
            "table_ranks",
        ] {
            assert!(v.get(k).is_some(), "TokenInfo 序列化缺少字段 {k}：{v}");
        }
        // `domain_ranks` 在 schema v3 被 `table_ranks` 取代；两者同时存在会让界面
        // 一半新一半旧，所以钉死旧字段已经消失。
        assert!(
            v.get("domain_ranks").is_none(),
            "旧字段 domain_ranks 不该再出现"
        );
        for k in ["scope", "kind", "rank", "top_pct", "entries", "count"] {
            assert!(
                v["table_ranks"][0].get(k).is_some(),
                "TableRank 缺少字段 {k}"
            );
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
            assert!(
                h.word.starts_with("ab"),
                "前缀搜索返回了不匹配的词 {:?}",
                h.word
            );
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

    // ------------------------------------------------------------ positions / clone

    #[test]
    fn positions_yields_every_entry_exactly_once_in_word_order() {
        let entries = sample_entries();
        let t = build("positions", &entries, true);
        let got: Vec<Hit> = t.positions().collect();
        assert_eq!(got.len(), entries.len(), "整表遍历必须一条不多一条不少");
        for w in got.windows(2) {
            assert!(
                w[0].word < w[1].word,
                "遍历顺序必须是词序：{} 之后是 {}",
                w[0].word,
                w[1].word
            );
        }
        // 每一条的 count/rank 都要与查表结果一致
        for h in &got {
            let l = t.lookup(&h.word).expect("遍历出来的词必须查得到");
            assert_eq!(l.count, h.count);
            assert_eq!(l.rank, h.rank);
        }
        // 空表要能正常收尾（块数为 0）
        let p = tmpfile("positions_empty");
        crate::artifact::write_vfr(&p, KIND_WORD, &[], 0, true).unwrap();
        let e = VfrTable::open(&p).unwrap();
        assert_eq!(e.positions().count(), 0);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn positions_covers_block_boundaries() {
        // 造出比一个块（64）多得多的词，跨块收尾最容易写错
        let mut m: FxHashMap<Box<str>, u64> = FxHashMap::default();
        for i in 0..500u32 {
            m.insert(format!("w{i:04}").into(), (i + 1) as u64);
        }
        let entries = rank_entries(&m, &|_| 0);
        let t = build("positions_blocks", &entries, true);
        let got: Vec<Hit> = t.positions().collect();
        assert_eq!(got.len(), 500);
        assert_eq!(
            got.iter().map(|h| h.count).sum::<u64>(),
            (1..=500u64).sum::<u64>()
        );
    }

    #[test]
    fn clone_into_copies_the_table_verbatim() {
        let entries = sample_entries();
        let t = build("clone_src", &entries, true);
        let dst = tmpfile("clone_dst");
        let bytes = t.clone_into(&dst).unwrap();
        let c = VfrTable::open(&dst).unwrap();
        assert_eq!(c.len(), t.len());
        assert_eq!(c.header().total_tokens, t.header().total_tokens);
        assert_eq!(bytes, std::fs::metadata(&dst).unwrap().len());
        for e in &entries {
            assert_eq!(c.lookup(&e.word).unwrap().count, e.count);
        }
        std::fs::remove_file(&dst).ok();
    }

    #[test]
    fn top_pct_uses_entry_count_not_token_count() {
        let entries = sample_entries();
        let t = build("top_pct", &entries, true);
        // 第一名：1 / 条目数
        let p = t.top_pct(1);
        assert!((p - 100.0 / t.len() as f64).abs() < 1e-12);
        assert!(p > 0.0, "第 1 名的前% 不能是 0");
        assert!(
            (t.top_pct(t.len() as u32) - 100.0).abs() < 1e-9,
            "最后一名的前% 是 100"
        );
    }

    // ------------------------------------------------------------ Dataset 发现

    /// 造一个最小可用的新布局产物目录：`<root>/<scope>/{word,char}.vfr` + meta.json。
    fn make_dataset(root: &Path, scopes: &[&str], schema: u32) -> Meta {
        use crate::artifact::TableMeta;
        use crate::rank::{default_tier_pct_for, tiers_from_pct};
        let mut tables = Vec::new();
        for s in scopes {
            let w = sample_entries();
            let total: u64 = w.iter().map(|e| e.count).sum();
            std::fs::create_dir_all(root.join(s)).unwrap();
            crate::artifact::write_vfr(&root.join(s).join("word.vfr"), KIND_WORD, &w, total, true)
                .unwrap();
            let chars: Vec<(char, u64)> = (0..40u32)
                .map(|i| (char::from_u32(0x4E00 + i).unwrap(), 40 - i as u64))
                .collect();
            let c = crate::rank::rank_chars(chars);
            let ctotal: u64 = c.iter().map(|e| e.count).sum();
            crate::artifact::write_vfr(
                &root.join(s).join("char.vfr"),
                KIND_CHAR,
                &c,
                ctotal,
                false,
            )
            .unwrap();
            for (kind, entries, total) in [
                ("word", w.len() as u64, total),
                ("char", c.len() as u64, ctotal),
            ] {
                let pct = default_tier_pct_for(kind);
                tables.push(TableMeta {
                    path: (*s).to_string(),
                    kind: kind.into(),
                    entries,
                    total_tokens: total,
                    vfr_bytes: 1,
                    tiers: tiers_from_pct(pct, entries),
                    tier_stats: Vec::new(),
                    min_count: 1,
                    tier_pct: pct.to_vec(),
                    source_tables: Vec::new(),
                });
            }
        }
        let meta = Meta {
            schema_version: schema,
            generated_at: String::new(),
            tool_version: String::new(),
            corpus_root: String::new(),
            elapsed_ms: 0,
            tokenizer: serde_json::from_value(serde_json::json!({
                "engine": "jieba-rs", "version": "0.11", "hmm": false, "dicts": [],
                "min_len": 1, "max_len": 64,
                "keep_latin": true, "keep_digit": false, "skip_single_char": false,
            }))
            .unwrap(),
            totals: crate::Totals::default(),
            domains: scopes
                .iter()
                .map(|s| crate::artifact::SourceScope {
                    name: (*s).to_string(),
                    files: 1,
                    bytes: 1,
                })
                .collect(),
            tables,
            tier_names: crate::rank::TIER_NAMES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            tier_keys: crate::rank::TIER_KEYS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        crate::artifact::write_meta(&root.join("meta.json"), &meta).unwrap();
        meta
    }

    #[test]
    fn dataset_discovers_every_scope_and_picks_full_as_primary() {
        let root = tmpdir("ds_scopes");
        make_dataset(
            &root,
            &["wiki", "full", "news"],
            crate::artifact::SCHEMA_VERSION,
        );
        let ds = Dataset::open(&root).unwrap();

        assert_eq!(
            ds.scopes(),
            vec!["full", "news", "wiki"],
            "full 必须排在最前"
        );
        assert_eq!(ds.primary_scope, "full");
        assert_eq!(ds.tables.len(), 6, "3 个表组 × 2 类");
        // 表身份解析：唯一入口
        assert!(ds.table_by_key("news/word").is_some());
        assert!(ds.table_by_key("news/char").is_some());
        assert!(ds.table_by_key("nope/word").is_none());
        assert!(ds.table_by_key("news/nope").is_none());
        // 主表回落链
        assert_eq!(ds.primary_table("word").unwrap().scope, "full");
        assert_eq!(ds.primary_table("char").unwrap().scope, "full");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dataset_primary_falls_back_when_full_is_missing() {
        let root = tmpdir("ds_nofull");
        make_dataset(&root, &["wiki", "news"], crate::artifact::SCHEMA_VERSION);
        let ds = Dataset::open(&root).unwrap();
        assert_eq!(
            ds.primary_scope, "news",
            "没有 full 时应取排序后的第一个表组"
        );
        // 显式指定一个不存在的表组 → 也要回落，而不是留一个查不到表的主表组
        let ds2 = Dataset::open_with_primary(&root, Some("不存在的表组")).unwrap();
        assert_eq!(ds2.primary_scope, "news");
        // 指定存在的
        let ds3 = Dataset::open_with_primary(&root, Some("wiki")).unwrap();
        assert_eq!(ds3.primary_scope, "wiki");
        assert_eq!(ds3.primary_table("word").unwrap().scope, "wiki");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dataset_primary_falls_back_per_kind_when_scope_lacks_that_kind() {
        // 只相加了词频表的表组：它的字表不存在，字表要回落到 full，
        // 否则单字全变"未收录"，整句话都是灰的。
        let root = tmpdir("ds_kindfall");
        let mut meta = make_dataset(&root, &["full", "combo"], crate::artifact::SCHEMA_VERSION);
        meta.tables
            .retain(|t| !(t.path == "combo" && t.kind == "char"));
        crate::artifact::write_meta(&root.join("meta.json"), &meta).unwrap();
        let _ = std::fs::remove_file(root.join("combo").join("char.vfr"));

        let ds = Dataset::open_with_primary(&root, Some("combo")).unwrap();
        assert_eq!(ds.primary_scope, "combo");
        assert_eq!(ds.primary_table("word").unwrap().scope, "combo");
        assert_eq!(
            ds.primary_table("char").unwrap().scope,
            "full",
            "字表应回落到 full"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dataset_rejects_legacy_layout_with_an_actionable_message() {
        let root = tmpdir("ds_legacy");
        // schema v2：老布局（full/ 与 domains/ 两个目录树）
        make_dataset(&root, &["full"], 2);
        let err = Dataset::open(&root).expect_err("老布局必须被明确拒绝");
        let msg = err.to_string();
        assert!(msg.contains("v2"), "错误里要写清是哪一版：{msg}");
        assert!(msg.contains("重新统计"), "错误里要给出可执行的动作：{msg}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dataset_reports_a_missing_table_file_by_name() {
        let root = tmpdir("ds_missing");
        make_dataset(&root, &["full"], crate::artifact::SCHEMA_VERSION);
        std::fs::remove_file(root.join("full").join("char.vfr")).unwrap();
        let err = Dataset::open(&root).expect_err("缺文件必须报错");
        let msg = err.to_string();
        assert!(msg.contains("char"), "要点名缺的是哪一张表：{msg}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dataset_rejects_scope_names_containing_separators() {
        let root = tmpdir("ds_badscope");
        let mut meta = make_dataset(&root, &["full"], crate::artifact::SCHEMA_VERSION);
        meta.tables[0].path = "full/word".into(); // 老写法的残留
        crate::artifact::write_meta(&root.join("meta.json"), &meta).unwrap();
        let err = Dataset::open(&root).expect_err("表组名里有分隔符必须报错");
        assert!(err.to_string().contains("表组名非法"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dataset_analyze_uses_primary_scope_and_fills_top_pct() {
        let root = tmpdir("ds_analyze");
        make_dataset(&root, &["full", "news"], crate::artifact::SCHEMA_VERSION);
        let ds = Dataset::open_with_primary(&root, Some("news")).unwrap();
        // 分词器至少要有一份词典（`from_dicts` 对空链直接报错），所以落一份最小词典
        let dict_path = root.join("test.dict");
        std::fs::write(&dict_path, "中国 100 n\n人工智能 100 n\n").unwrap();
        let tk = crate::tokenize::Tokenizer::from_dict(
            &dict_path,
            crate::tokenize::TokenizeOpts::default(),
        )
        .expect("落一份词典就该能建分词器");
        let infos = ds.analyze(&tk, "中国人工智能");
        assert!(!infos.is_empty());
        assert!(
            infos.iter().any(|i| i.rank.is_some()),
            "语料里应当能查到这个句子里的词"
        );
        for i in &infos {
            if let Some(rank) = i.rank {
                let entries = i.entries.expect("有排名就必须有分母");
                let top = i.top_pct.expect("有排名就必须有前%");
                assert!((top - rank as f64 * 100.0 / entries as f64).abs() < 1e-12);
                assert!(top > 0.0, "前% 不能是 0（第 1 名也不行）");
            }
            // 对比列必须覆盖全部表组（同 kind），主表也在里面
            for r in &i.table_ranks {
                assert_eq!(r.kind, i.table);
                assert_eq!(
                    r.top_pct.is_some(),
                    r.rank.is_some(),
                    "有排名才有前%，两者必须同进同出"
                );
                assert!(r.entries > 0);
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    fn tmpdir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("vocfreq_ds_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
