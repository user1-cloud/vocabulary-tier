//! 排行与七组分带。
//!
//! 分带口径为**排名绝对值**（用户选定），且**词表与字表阈值不同**：
//! 字表只有约一万个不重复汉字，若套用词表阈值会导致所有字都落进「极多~很少」，
//! 色阶完全失去区分度。

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::tokenize::{FLAG_FROM_USER, FLAG_IN_DICT};

pub const TIER_NAMES: [&str; 7] = ["极多", "很多", "较多", "中等", "较少", "很少", "极少"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tier {
    pub name: String,
    /// 该组的排名上界（含）。最后一组用 `u64::MAX`。
    pub max_rank: u64,
}

fn tiers(bounds: [u64; 6]) -> Vec<Tier> {
    let mut v: Vec<Tier> = TIER_NAMES
        .iter()
        .zip(bounds.iter())
        .map(|(n, b)| Tier { name: (*n).to_string(), max_rank: *b })
        .collect();
    v.push(Tier { name: TIER_NAMES[6].to_string(), max_rank: u64::MAX });
    v
}

/// 词表默认阈值。
///
/// 这是按**实测累计覆盖率**校准过的（全库 26.43 亿 token，见 `docs/DESIGN.md` §5.3）：
///
/// | 组 | 排名上界 | 累计覆盖 |
/// |---|---|---|
/// | 极多 | ≤ 100 | 26.7% |
/// | 很多 | ≤ 1,000 | 55.6% |
/// | 较多 | ≤ 5,000 | 78.3% |
/// | 中等 | ≤ 20,000 | 90.8% |
/// | 较少 | ≤ 50,000 | 95.8% |
/// | 很少 | ≤ 150,000 | ~99% |
/// | 极少 | > 150,000 | 100% |
///
/// 「极多」刻意收得很紧：只有 100 个词。早期版本用 500，那 500 个词就盖住了 45%
/// 的正文，结果「极多」和「很多」看起来差不多，色阶失去区分度。
pub fn default_word_tiers() -> Vec<Tier> {
    tiers([100, 1_000, 5_000, 20_000, 50_000, 150_000])
}

/// 字表默认阈值。
///
/// 字表只有约 1.9 万个不重复汉字（词表有 380 万），套用词表阈值会让所有字都落进
/// 「极多~很少」而丢掉区分度。实测前 50 个汉字覆盖约 29% 的汉字出现次数，
/// 与词表「极多 ≤100」的覆盖率量级相当。
pub fn default_char_tiers() -> Vec<Tier> {
    tiers([50, 200, 600, 1_500, 3_000, 5_000])
}

/// 某排名落在第几组（0..7）。
pub fn tier_of(rank: u32, tiers: &[Tier]) -> usize {
    let r = rank as u64;
    for (i, t) in tiers.iter().enumerate() {
        if r <= t.max_rank {
            return i;
        }
    }
    tiers.len() - 1
}

/// 排行中的一个词条。
#[derive(Debug, Clone, Serialize)]
pub struct RankedEntry {
    pub word: Box<str>,
    pub count: u64,
    /// 从 1 开始
    pub rank: u32,
    pub flags: u8,
}

impl RankedEntry {
    pub fn in_dict(&self) -> bool {
        self.flags & FLAG_IN_DICT != 0
    }
    pub fn from_user(&self) -> bool {
        self.flags & FLAG_FROM_USER != 0
    }
}

/// 按频次降序排名。同频次时以词的字节序为次序，保证**结果可复现**。
///
/// `flag_fn` 为每个词计算标记位。标记是按**去重后的词**算的，不是按 token 算的，
/// 因此只在 `O(unique)` 次而非 `O(tokens)` 次上付出代价。
pub fn rank_entries(
    map: &FxHashMap<Box<str>, u64>,
    flag_fn: &dyn Fn(&str) -> u8,
) -> Vec<RankedEntry> {
    let mut v: Vec<(Box<str>, u64)> = map.iter().map(|(w, c)| (w.clone(), *c)).collect();
    v.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.as_bytes().cmp(b.0.as_bytes())));
    v.into_iter()
        .enumerate()
        .map(|(i, (word, count))| {
            let flags = flag_fn(&word);
            RankedEntry { word, count, rank: (i + 1) as u32, flags }
        })
        .collect()
}

/// 同 [`rank_entries`]，但**消费**计数表、直接搬走已有的 `Box<str>`。
///
/// 全库词表在千万级时，逐个 `clone()` 意味着上千万次堆分配；实测这是排行榜阶段
/// 最大的一笔开销。计数表在排名后不再需要，因此能搬就不要克隆。
pub fn rank_entries_owned(
    map: FxHashMap<Box<str>, u64>,
    flag_fn: &dyn Fn(&str) -> u8,
) -> Vec<RankedEntry> {
    let mut v: Vec<(Box<str>, u64)> = map.into_iter().collect();
    v.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.as_bytes().cmp(b.0.as_bytes())));
    v.into_iter()
        .enumerate()
        .map(|(i, (word, count))| {
            let flags = flag_fn(&word);
            RankedEntry { word, count, rank: (i + 1) as u32, flags }
        })
        .collect()
}

/// 用字表的 (字符, 频次) 构造排行（字符已按频次降序传入）。
pub fn rank_chars(chars: Vec<(char, u64)>) -> Vec<RankedEntry> {
    chars
        .into_iter()
        .enumerate()
        .map(|(i, (c, count))| RankedEntry {
            word: c.to_string().into_boxed_str(),
            count,
            rank: (i + 1) as u32,
            flags: 0,
        })
        .collect()
}

/// 每组的词条数与**累计 token 覆盖率**。
///
/// 覆盖率是校准阈值的关键依据：排名分带只看位次，看不出「这 500 个词实际盖住了
/// 全文多少 token」。跑完首轮后应把这张表展示给用户，并据此调整阈值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierStat {
    pub name: String,
    pub max_rank: u64,
    pub entries: u64,
    pub tokens: u64,
    /// 该组 token 占全部 token 的比例（0..1）
    pub coverage: f64,
    /// 截至该组的累计覆盖率
    pub cumulative: f64,
}

pub fn tier_stats(entries: &[RankedEntry], tiers: &[Tier]) -> Vec<TierStat> {
    let total: u64 = entries.iter().map(|e| e.count).sum();
    let total = total.max(1);
    let mut out: Vec<TierStat> = tiers
        .iter()
        .map(|t| TierStat {
            name: t.name.clone(),
            max_rank: t.max_rank,
            entries: 0,
            tokens: 0,
            coverage: 0.0,
            cumulative: 0.0,
        })
        .collect();
    for e in entries {
        let i = tier_of(e.rank, tiers);
        out[i].entries += 1;
        out[i].tokens += e.count;
    }
    let mut cum = 0u64;
    for s in out.iter_mut() {
        cum += s.tokens;
        s.coverage = s.tokens as f64 / total as f64;
        s.cumulative = cum as f64 / total as f64;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, u64)]) -> FxHashMap<Box<str>, u64> {
        pairs.iter().map(|(w, c)| ((*w).into(), *c)).collect()
    }

    #[test]
    fn ranks_by_count_desc_then_word() {
        let m = map(&[("b", 5), ("a", 5), ("c", 9)]);
        let r = rank_entries(&m, &|_| 0);
        assert_eq!(&*r[0].word, "c");
        assert_eq!(r[0].rank, 1);
        // 同频次按字节序，保证可复现
        assert_eq!(&*r[1].word, "a");
        assert_eq!(&*r[2].word, "b");
    }

    #[test]
    fn char_tiers_do_not_collapse() {
        // 一万个字的表，用字表阈值应分布在多组里，而不是全落在「极多」
        let chars: Vec<(char, u64)> =
            (0..10_000u32).map(|i| (char::from_u32(0x4E00 + i).unwrap(), 10_000 - i as u64)).collect();
        let entries = rank_chars(chars);
        let tiers = default_char_tiers();
        let stats = tier_stats(&entries, &tiers);
        let nonempty = stats.iter().filter(|s| s.entries > 0).count();
        assert_eq!(nonempty, 7, "七组都应非空: {stats:?}");
        // 反之，若误用词表阈值，最后一组会是空的
        let word_stats = tier_stats(&entries, &default_word_tiers());
        assert_eq!(word_stats[6].entries, 0, "词表阈值套在字表上会让「极少」为空");
    }

    #[test]
    fn tier_stats_coverage_sums_to_one() {
        let m = map(&[("a", 60), ("b", 30), ("c", 10)]);
        let entries = rank_entries(&m, &|_| 0);
        let stats = tier_stats(&entries, &default_word_tiers());
        let sum: f64 = stats.iter().map(|s| s.coverage).sum();
        assert!((sum - 1.0).abs() < 1e-9, "覆盖率之和应为 1，实际 {sum}");
        assert_eq!(stats[0].entries, 3, "三个词排名都 <= 500，全在「极多」");
        assert!((stats[0].coverage - 1.0).abs() < 1e-9);
    }

    #[test]
    fn tier_of_boundaries() {
        let t = default_word_tiers();
        assert_eq!(tier_of(1, &t), 0);
        assert_eq!(tier_of(100, &t), 0);
        assert_eq!(tier_of(101, &t), 1);
        assert_eq!(tier_of(1_000, &t), 1);
        assert_eq!(tier_of(1_001, &t), 2);
        assert_eq!(tier_of(150_001, &t), 6);
        assert_eq!(tier_of(u32::MAX, &t), 6);
    }

    #[test]
    fn word_tiers_are_strict_about_the_top_group() {
        // 「极多」必须只留给极少数词：500 个词就盖住 45% 正文的旧口径区分度不足
        let t = default_word_tiers();
        assert_eq!(t[0].max_rank, 100, "极多应只收前 100 名");
        // 阈值必须严格递增，否则某些组会永远为空
        for w in t.windows(2) {
            assert!(w[0].max_rank < w[1].max_rank, "阈值必须递增：{:?}", t);
        }
    }

    #[test]
    fn flags_surface_on_entries() {
        let m = map(&[("元宇宙", 7)]);
        let r = rank_entries(&m, &|_| FLAG_FROM_USER);
        assert!(r[0].from_user());
        assert!(!r[0].in_dict());
    }
}
