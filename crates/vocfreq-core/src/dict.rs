//! 词库（jieba 格式词典文件）的读取、校验、内容指纹与装载。
//!
//! 词库从「编进 exe 的常量」变成了「数据文件夹里的一份普通文件」，于是这个模块
//! 要承担三件 jieba-rs 自己不管的事：
//!
//! 1. **跳过注释与空行**。jieba 的 `load_dict` 会把 `#` 开头的行也当词条解析：
//!    第二列不是整数就返回 `InvalidDictEntry` 直接失败；若第二列恰好是数字，
//!    则会把 `#` 本身当成一个词静默插进词典。而本项目的 [`crate::artifact::write_oov`]
//!    产出的候选词文件正带着 `#` 注释头，文档还明确要求用户拿它去喂用户词典 ——
//!    所以这条路径必须由我们自己处理注释。词库现在用户可编辑，注释只会更多。
//!
//! 2. **整份先校验、再装载**。jieba 的 `load_dict` 一进门就把 `total` 归零，
//!    中途出错又**不会**走 `finish_load()`，于是分词器会停在「半装载」状态
//!    （`total` 与 `log_total` 打架、部分词已进 trie）。这里先把文件完整读进内存、
//!    逐行校验，确认全文件无误后才喂给 jieba，因此不存在加载到一半的中间态。
//!
//! 3. **内容指纹**。词库一旦可由用户替换、删除、自建，`meta.json` 里就必须记住
//!    「这张表是基于哪一份词库生成的」（见 [`DictRef`]）。否则用户换了词库再去查
//!    旧表，会静默拿到系统性偏错的频次 —— 正是本项目在别处极力避免的那类错误。
//!
//! 顺带一个语义选择：jieba 的 `load_dict` 把**省略词频**的条目按 `0` 处理，而
//! `freq = 0` 意味着路径概率 `ln(0) = -inf`，该词会被登记进词典（`has_word` 为真）
//! 却**永远不可能被切分出来**。文档说词频可省略，用户省略了却是这个结果，属于
//! 隐蔽的坑。本模块改为走 jieba 自己的 [`jieba_rs::Jieba::add_word`]（`freq = None`
//! 时由 `suggest_freq` 折算），与 Python 版 jieba 的行为一致，并在
//! [`DictLoadReport`] 里如实报出条数，界面与日志都能看见。

use std::fmt::Write as _;
use std::fs::File;
use std::io::{Cursor, Read as _};
use std::path::{Path, PathBuf};

use jieba_rs::Jieba;
use serde::{Deserialize, Serialize, de};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// sha256 十六进制串的长度。
pub const SHA256_HEX_LEN: usize = 64;

/// 词库文件的扩展名。数据文件夹里只认这个后缀，避免把 README 之类也当词库。
pub const DICT_EXT: &str = "dict";

// ===========================================================================
// 词库身份
// ===========================================================================

/// 一份词库的身份，写进 `meta.json`。
///
/// **序列化永远写成对象**，但反序列化额外接受一个纯字符串 —— 那是
/// `schema_version = 1` 的老产物写法，例如
/// `"dict": "builtin(jieba dict.txt, 349046 entries)"`。那种产物没有指纹，
/// 无从校验，只能降级成「有名字但不可验证」，见 [`DictRef::legacy`]。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictRef {
    /// 稳定标识。当前实现取文件名去掉 `.dict`（中文名照留），同一名字的词库靠
    /// `sha256` 区分。**写进产物后不要改名**，它是表与词库之间的人可读连接键。
    pub id: String,
    /// 展示名（默认同 [`Self::id`]）。
    pub name: String,
    /// 装载时的绝对路径。换机器、换盘、搬目录就失效，所以**校验只认 `sha256`**，
    /// 这个字段仅供排查与界面提示"它当时在哪"。
    pub path: String,
    /// 实际装载的**有效**词条数（已剔除注释与空行）。
    pub entries: u64,
    /// 词库文件内容的 sha256（小写十六进制）。
    ///
    /// `schema_version = 1` 的老产物为空串 —— 用 [`Self::is_verifiable`] 判断。
    pub sha256: String,
}

impl DictRef {
    /// `schema_version = 1` 的老产物只有一句自由文本标签，没有指纹。
    ///
    /// 这种身份**不可校验**：读取方必须降级成「不知道对应哪份词库」并继续工作，
    /// 而不是报错 —— 老产物本身是完全合法的，只是当年没有这个概念。
    pub fn legacy(label: &str) -> Self {
        DictRef {
            id: String::new(),
            name: label.to_string(),
            path: String::new(),
            entries: 0,
            sha256: String::new(),
        }
    }

    /// 有没有可用的内容指纹。
    pub fn is_verifiable(&self) -> bool {
        self.sha256.len() == SHA256_HEX_LEN
    }

    /// 两份词库的内容是否相同。
    ///
    /// 返回 `None` 表示**不可判定**（任一方没有指纹）。调用方不能用 `false` 顶替
    /// `None`：老产物跟任何词库都"不可判定"，报成"不一致"会让所有老产物一起报错。
    pub fn same_content(&self, other: &DictRef) -> Option<bool> {
        if !self.is_verifiable() || !other.is_verifiable() {
            return None;
        }
        Some(self.sha256.eq_ignore_ascii_case(&other.sha256))
    }

    /// 展示用短描述：`预制词库（349046 条 / a1b2c3d4）`。
    pub fn short(&self) -> String {
        if !self.is_verifiable() {
            return format!("{}（{} 条 / 无指纹）", self.name, self.entries);
        }
        let head = self.sha256.get(..8).unwrap_or(&self.sha256);
        format!("{}（{} 条 / {head}）", self.name, self.entries)
    }
}

impl<'de> Deserialize<'de> for DictRef {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Obj {
            #[serde(default)]
            id: String,
            #[serde(default)]
            name: String,
            #[serde(default)]
            path: String,
            #[serde(default)]
            entries: u64,
            #[serde(default)]
            sha256: String,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Obj(Obj),
            /// v1：`"dict": "builtin(jieba dict.txt, 349046 entries)"`
            Label(String),
        }

        Ok(match Repr::deserialize(d)? {
            Repr::Obj(o) => DictRef {
                id: o.id,
                name: o.name,
                path: o.path,
                entries: o.entries,
                sha256: o.sha256,
            },
            Repr::Label(l) => DictRef::legacy(&l),
        })
    }
}

// ===========================================================================
// 词条与读取报告
// ===========================================================================

/// 一行有效词条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictEntry {
    pub word: String,
    /// jieba 格式第二列：**分词用的概率权重，不是词频**（见 `docs/DESIGN.md`）。
    ///
    /// `None` 表示文件里省略了这一列 —— 与"显式写了 0"是两回事，后者会让该词
    /// 永远切不出来，所以必须分开表达。
    pub freq: Option<usize>,
    /// 第三列词性，可省略。
    pub tag: String,
}

/// 读取一份词库时的统计。界面用它提示「这份词库有 N 条没写词频」之类的隐患。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DictLoadReport {
    /// 有效词条数
    pub entries: u64,
    /// 跳过的注释行数
    pub comments: u64,
    /// 跳过的空行数
    pub blanks: u64,
    /// 省略了词频列、装载时按 `suggest_freq` 折算的条目数
    pub freq_omitted: u64,
    /// 显式写了 `0` 的条目数。
    ///
    /// 这类词会被登记进词典（`has_word` 为真，产物里的 `in_dict` 标记也是真），
    /// 但路径概率是 `ln(0) = -inf`，**永远不可能被切分出来**。几乎总是笔误。
    pub freq_zero: u64,
}

/// 一份词库读进来的全部结果。
#[derive(Debug, Clone)]
pub struct ReadDict {
    pub entries: Vec<DictEntry>,
    pub dict: DictRef,
    pub report: DictLoadReport,
}

// ===========================================================================
// 读取与校验
// ===========================================================================

/// 读取并校验一份词库。
///
/// 文件**只读一遍**：先整个读进内存（词库是几 MB 量级），据此算指纹，再按行解析。
///
/// 格式规则（比 jieba 自己宽松，但**不静默**）：
/// * 以 `#` 开头的**整行**是注释。行内出现的 `#` 不算注释。
///   ⚠ 代价是**以 `#` 开头的词无法表达**（如 `#话题`）—— 这是为了让
///   [`crate::artifact::write_oov`] 的注释头和用户手写的注释都能直接用，
///   属于刻意取舍，见测试 `hash_at_line_start_is_always_a_comment`。
/// * 空行跳过
/// * `词`、`词 词频`、`词 词频 词性` 三种都接受（`docs/DESIGN.md` 说词频可省略）
/// * 词频列写了非整数、或者一行超过 3 列 → **报错并指出行号**。
///   jieba 会静默忽略第三列之后的内容，用户会以为词加进去了，其实没有。
pub fn read_dict(path: &Path) -> Result<ReadDict> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::Dict(format!("打开词库 {} 失败: {e}", path.display())))?;

    let sha256 = sha256_hex(&bytes);
    // 词库理论上可以有非 UTF-8 的脏字节；这里宽容处理，坏字节按替换字符走，
    // 后续解析会因为列数/词频不合法而报出具体行号，好过整份读不了。
    let text = String::from_utf8_lossy(&bytes);

    let mut entries: Vec<DictEntry> = Vec::new();
    let mut report = DictLoadReport::default();

    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            report.blanks += 1;
            continue;
        }
        if line.starts_with('#') {
            report.comments += 1;
            continue;
        }

        let line_no = idx + 1;
        let mut it = line.split_whitespace();
        // 上面已确认非空，所以 next() 一定有值
        let Some(word) = it.next() else { continue };

        let freq = match it.next() {
            None => {
                report.freq_omitted += 1;
                None
            }
            Some(col) => Some(col.parse::<usize>().map_err(|_| {
                Error::Dict(format!(
                    "{} 第 {line_no} 行的词频 `{col}` 不是非负整数。\
                     格式是「词 词频 词性」，词频与词性都可省略，以 # 开头的整行是注释；\
                     若词里含空格，jieba 无法表达，请先把空格去掉",
                    path.display()
                ))
            })?),
        };

        let tag = it.next().unwrap_or("");
        if it.next().is_some() {
            return Err(Error::Dict(format!(
                "{} 第 {line_no} 行超过 3 列（词 词频 词性）：`{line}`。\
                 jieba 会静默丢掉多余的列，所以这里直接报错",
                path.display()
            )));
        }

        if freq == Some(0) {
            report.freq_zero += 1;
        }

        entries.push(DictEntry {
            word: word.to_string(),
            freq,
            tag: tag.to_string(),
        });
    }

    report.entries = entries.len() as u64;
    let name = display_name(path);
    let dict = DictRef {
        id: name.clone(),
        name,
        path: path.display().to_string(),
        entries: report.entries,
        sha256,
    };

    Ok(ReadDict {
        entries,
        dict,
        report,
    })
}

/// 词库的展示名 = 文件名去掉 `.dict`。
pub fn display_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 一个路径是不是词库文件（只看扩展名，大小写不敏感）。
pub fn is_dict_file(path: &Path) -> bool {
    path.extension()
        .map(|e| e.eq_ignore_ascii_case(DICT_EXT))
        .unwrap_or(false)
}

/// 列出一个目录里的词库文件，按文件名排序。
///
/// 排序是为了让「把文件扔进文件夹就生效」这件事**可复现** —— 顺序不同，
/// 同名条目的覆盖结果就不同。目录不存在时返回空表（不是错误：数据文件夹
/// 可能还没建起来）。
pub fn list_dicts(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_dict_file(p))
        .collect();
    out.sort();
    out
}

/// 流式算一份文件的 sha256（十六进制小写）。
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f =
        File::open(path).map_err(|e| Error::Dict(format!("打开 {} 失败: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// 字节串的 sha256（十六进制小写）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    to_hex(&hasher.finalize())
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // 写进 String 不会失败
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ===========================================================================
// 装载
// ===========================================================================

/// 把已经 [`read_dict`] 校验过的词条装进 `jieba`。
///
/// 两条路径，按「这份文件有没有省略词频」来选：
///
/// * **全部写了词频**（常见情况，包括内置那份 349,046 条的词库）→ 拼成
///   `词 词频 词性` 文本，一次 `load_dict` 装入。与 jieba 自己的 `finish_load`
///   语义完全一致，也最快。
/// * **有省略词频的条目** → 逐条 [`Jieba::add_word`]，省略词频的交给 `suggest_freq`
///   折算。必须**逐条且保持文件原序**：`suggest_freq` 的结果取决于「此刻词典里
///   已经有什么」，重排会让同一份文件得出不同结果，破坏可复现性。
///
/// 同名条目**后者覆盖前者的词频**（jieba `load_dict` 的既有语义：已存在的词只改
/// 词频，不新增记录）。所以词库链的顺序有意义。
pub fn install(jieba: &mut Jieba, entries: &[DictEntry]) -> Result<()> {
    let has_omitted = entries.iter().any(|e| e.freq.is_none());

    if !has_omitted {
        let mut buf = String::with_capacity(entries.len() * 16);
        for e in entries {
            let freq = e.freq.unwrap_or(0);
            if e.tag.is_empty() {
                let _ = writeln!(buf, "{} {}", e.word, freq);
            } else {
                let _ = writeln!(buf, "{} {} {}", e.word, freq, e.tag);
            }
        }
        jieba
            .load_dict(&mut Cursor::new(buf.as_bytes()))
            .map_err(|e| Error::Dict(format!("装载词库失败: {e}")))?;
    } else {
        for e in entries {
            let tag = if e.tag.is_empty() {
                None
            } else {
                Some(e.tag.as_str())
            };
            jieba.add_word(&e.word, e.freq, tag);
        }
    }
    Ok(())
}

/// 按给定顺序装载一整条词库链，返回每个成员的身份与读取报告。
///
/// `jieba` 应当从 [`Jieba::empty`] 开始 —— 现在已经没有"内置词典"这回事了。
pub fn load_chain(jieba: &mut Jieba, paths: &[PathBuf]) -> Result<Vec<(DictRef, DictLoadReport)>> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let rd = read_dict(p)?;
        install(jieba, &rd.entries)?;
        out.push((rd.dict, rd.report));
    }
    Ok(out)
}

/// 把整条链的身份压缩成一句给日志/界面看的话。
pub fn describe_chain(chain: &[DictRef]) -> String {
    match chain.len() {
        0 => "（无词库）".to_string(),
        1 => chain[0].short(),
        _ => {
            let names: Vec<&str> = chain.iter().map(|d| d.name.as_str()).collect();
            format!("{}（共 {} 份）", names.join(" + "), chain.len())
        }
    }
}

/// 测试用的临时词库文件。
///
/// 放在这里供 `dict` 与 `tokenize` 两个测试模块共用 —— 两个模块都需要"造一份小词库"，
/// 各自抄一遍容易走样。
#[cfg(test)]
pub(crate) mod testutil {
    use std::io::Write as _;
    use std::path::{Path, PathBuf};

    /// 落一个临时 `.dict` 文件，`Drop` 时删掉。
    pub(crate) struct TempDict(PathBuf);

    impl TempDict {
        pub(crate) fn new(name: &str, body: &str) -> Self {
            // ⚠ 名字里必须带**进程内唯一的序号**，只带进程号是不够的：
            // `cargo test` 默认并行跑，两个测试用同一个 `name` 时第二个 `File::create`
            // 会把第一个正在读的文件截断 —— 症状是随机的 `has_word` 失败或
            // 「第 1 行不是合法词条」，与真实代码毫无关系（实测复现率约 1/3）。
            static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut p = std::env::temp_dir();
            p.push(format!(
                "vocfreq-test-{}-{seq}-{name}.dict",
                std::process::id()
            ));
            let mut f = std::fs::File::create(&p).expect("建临时词库");
            f.write_all(body.as_bytes()).expect("写临时词库");
            f.flush().expect("刷临时词库");
            TempDict(p)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDict {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::TempDict;
    use super::*;

    #[test]
    fn comments_and_blanks_are_skipped() {
        // 这正是 jieba 自己的 load_dict 处理不了的那类文件 ——
        // 而 artifact::write_oov 产出的候选词文件就长这样，且文档让用户拿它喂用户词典。
        let f = TempDict::new(
            "comments",
            "# 这是注释\n\n# 词频列不是数字也不该报错\n對 731971\n業 620532\n\n",
        );
        let rd = read_dict(f.path()).expect("带注释的词库必须能读");
        assert_eq!(rd.entries.len(), 2, "只应剩两条真词条");
        assert_eq!(rd.entries[0].word, "對");
        assert_eq!(rd.entries[0].freq, Some(731971));
        assert_eq!(rd.report.comments, 2);
        assert_eq!(rd.report.blanks, 2);
        assert_eq!(rd.report.entries, 2);
    }

    #[test]
    fn hash_at_line_start_is_always_a_comment() {
        // 这是**刻意的取舍**：`#` 开头的整行一律当注释。
        //
        // 好处：artifact::write_oov 产出的候选词文件开头有 10 行 `#` 注释（而文档
        // 明确要求用户拿它去喂词库），用户手写的注释也能直接用。
        // 代价：**以 `#` 开头的词无法在词库里表达**，比如微博话题词 `#话题`。
        //
        // 两害相权：注释的 `#` 后面不带空格很常见，而词以 `#` 开头很罕见 ——
        // 所以宁可让前者工作。真要收录这类词，只能改写清洗规则去掉前导 `#`。
        let f = TempDict::new("hashline", "#话题 100 n\n# 真注释\n甲 5\n");
        let rd = read_dict(f.path()).expect("读词库");
        let words: Vec<&str> = rd.entries.iter().map(|e| e.word.as_str()).collect();
        assert_eq!(words, vec!["甲"], "`#` 开头的行应全部当注释丢掉");
        assert_eq!(rd.report.comments, 2);
    }

    #[test]
    fn word_without_freq_is_allowed_and_reported() {
        let f = TempDict::new("nofreq", "元宇宙\n区块链 500\n");
        let rd = read_dict(f.path()).expect("词频可省略");
        assert_eq!(rd.entries.len(), 2);
        assert_eq!(rd.entries[0].freq, None);
        assert_eq!(rd.entries[1].freq, Some(500));
        assert_eq!(rd.report.freq_omitted, 1);
    }

    #[test]
    fn explicit_zero_freq_is_reported_separately() {
        // 0 与"省略"必须分开：0 的词路径概率是 -inf，永远切不出来
        let f = TempDict::new("zerofreq", "甲 0\n乙 5\n");
        let rd = read_dict(f.path()).expect("显式 0 合法");
        assert_eq!(rd.entries[0].freq, Some(0));
        assert_eq!(rd.report.freq_zero, 1);
        assert_eq!(rd.report.freq_omitted, 0);
    }

    #[test]
    fn bad_freq_points_at_the_line_number() {
        let f = TempDict::new("badfreq", "甲 100\n乙 不是数字\n");
        let err = read_dict(f.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("第 2 行"), "要指出行号，实际是：{msg}");
    }

    #[test]
    fn extra_columns_are_rejected_not_ignored() {
        // jieba 自己会静默丢掉第三列之后的内容 —— 用户会以为词加进去了，其实没有
        let f = TempDict::new("extracol", "甲 100 n 多余\n");
        let err = read_dict(f.path()).unwrap_err();
        assert!(err.to_string().contains("超过 3 列"), "实际是：{err}");
    }

    #[test]
    fn word_with_space_surfaces_as_a_freq_error_with_a_hint() {
        // 词里带空格（jieba 格式无法表达）会被 split_whitespace 拆到第二列，
        // 报出来的错是"词频不是数字"。错误信息里必须点出这个可能的原因，
        // 否则用户对着 `新 词 100 n` 完全想不通哪里错了。
        let f = TempDict::new("spaceword", "新 词 100 n\n");
        let err = read_dict(f.path()).unwrap_err().to_string();
        assert!(err.contains("含空格"), "错误信息要提示这个原因：{err}");
    }

    #[test]
    fn missing_freq_word_is_actually_segmentable() {
        // 回归：jieba 的 load_dict 把省略词频当 0，于是该词 has_word 为真
        // 却永远切不出来。我们改走 add_word/suggest_freq，它必须真能成词。
        let f = TempDict::new("seg", "元宇宙\n");
        let rd = read_dict(f.path()).expect("读词库");
        let mut jieba = Jieba::empty();
        install(&mut jieba, &rd.entries).expect("装词库");
        assert!(jieba.has_word("元宇宙"), "应登记进词典");
        let toks: Vec<&str> = jieba
            .cut("元宇宙概念", false)
            .iter()
            .map(|t| t.word)
            .collect();
        assert!(
            toks.contains(&"元宇宙"),
            "省略词频的词也必须能切出来，实际切成了 {toks:?}"
        );
    }

    #[test]
    fn chain_order_lets_later_dicts_override_freq() {
        let a = TempDict::new("chain-a", "苹果 1\n");
        let b = TempDict::new("chain-b", "苹果 999999\n");
        let mut jieba = Jieba::empty();
        let chain = load_chain(
            &mut jieba,
            &[a.path().to_path_buf(), b.path().to_path_buf()],
        )
        .expect("装词库链");
        assert_eq!(chain.len(), 2);
        // 后者覆盖前者：把「苹果」整成一个词的倾向应当变得很强
        let toks: Vec<&str> = jieba.cut("苹果", false).iter().map(|t| t.word).collect();
        assert_eq!(toks, vec!["苹果"], "覆盖后的词频应让整词胜出");
    }

    #[test]
    fn dict_ref_hash_is_stable_and_change_sensitive() {
        let a = TempDict::new("hash-a", "甲 1\n");
        let b = TempDict::new("hash-b", "甲 2\n");
        let ra = read_dict(a.path()).unwrap().dict;
        let rb = read_dict(b.path()).unwrap().dict;
        assert!(ra.is_verifiable());
        assert_eq!(ra.sha256.len(), SHA256_HEX_LEN);
        assert_eq!(ra.same_content(&ra), Some(true));
        assert_eq!(ra.same_content(&rb), Some(false), "内容不同必须能看出来");
    }

    #[test]
    fn legacy_string_form_deserializes_as_unverifiable() {
        // v1 产物写的是纯字符串，必须还能读，且被判为"不可校验"
        let r: DictRef = serde_json::from_str("\"builtin(jieba dict.txt, 349046 entries)\"")
            .expect("v1 的字符串形式必须能反序列化");
        assert!(!r.is_verifiable());
        assert_eq!(r.entries, 0);
        assert!(r.name.contains("builtin"));
        // 不可判定 ≠ 不一致
        let real = read_dict(TempDict::new("legacy", "甲 1\n").path())
            .unwrap()
            .dict;
        assert_eq!(r.same_content(&real), None);
    }

    #[test]
    fn v2_object_form_round_trips() {
        let d = read_dict(TempDict::new("roundtrip", "甲 1 z\n").path())
            .unwrap()
            .dict;
        let json = serde_json::to_string(&d).unwrap();
        let back: DictRef = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn list_dicts_ignores_other_extensions_and_sorts() {
        let dir = std::env::temp_dir().join(format!("vocfreq-test-list-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (n, body) in [
            ("b.dict", "甲 1\n"),
            ("a.dict", "乙 1\n"),
            ("readme.txt", "不是词库\n"),
        ] {
            std::fs::write(dir.join(n), body).unwrap();
        }
        let found = list_dicts(&dir);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.dict", "b.dict"], "只认 .dict，且按名排序");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_dicts_on_missing_dir_is_empty_not_error() {
        let p = std::env::temp_dir().join("vocfreq-test-definitely-missing-dir");
        let _ = std::fs::remove_dir_all(&p);
        assert!(list_dicts(&p).is_empty());
    }
}
