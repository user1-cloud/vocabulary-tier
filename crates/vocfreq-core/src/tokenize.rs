//! 分词与过滤。
//!
//! 关键决策（`docs/DESIGN.md` §3）：
//! * 默认 **关闭 HMM**：开启后同一实体在不同上下文可能被切成不同形态，导致频次被
//!   拆散、排行不可复现。实测关闭时 jieba 只输出「词典命中的词 + 未命中的单字」。
//! * 词典**外置**：不再有"编进 exe 的内置词典"这回事，分词器一律由一条**词典链**
//!   建起来（[`Tokenizer::from_dicts`]），链的顺序有意义 —— 同名条目**后者覆盖前者**的
//!   词频。链上每一份的身份（含 `sha256` 指纹）与读取报告都记在 [`Tokenizer::dicts`] /
//!   [`Tokenizer::dict_reports`] 里，写进 `meta.json` 供之后校验。
//! * 词典第三列是**分词用的概率权重**，不是词频（实测 45.6% 的条目权重都是保底值
//!   3），因此稀有度一律由语料统计得出，不使用它。
//!
//! 词典文件的读取与校验**不在这里** —— 那部分在 [`crate::dict`]，它负责剥注释、按行报错、
//! 算指纹，并把整份先读进内存再装进 jieba。这里只负责"把一条链装起来"与分词过滤。

use std::path::{Path, PathBuf};

use jieba_rs::Jieba;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use crate::clean::is_cjk_ideograph;
use crate::count::LocalCounts;
use crate::dict::{self, DictLoadReport, DictRef};
use crate::{Error, Result};

/// 词条标记位。
pub const FLAG_IN_DICT: u8 = 1 << 0;
/// 该词来自**叠加词典**（词典链上除第一份之外的成员）。
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

#[derive(Debug)]
pub struct Tokenizer {
    jieba: Jieba,
    /// 叠加词典带来的词（词典链上第 2 份及之后），用于标记 `from_user`。
    user_words: FxHashSet<Box<str>>,
    pub opts: TokenizeOpts,
    /// 词典链，**按装载顺序**：`dicts[0]` 是主词典。每份都带内容指纹。
    ///
    /// 它会被原样写进 `meta.json` 的 `tokenizer.dicts`，所以**不要**在别处再拼一份 ——
    /// 产物与运行时用的一定是同一份记录。
    pub dicts: Vec<DictRef>,
    /// 与 [`Self::dicts`] **同序**的读取报告（有效条目数、跳过的注释/空行、
    /// 省略词频与显式 0 的条数）。用来在日志里提示隐患。
    pub dict_reports: Vec<DictLoadReport>,
}

/// `count_into` 的计数结果。
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counted {
    /// 计入词频表的 token 数
    pub words: u64,
    /// 计入字表的汉字数
    pub chars: u64,
}

impl Tokenizer {
    /// 由**一条词典链**建分词器。链不能为空。
    ///
    /// 空链直接报错，而不是拿一份空词典跑出一张只有单字的废表：词典不再编进程序里了，
    /// "没有词典"永远是配置问题，必须让用户看见。
    ///
    /// 顺序有意义：同名条目**后者覆盖前者**的词频（jieba 的既有语义），所以
    /// `dicts[0]` 是主词典，其后都是叠加。第 2 份及之后的词会被标上
    /// [`FLAG_FROM_USER`]（界面上叫「来自叠加词典」）。
    pub fn from_dicts(paths: &[PathBuf], opts: TokenizeOpts) -> Result<Self> {
        if paths.is_empty() {
            return Err(Error::Dict(
                "没有词典：现在词典不再编进程序里，必须显式指定一条词典链（至少一份 .dict）。\
                 命令行用 --dict / --dict-dir，桌面端的数据文件夹里默认放在 dicts\\ 下。"
                    .into(),
            ));
        }
        let mut jieba = Jieba::empty();
        let mut chain: Vec<DictRef> = Vec::with_capacity(paths.len());
        let mut reports: Vec<DictLoadReport> = Vec::with_capacity(paths.len());
        let mut user_words: FxHashSet<Box<str>> = FxHashSet::default();
        for (i, p) in paths.iter().enumerate() {
            let rd = dict::read_dict(p).map_err(|e| {
                Error::Dict(format!(
                    "装载词典链失败（第 {} 份 / 共 {} 份）：{e}",
                    i + 1,
                    paths.len()
                ))
            })?;
            // 整份先读进内存、逐行校验过了才喂给 jieba，因此不会有"半装载"的中间态
            dict::install(&mut jieba, &rd.entries)?;
            if i > 0 {
                for e in &rd.entries {
                    user_words.insert(e.word.as_str().into());
                }
            }
            chain.push(rd.dict);
            reports.push(rd.report);
        }
        Ok(Tokenizer {
            jieba,
            user_words,
            opts,
            dicts: chain,
            dict_reports: reports,
        })
    }

    /// 单份词典的便捷入口（等价于长度为 1 的链）。
    pub fn from_dict(path: &Path, opts: TokenizeOpts) -> Result<Self> {
        Self::from_dicts(std::slice::from_ref(&path.to_path_buf()), opts)
    }

    /// 词典链的一句话描述，写进日志（`meta.json` 里另有一份结构化记录）。
    pub fn dict_label(&self) -> String {
        dict::describe_chain(&self.dicts)
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

    /// 该 token 是否计入词频表。
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
    /// 字表**不受词过滤影响**：即使某个 token 因长度或类型被词频表拒绝，其中的汉字
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::testutil::TempDict;

    /// 主词典：让下面几句话的词能整词切出来（否则全是单字，没什么可测的）。
    fn main_dict() -> TempDict {
        TempDict::new(
            "tokenize-main",
            "中华人民共和国 1000000 n\n概念 1000000 n\n宇宙 1000000 n\n中国 1000000 n\n的 1000000 u\n",
        )
    }

    fn tk() -> (TempDict, Tokenizer) {
        let d = main_dict();
        let t =
            Tokenizer::from_dict(d.path(), TokenizeOpts::default()).expect("单份词典应能建分词器");
        (d, t)
    }

    #[test]
    fn splits_known_compound_words() {
        let (_d, t) = tk();
        let segs = t.segment("中华人民共和国成立了");
        assert!(
            segs.iter().any(|s| s.text == "中华人民共和国"),
            "词典内的词应被切成一个 token: {:?}",
            segs.iter().map(|s| &s.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hmm_off_leaves_oov_word_fragmented() {
        // 「元宇宙」不在上面那份主词典里
        let (_d, t) = tk();
        let segs = t.segment("元宇宙概念");
        let joined: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
        assert!(
            !joined.contains(&"元宇宙"),
            "关闭 HMM 时词典外词必然被切碎: {joined:?}"
        );
    }

    #[test]
    fn has_word_matches_dict_contents() {
        let (_d, t) = tk();
        assert!(t.has_word("中国"));
        assert!(!t.has_word("元宇宙"));
        assert!(!t.has_word("带带大师兄"));
    }

    #[test]
    fn accept_rejects_punctuation_and_keeps_cjk() {
        let (_d, t) = tk();
        assert!(t.accept("中国"));
        assert!(!t.accept("，"));
        assert!(!t.accept(" "));
        assert!(!t.accept("——"));
        assert!(t.accept("iPhone"));
        assert!(!t.accept("2023"), "默认丢弃纯数字");
    }

    #[test]
    fn char_table_ignores_word_filter() {
        let d = main_dict();
        let opts = TokenizeOpts {
            skip_single_char: true,
            ..Default::default()
        };
        let t = Tokenizer::from_dict(d.path(), opts).expect("建分词器");
        let mut c = LocalCounts::new();
        t.count_into("中", &mut c);
        assert!(c.words.is_empty(), "单字词被词频表过滤");
        assert_eq!(c.chars['中' as usize], 1, "但字表仍应记到");
    }

    #[test]
    fn segments_carry_byte_offsets_covering_input() {
        let (_d, t) = tk();
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
        let (_d, t) = tk();
        let segs = t.segment("中国的");
        for s in &segs {
            let expect =
                s.text.chars().count() == 1 && is_cjk_ideograph(s.text.chars().next().unwrap());
            assert_eq!(s.single_cjk, expect, "token {:?}", s.text);
        }
    }

    // ------------------------------------------------------------ 词典链

    #[test]
    fn overlay_dict_words_get_the_from_user_flag() {
        // 链的顺序有意义：主词典在前，叠加词典在后。叠加进来的词要能被认出来
        // （界面上的「用户词典」标记），主词典里的词则不能因此被误标。
        let base = TempDict::new("chain-base", "中国 100 n\n");
        let overlay = TempDict::new("chain-overlay", "元宇宙 100 n\n");
        let t = Tokenizer::from_dicts(
            &[base.path().to_path_buf(), overlay.path().to_path_buf()],
            TokenizeOpts::default(),
        )
        .expect("两份词典应能组成一条链");

        assert_eq!(t.dicts.len(), 2, "链上两份都要记进 meta");
        assert_eq!(t.dict_reports.len(), 2, "报告与链同序、同长");
        assert_eq!(t.dict_reports[0].entries, 1);
        assert_eq!(t.dict_reports[1].entries, 1);
        assert_eq!(t.dicts[0].entries, 1);
        assert_eq!(t.dicts[1].entries, 1);
        // 每份都带内容指纹 —— 这是跨产物合流的硬门槛（见 compose::check_mergeable）
        for d in &t.dicts {
            assert!(d.is_verifiable(), "{} 应当带 sha256", d.name);
        }
        // 叠加词典里的词：在词典里、且标为来自叠加
        assert!(t.has_word("元宇宙"));
        assert!(t.is_user_word("元宇宙"));
        assert_eq!(t.flags("元宇宙"), FLAG_IN_DICT | FLAG_FROM_USER);
        // 主词典里的词：在词典里，但**不是**来自叠加
        assert!(t.has_word("中国"));
        assert!(!t.is_user_word("中国"));
        assert_eq!(t.flags("中国"), FLAG_IN_DICT);
        // 词典链的一句话描述要能列全（写进日志用）
        let label = t.dict_label();
        assert!(
            label.contains("chain-base") && label.contains("chain-overlay"),
            "{label}"
        );
    }

    #[test]
    fn later_dict_wins_when_the_same_word_appears_twice() {
        // jieba 的既有语义：已存在的词只改词频，不新增记录。所以链的顺序决定谁覆盖谁，
        // 而 `from_user` 也只该标记"叠加进来的那一份"。
        let base = TempDict::new("dup-base", "甲 100 n\n");
        let overlay = TempDict::new("dup-overlay", "甲 900 n\n");
        let t = Tokenizer::from_dicts(
            &[base.path().to_path_buf(), overlay.path().to_path_buf()],
            TokenizeOpts::default(),
        )
        .expect("同名条目不应报错");
        assert!(t.has_word("甲"));
        assert_eq!(t.flags("甲"), FLAG_IN_DICT | FLAG_FROM_USER);
    }

    #[test]
    fn empty_chain_is_rejected_loudly() {
        // 词典不再编进程序里了，"没有词典"永远是配置问题，必须报错而不是跑出一张废表
        let e = Tokenizer::from_dicts(&[], TokenizeOpts::default()).expect_err("空链必须报错");
        assert!(e.to_string().contains("没有词典"), "{e}");
    }

    #[test]
    fn broken_dict_in_the_chain_points_at_which_one() {
        // 链上第 2 份坏了：错误里要指出是第几份、共几份（否则用户不知道去修哪一份）
        let good = TempDict::new("broken-good", "中国 100 n\n");
        let bad = TempDict::new("broken-bad", "甲 不是整数\n");
        let e = Tokenizer::from_dicts(
            &[good.path().to_path_buf(), bad.path().to_path_buf()],
            TokenizeOpts::default(),
        )
        .expect_err("坏词典必须报错");
        let msg = e.to_string();
        assert!(msg.contains("第 2 份"), "要指出是哪一份：{msg}");
        assert!(msg.contains("共 2 份"), "{msg}");
    }
}
