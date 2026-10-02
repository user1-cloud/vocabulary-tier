//! VocTier 词频统计核心库。
//!
//! 设计约束来自实测（见 `docs/DESIGN.md`）：
//! 1. 语料含非法 UTF-8 行 → 全程按字节读行，坏行只跳过、绝不断流；
//! 2. 丢弃区间边界半行必须复用同一个 `BufReader`，否则析构会带走已缓冲数据；
//! 3. 计数用每线程独占哈希表 + 末尾归并，绝不能用互斥锁分片表。

pub mod artifact;
pub mod clean;
pub mod compose;
pub mod count;
pub mod dict;
pub mod merge;
pub mod query;
pub mod rank;
pub mod scan;
pub mod source;
pub mod tokenize;

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// 语料库格式无法识别，或规则配置有误
    Schema(String),
    /// 产物格式错误或版本不匹配
    Format(String),
    /// 词库文件读不了、格式不对，或找不到该用的词库
    Dict(String),
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "IO 错误: {e}"),
            Error::Json(e) => write!(f, "JSON 错误: {e}"),
            Error::Schema(m) => write!(f, "语料库格式错误: {m}"),
            Error::Format(m) => write!(f, "产物格式错误: {m}"),
            Error::Dict(m) => write!(f, "词库错误: {m}"),
            Error::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

/// 一次统计扫描的累计统计量。
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Totals {
    pub files: u64,
    pub bytes: u64,
    pub lines: u64,
    pub paras: u64,
    pub tokens: u64,
    /// UTF-8 非法或 JSON 解析失败、被跳过的行数
    pub bad_lines: u64,
}

impl Totals {
    pub fn add(&mut self, o: &Totals) {
        self.files += o.files;
        self.bytes += o.bytes;
        self.lines += o.lines;
        self.paras += o.paras;
        self.tokens += o.tokens;
        self.bad_lines += o.bad_lines;
    }
}

/// 工具版本，写进 `meta.json`。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
