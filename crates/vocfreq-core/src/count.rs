//! 计数容器。
//!
//! 实测结论（`docs/DESIGN.md` §0）：互斥锁分片哈希表在 32 线程时会**倒退**
//! （501 → 434 MB/s），而无锁的「每线程独占 + 末尾归并」能到 848 MB/s 且单调扩展。
//! 因此这里只提供每线程独占的 [`LocalCounts`] 与归并用的 [`GlobalCounts`]。

use rustc_hash::{FxHashMap, FxHashSet};

/// Unicode 码点槽位数（含扩展 B 区的高位平面字符），单线程约 557 KB。
pub const CHAR_SLOTS: usize = 0x11_000;

/// 单个线程独占的计数表，无任何锁。
pub struct LocalCounts {
    pub words: FxHashMap<Box<str>, u64>,
    pub chars: Vec<u64>,
}

impl Default for LocalCounts {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalCounts {
    pub fn new() -> Self {
        LocalCounts {
            // 初始容量按 64MB 语料块的去重规模估（约 5~10 万词）。用较小的初值是因为
            // rayon 的 fold 会创建多个累加器，预留过大（如 1<<20）会白白吃掉几十 MB。
            words: FxHashMap::with_capacity_and_hasher(1 << 16, Default::default()),
            chars: vec![0u64; CHAR_SLOTS],
        }
    }

    #[inline]
    pub fn add_word(&mut self, w: &str) {
        match self.words.get_mut(w) {
            Some(v) => *v += 1,
            None => {
                self.words.insert(w.into(), 1);
            }
        }
    }

    #[inline]
    pub fn add_char(&mut self, c: char) {
        let cp = c as usize;
        if cp < CHAR_SLOTS {
            self.chars[cp] += 1;
        }
    }

    /// 把 `other` 并入 `self`。用于线程间的结果归并。
    pub fn merge(&mut self, other: &LocalCounts) {
        for (w, c) in &other.words {
            match self.words.get_mut(w.as_ref()) {
                Some(v) => *v += *c,
                None => {
                    self.words.insert(w.clone(), *c);
                }
            }
        }
        for (i, c) in other.chars.iter().enumerate() {
            if *c > 0 {
                self.chars[i] += *c;
            }
        }
    }

    pub fn word_total(&self) -> u64 {
        self.words.values().sum()
    }

    pub fn char_total(&self) -> u64 {
        self.chars.iter().sum()
    }

    /// 按次数降序取出全部汉字及其频次。
    pub fn char_entries(&self) -> Vec<(char, u64)> {
        let mut v: Vec<(char, u64)> = self
            .chars
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .filter_map(|(i, c)| Some((char::from_u32(i as u32)?, *c)))
            .collect();
        v.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }
}

/// 归并后的全局计数表（词 + 字）。
pub struct GlobalCounts {
    pub words: FxHashMap<Box<str>, u64>,
    pub chars: LocalCounts,
}

impl Default for GlobalCounts {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalCounts {
    pub fn new() -> Self {
        GlobalCounts {
            // 全库去重词数在千万级以下，1<<20 起步足够，避免一上来就预留上百 MB
            words: FxHashMap::with_capacity_and_hasher(1 << 20, Default::default()),
            chars: LocalCounts::new(),
        }
    }

    /// 把某个域（或某批）的结果并入全库表。
    pub fn merge(&mut self, other: &LocalCounts) {
        for (w, c) in &other.words {
            match self.words.get_mut(w.as_ref()) {
                Some(v) => *v += *c,
                None => {
                    self.words.insert(w.clone(), *c);
                }
            }
        }
        for (i, c) in other.chars.iter().enumerate() {
            if *c > 0 {
                self.chars.chars[i] += *c;
            }
        }
    }
}

/// 高频但不在 jieba 词典里的候选词，供 `--user-dict` 回填。
///
/// 关闭 HMM 时词典外词主要来自这些来源：专名、网络新词、以及被切碎的固定搭配
/// （实测「元宇宙」「区块链」「直播带货」「情绪价值」都不在词典里）。
pub fn oov_candidates(
    words: &FxHashMap<Box<str>, u64>,
    in_dict: &dyn Fn(&str) -> bool,
    min_count: u64,
    min_len: usize,
    limit: usize,
) -> Vec<(Box<str>, u64)> {
    let mut v: Vec<(Box<str>, u64)> = words
        .iter()
        .filter(|(w, c)| **c >= min_count && w.chars().count() >= min_len && !in_dict(w.as_ref()))
        .map(|(w, c)| (w.clone(), *c))
        .collect();
    v.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(limit);
    v
}

/// 供 `in_dict` 判定用的词集合。
pub type WordSet = FxHashSet<Box<str>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_sums_counts() {
        let mut a = LocalCounts::new();
        let mut b = LocalCounts::new();
        a.add_word("的");
        a.add_word("的");
        a.add_word("中");
        b.add_word("的");
        b.add_word("国");
        a.add_char('的');
        b.add_char('的');
        a.merge(&b);
        assert_eq!(a.words.get("的"), Some(&3));
        assert_eq!(a.words.get("中"), Some(&1));
        assert_eq!(a.words.get("国"), Some(&1));
        assert_eq!(a.chars['的' as usize], 2);
        assert_eq!(a.word_total(), 5);
        assert_eq!(a.char_total(), 2);
    }

    #[test]
    fn char_entries_sorted_desc() {
        let mut a = LocalCounts::new();
        for _ in 0..3 {
            a.add_char('中');
        }
        a.add_char('国');
        let e = a.char_entries();
        assert_eq!(e[0], ('中', 3));
        assert_eq!(e[1], ('国', 1));
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn oov_filters_by_dict_and_length() {
        let mut w: FxHashMap<Box<str>, u64> = FxHashMap::default();
        w.insert("元宇宙".into(), 100);
        w.insert("的".into(), 9_999_999);
        w.insert("稀".into(), 50);
        let dict: WordSet = ["的".into()].into_iter().collect();
        let got = oov_candidates(&w, &|s| dict.contains(s), 10, 2, 10);
        assert_eq!(got.len(), 1);
        assert_eq!(&*got[0].0, "元宇宙");
    }
}
