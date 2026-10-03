//! 排行与七组分带。
//!
//! 分带口径默认是**前%**（`排名 ÷ 该表条目数 × 100`），见 `docs/DESIGN.md` §5.3：
//! 前%只跟位次有关，换一张规模差很多的表（表组几万条、全量表几百万条）也不会失真。
//! 绝对排名的两套默认阈值仍然保留（[`default_word_tiers`] / [`default_char_tiers`]），
//! 供设置里切回 `tierMethod = rank` 时用。
//!
//! ⚠ 词频表与字表的默认阈值**不同**：字表只有约一万个不重复汉字，套用词频表阈值会让所有字
//! 都落进「极多~很少」，色阶完全失去区分度。前%与绝对排名两套口径都遵守这条。

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::tokenize::{FLAG_FROM_USER, FLAG_IN_DICT};

pub const TIER_NAMES: [&str; 7] = ["极多", "很多", "较多", "中等", "较少", "很少", "极少"];

/// 七组的**稳定标识**，与 [`TIER_NAMES`] 同序、与语言无关。
///
/// 界面用它把组号映射到配色（`tier-colors.ts` 的 `TIER_PALETTES`）：产物调整分组顺序
/// 也不会串色，遇到不认识的 key 会退化成中性灰。**不要用中文组名当身份** ——
/// 组名是文案，将来接多语言就会整片串色。
///
/// 前端的等价常量是 `tier-colors.ts::TIER_KEYS`，两者必须逐字一致（有测试比对）。
pub const TIER_KEYS: [&str; 7] = [
    "very_common",
    "common",
    "fairly_common",
    "medium",
    "fairly_rare",
    "rare",
    "very_rare",
];

/// 词频表默认的**前%上界**（0..100，6 个数，第 7 组是"以上全部"）。
///
/// 不是新拍的：它们是把旧的**绝对排名**默认值（`100 / 1k / 5k / 20k / 50k / 150k`）
/// 放在实测的 380 万条全库词频表上换算出来的，所以换口径之后色阶观感与从前一致
/// （推导与双向验证见 `docs/DESIGN.md` §5.3）。
pub const DEFAULT_TIER_PCT: [f64; 6] = [0.0026, 0.026, 0.132, 0.526, 1.32, 3.95];

/// 字表默认的前%上界。
///
/// 由旧的 `≤50 / ≤200 / ≤600 / ≤1500 / ≤3000 / ≤5000` 在约 1.9 万字的表上换算而来。
/// 直接套词频表那套会让头几档只剩个位数的字、九成以上的字全挤进「极少」。
pub const DEFAULT_CHAR_TIER_PCT: [f64; 6] = [0.26, 1.05, 3.16, 7.89, 15.8, 26.3];

/// 词频表的默认前%口径。
pub fn default_tier_pct() -> &'static [f64; 6] {
    &DEFAULT_TIER_PCT
}

/// 按表的类型取默认前%口径（`char` 用字表那套，其余一律词频表那套）。
pub fn default_tier_pct_for(kind: &str) -> &'static [f64; 6] {
    if kind == "char" {
        &DEFAULT_CHAR_TIER_PCT
    } else {
        &DEFAULT_TIER_PCT
    }
}

/// 排名 → 前%（0..100）。`entries` 是**条目数**，不是 token 数。
///
/// 与前端 `format.ts::pctForRank` 等价。空表返回 0（不是 1）：调用方要能一眼看出
/// "这张表没有条目，别拿百分比切它"。
pub fn pct_for_rank(rank: u32, entries: u64) -> f64 {
    if entries == 0 {
        return 0.0;
    }
    rank as f64 * 100.0 / entries as f64
}

/// 前%上界 → 排名上界（含），与前端 `format.ts::rankForPct` 等价。
///
/// 取整方向是 **ceil**：判前%时要拿边界那一条自己的前%去比，`ceil` 才不会把边界条目
/// 推到下一档；`floor` 还会在条目少时把几档压成同一个排名，七组里出现空档、组号到
/// 配色的映射整体错位。空表返回 1，非正数返回 0（"这一档收不到任何条目"）。
pub fn rank_for_pct(pct: f64, entries: u64) -> u32 {
    if entries == 0 || !pct.is_finite() || pct <= 0.0 {
        return 0;
    }
    let r = (pct / 100.0 * entries as f64).ceil();
    if !r.is_finite() || r < 1.0 {
        return 1;
    }
    if r >= u32::MAX as f64 {
        return u32::MAX;
    }
    r as u32
}

/// 把 6 个前%上界换算成**严格递增**的 6 个排名上界，与前端 `format.ts::ranksFromPct` 等价。
///
/// 必须夹成严格递增：用户可能填一组没拉开的前%，或条目数太少（比如某张表只有 10 条），
/// 那样相邻两档会算出同一个排名，后面几组就永远是空的。空表返回 6 个 1 ——
/// **不能返回空数组**：缺了前 6 个上界，七组就只剩"以上全部"一组，组号到配色的映射
/// 会整体错位。
pub fn ranks_from_pct(pcts: &[f64], entries: u64) -> Vec<u32> {
    if entries == 0 {
        return pcts.iter().map(|_| 1).collect();
    }
    let mut out: Vec<u32> = Vec::with_capacity(pcts.len());
    for p in pcts {
        let mut r = rank_for_pct(*p, entries);
        if let Some(prev) = out.last() {
            if r <= *prev {
                r = prev.saturating_add(1);
            }
        }
        out.push(r);
    }
    out
}

/// 按前%口径切出七档。`pct` 是 6 个前%上界。
pub fn tiers_from_pct(pct: &[f64], entries: u64) -> Vec<Tier> {
    let bounds = ranks_from_pct(pct, entries);
    let mut v: Vec<Tier> = TIER_NAMES
        .iter()
        .zip(bounds.iter())
        .map(|(n, b)| Tier {
            name: (*n).to_string(),
            max_rank: *b as u64,
        })
        .collect();
    v.push(Tier {
        name: TIER_NAMES[6].to_string(),
        max_rank: u64::MAX,
    });
    v
}

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
        .map(|(n, b)| Tier {
            name: (*n).to_string(),
            max_rank: *b,
        })
        .collect();
    v.push(Tier {
        name: TIER_NAMES[6].to_string(),
        max_rank: u64::MAX,
    });
    v
}

/// 词频表默认阈值。
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
/// 字表只有约 1.9 万个不重复汉字（词频表有 380 万），套用词频表阈值会让所有字都落进
/// 「极多~很少」而丢掉区分度。实测前 50 个汉字覆盖约 29% 的汉字出现次数，
/// 与词频表「极多 ≤100」的覆盖率量级相当。
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
    v.sort_unstable_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });
    v.into_iter()
        .enumerate()
        .map(|(i, (word, count))| {
            let flags = flag_fn(&word);
            RankedEntry {
                word,
                count,
                rank: (i + 1) as u32,
                flags,
            }
        })
        .collect()
}

/// 同 [`rank_entries`]，但**消费**计数表、直接搬走已有的 `Box<str>`。
///
/// 全库词频表在千万级时，逐个 `clone()` 意味着上千万次堆分配；实测这是排行榜阶段
/// 最大的一笔开销。计数表在排名后不再需要，因此能搬就不要克隆。
pub fn rank_entries_owned(
    map: FxHashMap<Box<str>, u64>,
    flag_fn: &dyn Fn(&str) -> u8,
) -> Vec<RankedEntry> {
    let mut v: Vec<(Box<str>, u64)> = map.into_iter().collect();
    v.sort_unstable_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });
    v.into_iter()
        .enumerate()
        .map(|(i, (word, count))| {
            let flags = flag_fn(&word);
            RankedEntry {
                word,
                count,
                rank: (i + 1) as u32,
                flags,
            }
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
        let chars: Vec<(char, u64)> = (0..10_000u32)
            .map(|i| (char::from_u32(0x4E00 + i).unwrap(), 10_000 - i as u64))
            .collect();
        let entries = rank_chars(chars);
        let tiers = default_char_tiers();
        let stats = tier_stats(&entries, &tiers);
        let nonempty = stats.iter().filter(|s| s.entries > 0).count();
        assert_eq!(nonempty, 7, "七组都应非空: {stats:?}");
        // 反之，若误用词频表阈值，最后一组会是空的
        let word_stats = tier_stats(&entries, &default_word_tiers());
        assert_eq!(
            word_stats[6].entries, 0,
            "词频表阈值套在字表上会让「极少」为空"
        );
    }

    #[test]
    fn tier_stats_coverage_sums_to_one() {
        let m = map(&[("a", 60), ("b", 30), ("c", 10)]);
        let entries = rank_entries(&m, &|_| 0);
        let stats = tier_stats(&entries, &default_word_tiers());
        let sum: f64 = stats.iter().map(|s| s.coverage).sum();
        assert!((sum - 1.0).abs() < 1e-9, "覆盖率之和应为 1，实际 {sum}");
        // 三个词（60/30/10）都排在前 3，必定全落在第一档里
        assert_eq!(stats[0].entries, 3, "三个词应全在「极多」");
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

    // ---------------------------------------------------------------- 前%口径

    #[test]
    fn pct_and_rank_round_trip_on_the_ceil_side() {
        // 第 1 名的前%必须 > 0（不能因为"太小"被压成 0），这是界面显示与分档的前提
        assert!(pct_for_rank(1, 3_800_000) > 0.0);
        assert_eq!(pct_for_rank(1, 3_800_000), 100.0 / 3_800_000.0);
        assert_eq!(pct_for_rank(50, 1_000), 5.0);
        // `rank_for_pct` 是 `pct_for_rank` 的**上取整**逆：换回来不会比原排名小，
        // 最多少 1（相邻名次的前%差小于一个刻度时必然是同一个上界）
        for rank in [1u32, 7, 99, 5_000, 150_000, 3_799_999] {
            let pct = pct_for_rank(rank, 3_800_000);
            let back = rank_for_pct(pct, 3_800_000);
            assert!(
                back >= rank && back as i64 - rank as i64 <= 1,
                "rank {rank} -> {pct}% -> {back} 不该偏出一个名次"
            );
        }
    }

    #[test]
    fn rank_for_pct_handles_the_empty_and_degenerate_cases() {
        // 空表：不能返回 0（那会让七组只剩"以上全部"一组），也不能 panic
        assert_eq!(rank_for_pct(1.0, 0), 0);
        assert_eq!(ranks_from_pct(&DEFAULT_TIER_PCT, 0), vec![1; 6]);
        assert_eq!(tiers_from_pct(&DEFAULT_TIER_PCT, 0).len(), 7);
        // 非正数 = 这一档收不到条目
        assert_eq!(rank_for_pct(0.0, 1_000), 0);
        assert_eq!(rank_for_pct(-1.0, 1_000), 0);
        assert_eq!(rank_for_pct(f64::NAN, 1_000), 0);
        // 极小的一档在少条目表上会被抬到 1（否则它永远是空组）
        assert_eq!(rank_for_pct(0.001, 10), 1);
    }

    #[test]
    fn ranks_from_pct_is_strictly_increasing_even_when_the_input_is_not() {
        // 用户可能填一组没拉开的前%，或表太小 —— 夹成严格递增，否则后面几组永远为空
        let got = ranks_from_pct(&[0.0026, 0.0026, 0.0027, 0.0028, 0.0029, 0.003], 10);
        assert_eq!(got, vec![1, 2, 3, 4, 5, 6]);
        for w in got.windows(2) {
            assert!(w[0] < w[1], "必须严格递增：{got:?}");
        }
        // 空输入 → 空输出（调用方自己兜底），不能 panic
        assert!(ranks_from_pct(&[], 1_000).is_empty());
    }

    #[test]
    fn default_pcts_reproduce_the_old_absolute_thresholds() {
        // 默认前%的**来历**：旧的绝对阈值在实测的 380 万条全库词频表上换算得来。
        // 换算回去必须落在旧阈值附近，否则说明口径被改动过。见 docs/DESIGN.md §5.3。
        //
        // 容差 5% 而不是更紧：表规模与"约 1.9 万字"都是**约数**，换算本身带取整误差。
        // 这条测试要抓的是"公式写错/取整方向反了"（那会差一个数量级），不是小数末位。
        let entries = 3_800_000u64;
        let old: [u32; 6] = [100, 1_000, 5_000, 20_000, 50_000, 150_000];
        let got = ranks_from_pct(&DEFAULT_TIER_PCT, entries);
        for (i, (g, o)) in got.iter().zip(old.iter()).enumerate() {
            let diff = (*g as f64 - *o as f64).abs() / *o as f64;
            assert!(
                diff < 0.05,
                "第 {} 档换算回 {} 与旧阈值 {o} 差得太多（{:.1}%）",
                i + 1,
                g,
                diff * 100.0
            );
        }
        // 字表那套同样由旧阈值换算而来
        let char_entries = 19_000u64;
        let old_char: [u32; 6] = [50, 200, 600, 1_500, 3_000, 5_000];
        let got = ranks_from_pct(&DEFAULT_CHAR_TIER_PCT, char_entries);
        for (i, (g, o)) in got.iter().zip(old_char.iter()).enumerate() {
            let diff = (*g as f64 - *o as f64).abs() / *o as f64;
            assert!(
                diff < 0.05,
                "字表第 {} 档换算回 {} 与旧阈值 {o} 差得太多",
                i + 1,
                g
            );
        }
    }

    #[test]
    fn tiers_from_pct_keeps_seven_strictly_increasing_groups() {
        // 这两条正是 compose 与界面依赖的性质：7 档、严格递增、最后一档是"以上全部"
        for entries in [1u64, 7, 3_800_000] {
            let t = tiers_from_pct(&DEFAULT_TIER_PCT, entries);
            assert_eq!(t.len(), 7, "entries={entries}");
            assert_eq!(t[6].max_rank, u64::MAX);
            assert_eq!(t[0].name, TIER_NAMES[0]);
            assert_eq!(t[6].name, TIER_NAMES[6]);
            assert!(
                t[..6].windows(2).all(|w| w[0].max_rank < w[1].max_rank),
                "阈值必须严格递增（entries={entries}）：{t:?}"
            );
        }

        // 空表是个**明确的例外**：前 6 档只能全给 1（没有条目可分），第 7 档照旧是
        // "以上全部"。这里绝不能返回空数组 —— 缺了前 6 个上界，七组就只剩一组，
        // 组号到配色的映射会整体错位。见 `ranks_from_pct` 的注释。
        let t = tiers_from_pct(&DEFAULT_TIER_PCT, 0);
        assert_eq!(t.len(), 7);
        assert!(
            t[..6].iter().all(|x| x.max_rank == 1),
            "空表前 6 档都是 1：{t:?}"
        );
        assert_eq!(t[6].max_rank, u64::MAX);
        // 任何排名都要落进合法的组里（不 panic、不越界）
        assert_eq!(tier_of(1, &t), 0);
        assert_eq!(tier_of(u32::MAX, &t), 6);
    }

    #[test]
    fn default_pct_for_picks_the_char_table_by_kind() {
        assert_eq!(default_tier_pct_for("char"), &DEFAULT_CHAR_TIER_PCT);
        assert_eq!(default_tier_pct_for("word"), &DEFAULT_TIER_PCT);
        // 不认识的类型按词频表处理（宁可给一套通用口径，也不要空数组）
        assert_eq!(default_tier_pct_for("something"), &DEFAULT_TIER_PCT);
        // 前%只有 6 档上界，第 7 组是"以上全部"
        assert_eq!(default_tier_pct().len(), TIER_NAMES.len() - 1);
        assert_eq!(DEFAULT_CHAR_TIER_PCT.len(), TIER_NAMES.len() - 1);
        // 字表那套必须比词频表**宽松**：字表条目少得多，套词频表那套头几档会只剩个位数
        for (w, c) in DEFAULT_TIER_PCT.iter().zip(DEFAULT_CHAR_TIER_PCT.iter()) {
            assert!(c > w, "字表前%上界 {c} 应大于词频表的 {w}");
        }
    }

    #[test]
    fn tier_keys_are_unique_and_match_the_frontend_contract() {
        // 前端 `tier-colors.ts::TIER_KEYS` 是同一份契约；这里钉住"不多不少、不重复、同序"
        assert_eq!(TIER_KEYS.len(), TIER_NAMES.len());
        let mut sorted: Vec<&str> = TIER_KEYS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), TIER_KEYS.len(), "稳定标识不能重复");
        assert_eq!(TIER_KEYS[0], "very_common");
        assert_eq!(TIER_KEYS[6], "very_rare");
        // 词频表的口径也应当能被前%算回来（默认值就是它）
        assert_eq!(default_tier_pct(), &DEFAULT_TIER_PCT);
    }
}
