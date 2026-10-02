//! 分词与过滤。
//!
//! 关键决策（`docs/DESIGN.md` §3）：
//! * 默认 **关闭 HMM**：开启后同一实体在不同上下文可能被切成不同形态，导致频次被
//!   拆散、排行不可复现。实测关闭时 jieba 只输出「词典命中的词 + 未命中的单字」。
//! * 词典可配置：`--user-dict` 叠加到内置 349,046 条词典（jieba-rs `load_dict`
//!   的语义确实是叠加而非替换），`--dict` 则整个替换。
//! * 词典第三列是**分词用的概率权重**，不是词频（实测 45.6% 的条目权重都是保底值
//!   3），因此稀有度一律由语料统计得出，不使用它。

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use jieba_rs::Jieba;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use crate::clean::is_cjk_ideograph;
use crate::count::LocalCounts;
use crate::{Error, Result};

/// 词条标记位。
pub const FLAG_IN_DICT: u8 = 1 << 0;
/// 该词来自用户自定义词典（不在 jieba 内置词典里）。
pub const FLAG_FROM_USER: u8 = 1 << 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenizeOpts {
    /// 是否启用 HMM 新词发现（默认 false）
    pub hmm: bool,
    pub min_len: usize,
    /// 防御性上限，避免超长 token 进表
    pub max_len: usize,
    /// 是否保留纯英文/含英文的 token（如 `iPhone`、`SU7`）
    pub keep_latin: bool,
    /// 是否保留纯数字 token（如 `2023`）；默认丢弃以保持表干净
    pub keep_digit: bool,
    /// 是否丢弃单字词（单字另有字表承载）
    pub skip_single_char: bool,
}

impl Default for TokenizeOpts {
    fn default() -> Self {
        TokenizeOpts {
            hmm: false,
            min_len: 1,
            max_len: 64,
            keep_latin: true,
            keep_digit: false,
            skip_single_char: false,
        }
    }
}

/// 一个分词结果，带字节区间，供界面直接高亮而无需自己对齐下标。
#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub text: String,
    pub byte_start: usize,
    pub byte_end: usize,
    /// 是否为计入统计的内容 token（标点与空白为 false）
    pub accepted: bool,
    /// 是否为单个汉字（界面应改查字表）
    pub single_cjk: bool,
}

pub struct Tokenizer {
    jieba: Jieba,
    user_words: FxHashSet<Box<str>>,
    pub opts: TokenizeOpts,
    /// 词典来源描述，写进 meta.json
    pub dict_label: String,
}

/// `count_into` 的计数结果。
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counted {
    /// 计入词表的 token 数
    pub words: u64,
    /// 计入字表的汉字数
    pub chars: u64,
}

impl Tokenizer {
    /// 使用 jieba 内置词典。
    pub fn builtin(opts: TokenizeOpts) -> Self {
        Tokenizer {
            jieba: Jieba::new(),
            user_words: FxHashSet::default(),
            opts,
            dict_label: "builtin(jieba dict.txt, 349046 entries)".into(),
        }
    }

    /// 完全替换为指定词典文件（`词 词频 词性`，词频与词性可省略）。
    pub fn with_dict_file(path: &Path, opts: TokenizeOpts) -> Result<Self> {
        let mut jieba = Jieba::empty();
        let mut f = BufReader::new(File::open(path)?);
        jieba
            .load_dict(&mut f)
            .map_err(|e| Error::Schema(format!("加载词典 {} 失败: {e}", path.display())))?;
        let n = read_dict_words(path)?.len();
        Ok(Tokenizer {
            jieba,
            user_words: FxHashSet::default(),
            opts,
            dict_label: format!("{} ({n} entries)", path.display()),
        })
    }

    /// 在现有词典基础上**叠加**用户词典，并记录来源以便标记 `from_user`。
    pub fn load_user_dict(&mut self, path: &Path) -> Result<usize> {
        let words = read_dict_words(path)?;
        let mut f = BufReader::new(File::open(path)?);
        self.jieba
            .load_dict(&mut f)
            .map_err(|e| Error::Schema(format!("加载用户词典 {} 失败: {e}", path.display())))?;
        let n = words.len();
        for w in words {
            self.user_words.insert(w);
        }
        self.dict_label = format!("{} + user:{}", self.dict_label, path.display());
        Ok(n)
    }

    #[inline]
    pub fn has_word(&self, w: &str) -> bool {
        self.jieba.has_word(w)
    }

    #[inline]
    pub fn is_user_word(&self, w: &str) -> bool {
        self.user_words.contains(w)
    }

    /// 词条标记位：`in_dict` 与 `from_user`。
    #[inline]
    pub fn flags(&self, w: &str) -> u8 {
        let mut f = 0u8;
        if self.jieba.has_word(w) {
            f |= FLAG_IN_DICT;
        }
        if self.user_words.contains(w) {
            f |= FLAG_FROM_USER;
        }
        f
    }

    /// 该 token 是否计入词表。
    ///
    /// 单趟遍历同时算出长度与字符类别。早期实现先 `chars().count()` 再遍历一次分类，
    /// 每个 token 要多走两遍字符；在 26 亿 token 的规模上这不是可以忽略的开销。
    pub fn accept(&self, w: &str) -> bool {
        let mut n = 0usize;
        let mut cjk = 0usize;
        let mut latin = 0usize;
        let mut digit = 0usize;
        for c in w.chars() {
            n += 1;
            if n > self.opts.max_len {
                return false;
            }
            if c.is_whitespace() {
                return false;
            }
            if is_cjk_ideograph(c) {
                cjk += 1;
            } else if c.is_ascii_alphabetic() {
                latin += 1;
            } else if c.is_ascii_digit() {
                digit += 1;
            } else if c.is_alphanumeric() {
                // 全角字母/数字、其他文字
                latin += 1;
            }
        }
        if n < self.opts.min_len || (self.opts.skip_single_char && n == 1) {
            return false;
        }
        if cjk + latin + digit == 0 {
            return false; // 纯标点
        }
        if cjk == 0 {
            if latin == 0 && !self.opts.keep_digit {
                return false; // 纯数字
            }
            if latin > 0 && digit == 0 && !self.opts.keep_latin {
                return false; // 纯英文
            }
        }
        true
    }

    /// 分词并把结果计入 `counts`，返回本次计入的 token 数与汉字数。
    ///
    /// 字表**不受词过滤影响**：即使某个 token 因长度或类型被词表拒绝，其中的汉字
    /// 仍会进字表，否则单字频率会因为过滤规则而失真。
    ///
    /// 返回计数而不是让调用方事后 `word_total()` 求和，是因为后者是 O(去重词数)，
    /// 放在逐段循环里会退化成灾难性的复杂度。
    pub fn count_into(&self, text: &str, counts: &mut LocalCounts) -> Counted {
        let mut c = Counted::default();
        for t in self.jieba.cut(text, self.opts.hmm) {
            let w = t.word;
            if !w.is_ascii() {
                for ch in w.chars() {
                    if is_cjk_ideograph(ch) {
                        counts.add_char(ch);
                        c.chars += 1;
                    }
                }
            }
            if self.accept(w) {
                counts.add_word(w);
                c.words += 1;
            }
        }
        c
    }

    /// 生成带字节区间的分词结果，供命令行 `segment` 与前端高亮使用。
    pub fn segment(&self, text: &str) -> Vec<Segment> {
        self.jieba
            .cut(text, self.opts.hmm)
            .into_iter()
            .map(|t| {
                let single_cjk = {
                    let mut it = t.word.chars();
                    matches!((it.next(), it.next()), (Some(c), None) if is_cjk_ideograph(c))
                };
                Segment {
                    text: t.word.to_string(),
                    byte_start: t.byte_start,
                    byte_end: t.byte_end,
                    accepted: self.accept(t.word),
                    single_cjk,
                }
            })
            .collect()
    }
}

/// 读词典文件里的词（忽略词频与词性），用于统计用户词典规模与记录来源。
fn read_dict_words(path: &Path) -> Result<Vec<Box<str>>> {
    use std::io::BufRead;
    let f = BufReader::new(File::open(path)?);
    let mut out = Vec::new();
    for line in f.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(w) = line.split_whitespace().next() {
            out.push(w.into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tk() -> Tokenizer {
        Tokenizer::builtin(TokenizeOpts::default())
    }

    #[test]
    fn splits_known_compound_words() {
        let t = tk();
        let segs = t.segment("中华人民共和国成立了");
        assert!(
            segs.iter().any(|s| s.text == "中华人民共和国"),
            "词典内的词应被切成一个 token: {:?}",
            segs.iter().map(|s| &s.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hmm_off_leaves_oov_word_fragmented() {
        // 实测「元宇宙」不在 jieba 词典中
        let t = tk();
        let segs = t.segment("元宇宙概念");
        let joined: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
        assert!(
            !joined.contains(&"元宇宙"),
            "关闭 HMM 时词典外词必然被切碎: {joined:?}"
        );
    }

    #[test]
    fn has_word_matches_dict_contents() {
        let t = tk();
        assert!(t.has_word("人工智能"));
        assert!(!t.has_word("元宇宙"));
        assert!(!t.has_word("带带大师兄"));
    }

    #[test]
    fn accept_rejects_punctuation_and_keeps_cjk() {
        let t = tk();
        assert!(t.accept("中国"));
        assert!(!t.accept("，"));
        assert!(!t.accept(" "));
        assert!(!t.accept("——"));
        assert!(t.accept("iPhone"));
        assert!(!t.accept("2023"), "默认丢弃纯数字");
    }

    #[test]
    fn char_table_ignores_word_filter() {
        let opts = TokenizeOpts { skip_single_char: true, ..Default::default() };
        let t = Tokenizer::builtin(opts);
        let mut c = LocalCounts::new();
        t.count_into("中", &mut c);
        assert!(c.words.is_empty(), "单字词被词表过滤");
        assert_eq!(c.chars['中' as usize], 1, "但字表仍应记到");
    }

    #[test]
    fn segments_carry_byte_offsets_covering_input() {
        let t = tk();
        let text = "你好，世界！";
        let segs = t.segment(text);
        assert_eq!(segs.first().unwrap().byte_start, 0);
        assert_eq!(segs.last().unwrap().byte_end, text.len());
        // 区间应连续覆盖
        for w in segs.windows(2) {
            assert_eq!(w[0].byte_end, w[1].byte_start, "分词区间应无缝拼接");
        }
    }

    #[test]
    fn single_cjk_flagged_for_char_table_lookup() {
        let t = tk();
        let segs = t.segment("中国的");
        for s in &segs {
            let expect = s.text.chars().count() == 1
                && is_cjk_ideograph(s.text.chars().next().unwrap());
            assert_eq!(s.single_cjk, expect, "token {:?}", s.text);
        }
    }
}
