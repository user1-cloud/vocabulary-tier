//! 数据文件夹：**词典库**与**词频表库**。
//!
//! 从"词典编在 exe 里、词频表由用户随手指一个目录"改成下面这套模型：
//!
//! ```text
//! <数据文件夹>            ← 设置里可改，默认 %LOCALAPPDATA%\com.voctier.desktop\data
//!   dicts\
//!     <名字>.dict         ← 一个词典就是一个文件，随便增删改
//!   tables\
//!     <名字>\             ← 一张表就是一个产物目录
//!       meta.json  full\*.vfr  domains\*.vfr
//! ```
//!
//! 三条设计原则：
//!
//! 1. **预置的和用户自建的没有本质区别。** 安装包只是"帮你放了两个进去"，
//!    它们和用户自己新建的完全是同一种东西，同样可删可改名可替换。所以这里
//!    没有 `builtin` 之类的权限位，只有一个纯展示用的 [`Origin`]。
//! 2. **表与词典用内容指纹绑定，不是一个路径字符串。** 路径换台机器就失效，
//!    指纹不会。所以匹配顺序是「指纹 → 名字」：用户把词典文件改个名、但内容没变，
//!    表照样能用；内容变了则判为"已漂移"。词典**统一住在数据文件夹的 `dicts\` 里**，
//!    不追记录里那个可能早已失效的绝对路径 —— 那正是"统一放到一个文件夹"的意义。
//! 3. **每个失败都要说出来。** 坏的词典文件、绑不上的表、v1 老产物，都要带着
//!    具体原因出现在列表里，而不是被静默跳过 —— 用户得能看见"这个文件坏了"。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use vocfreq_core::artifact::Meta;
use vocfreq_core::dict::{self, DictLoadReport, DictRef};

/// 数据文件夹里放词典的子目录名。
pub const DICTS_DIR: &str = "dicts";
/// 数据文件夹里放词频表的子目录名。
pub const TABLES_DIR: &str = "tables";

// ===========================================================================
// 展示用的来源标记
// ===========================================================================

/// 这个词典/词频表是怎么来的。**纯展示用**，不挂任何行为 —— 预置项同样可以删除。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// 安装包预置的
    Seeded,
    /// 用户导入的
    Imported,
    /// 用户自己扫描出来的
    Scanned,
    /// 不知道（比如迁移过来的老设置指向的目录）
    Unknown,
}

// ===========================================================================
// 词典
// ===========================================================================

/// 词典列表里的一项。
#[derive(Debug, Clone, Serialize)]
pub struct DictItem {
    pub dict: DictRef,
    /// 读取失败时带原因。**读不了的文件也要列出来**，否则用户只会看到
    /// "我明明放进去了怎么没有"，永远发现不了文件本身有问题。
    pub error: Option<String>,
    pub report: DictLoadReport,
    /// 词典文件名（`dicts\` 下的相对名），改名/删除都靠它定位
    pub file_name: String,
    pub origin: Origin,
}

impl DictItem {
    /// 能不能真的拿来分词。
    pub fn usable(&self) -> bool {
        self.error.is_none()
    }
}

// ===========================================================================
// 词频表
// ===========================================================================

/// 表与它所记录的词典链之间的绑定状态。
///
/// 这是词典外置之后**必须**有的一块：词典可以由用户随手改，改了以后旧表的
/// 频次就跟分词口径对不上了。以前词典编在 exe 里，没有这个问题，也就没有这个概念。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Binding {
    /// 记录里的词典都能在当前数据文件夹里对上（指纹一致）
    Ok,
    /// v1 老产物：只留了一句自由文本描述，没有指纹，**无从判断**
    Legacy,
    /// 内容变了（同一个词典被改过）
    Drifted { changed: Vec<String> },
    /// 找不到了
    Missing { missing: Vec<String> },
}

/// 词频表列表里的一项。
#[derive(Debug, Clone, Serialize)]
pub struct TableItem {
    pub name: String,
    pub path: String,
    /// `meta.json` 解析成功时才有
    pub meta: Option<Meta>,
    /// 读不了的表也要列出来（目录被删了一半、meta.json 坏了…）
    pub error: Option<String>,
    pub binding: Binding,
    /// 是不是当前激活的那张
    pub active: bool,
    /// 是不是住在数据文件夹的 `tables\` 里。
    ///
    /// `false` 表示这是"指向数据文件夹之外的既有产物目录"（老设置迁移过来的，
    /// 比如开发时仓库里的 `data\`）。这类表**不能删** —— 那个目录不归我们管，
    /// 用户可能还有别的东西在里面。界面对它只给"激活"和"打开所在文件夹"。
    pub in_library: bool,
    pub origin: Origin,
}

// ===========================================================================
// 数据文件夹
// ===========================================================================

#[derive(Debug, Clone)]
pub struct Library {
    root: PathBuf,
}

impl Library {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Library { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dicts_dir(&self) -> PathBuf {
        self.root.join(DICTS_DIR)
    }

    pub fn tables_dir(&self) -> PathBuf {
        self.root.join(TABLES_DIR)
    }

    pub fn table_dir(&self, name: &str) -> PathBuf {
        self.tables_dir().join(name)
    }

    pub fn dict_path(&self, file_name: &str) -> PathBuf {
        self.dicts_dir().join(file_name)
    }

    /// 建出 `dicts\` 与 `tables\`。反复调用无副作用。
    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.dicts_dir())?;
        std::fs::create_dir_all(self.tables_dir())
    }

    /// 列出全部词典。
    ///
    /// ⚠ 会**完整读取并解析**每个 `.dict`（一份 4.84 MB 的 jieba 词典约 50–100 ms），
    /// 因为列表要显示有效词条数、注释行数、以及"多少条显式写了 0"这类隐患，
    /// 不读就说不出来。界面应当按需调用并缓存，不要每次重渲染都调。
    pub fn list_dicts(&self) -> Vec<DictItem> {
        let mut out = Vec::new();
        for p in dict::list_dicts(&self.dicts_dir()) {
            let file_name = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let origin = origin_of_file(&p);
            match dict::read_dict(&p) {
                Ok(rd) => out.push(DictItem {
                    dict: rd.dict,
                    error: None,
                    report: rd.report,
                    file_name,
                    origin,
                }),
                Err(e) => {
                    let name = dict::display_name(&p);
                    out.push(DictItem {
                        dict: DictRef {
                            id: name.clone(),
                            name,
                            path: p.display().to_string(),
                            entries: 0,
                            sha256: String::new(),
                        },
                        error: Some(e.to_string()),
                        report: DictLoadReport::default(),
                        file_name,
                        origin,
                    });
                }
            }
        }
        out
    }

    /// 列出数据文件夹 `tables\` 下的全部词频表。
    ///
    /// `current` 是 [`Self::list_dicts`] 的结果 —— 拿它判断每张表记录的词典链
    /// 现在是否还对得上。传进来而不是在这里重算，是为了避免把每个词典再哈希一遍。
    pub fn list_tables(&self, current: &[DictItem], active: Option<&str>) -> Vec<TableItem> {
        let Ok(rd) = std::fs::read_dir(self.tables_dir()) else {
            return Vec::new();
        };
        // 目录名排序，让界面顺序稳定
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        dirs.iter()
            .map(|dir| {
                let name = dir
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                table_item(dir, current, active == Some(name.as_str()), true)
            })
            .collect()
    }

    /// 把一份词典文件复制进数据文件夹，返回落地后的文件名。
    ///
    /// 重名时**自动加后缀，绝不覆盖** —— 用户可能正拿旧的那份跑着统计，
    /// 悄悄替换掉会让他下一次打开表时莫名其妙地"词典已变"。
    pub fn import_dict(&self, src: &Path) -> Result<String, String> {
        self.ensure().map_err(|e| format!("建数据文件夹失败：{e}"))?;
        if !src.is_file() {
            return Err(format!("{} 不是文件", src.display()));
        }
        if !dict::is_dict_file(src) {
            return Err(format!(
                "{} 不是词典文件（词典必须用 .dict 扩展名，内容为 jieba 的「词 词频 词性」）",
                src.display()
            ));
        }
        // 先读一遍：坏文件不该被导进来占地
        dict::read_dict(src).map_err(|e| e.to_string())?;

        let base = dict::display_name(src);
        let mut name = format!("{base}.dict");
        let mut n = 2;
        while self.dict_path(&name).exists() {
            name = format!("{base} ({n}).dict");
            n += 1;
        }
        std::fs::copy(src, self.dict_path(&name))
            .map_err(|e| format!("复制到 {} 失败：{e}", self.dict_path(&name).display()))?;
        mark_origin(&self.dict_path(&name), Origin::Imported);
        Ok(name)
    }

    /// 删掉一份词典。返回被删的文件名。
    pub fn delete_dict(&self, file_name: &str) -> Result<(), String> {
        let p = self.dict_path(file_name);
        if !p.exists() {
            return Err(format!("{} 不存在", p.display()));
        }
        std::fs::remove_file(&p).map_err(|e| format!("删除 {} 失败：{e}", p.display()))?;
        let _ = std::fs::remove_file(origin_sidecar(&p));
        Ok(())
    }

    /// 删掉一张表（整个产物目录）。
    pub fn delete_table(&self, name: &str) -> Result<(), String> {
        let dir = self.table_dir(name);
        if !dir.exists() {
            return Err(format!("{} 不存在", dir.display()));
        }
        std::fs::remove_dir_all(&dir).map_err(|e| format!("删除 {} 失败：{e}", dir.display()))?;
        let _ = std::fs::remove_file(origin_sidecar(&dir));
        Ok(())
    }
}

/// 把一个产物目录读成一项表信息。
///
/// **永远返回一项**，读不了也要列出来 —— 否则用户只会看到"我那张表怎么没了"，
/// 永远发现不了那个目录本身有问题。原因放在 [`TableItem::error`] 里。
///
/// `in_library` 决定界面给不给"删除"：数据文件夹之外的表不归我们管，
/// 那个目录里可能还有别的东西。
pub fn table_item(dir: &Path, current: &[DictItem], active: bool, in_library: bool) -> TableItem {
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = dir.display().to_string();
    let origin = origin_of_file(dir);

    let broken = |error: String| TableItem {
        name: name.clone(),
        path: path.clone(),
        meta: None,
        error: Some(error),
        binding: Binding::Legacy,
        active,
        in_library,
        origin,
    };

    let meta: Meta = match std::fs::read(dir.join("meta.json")) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(m) => m,
            Err(e) => return broken(format!("meta.json 解析失败：{e}")),
        },
        Err(e) => return broken(format!("读不到 meta.json：{e}")),
    };

    // 老布局（schema < 3）：表按 `full/word`、`domains/news/word` 这种"路径"组织，
    // 磁盘上是 `full/` 与 `domains/` 两棵树。新读取端按"每个表组一个目录"去找，
    // 硬读只会得到一句「文件不存在」。所以**认出它并说清楚要重扫**，不要让它以
    // 一个莫名其妙的错误出现在列表里。
    if let Some(e) = legacy_layout_error(&meta, dir) {
        return broken(e);
    }

    // 至少要有**一张**能查的表（词频表）。缺了也常见（拷贝中断、被误删），要单独说清楚。
    let missing: Vec<String> = meta
        .tables
        .iter()
        .filter(|t| !dir.join(&t.path).join(format!("{}.vfr", t.kind)).is_file())
        .map(|t| t.key())
        .collect();
    let error = if !missing.is_empty() {
        Some(format!(
            "缺 {} 的 .vfr，这张表有一部分查不了词",
            missing.join("、")
        ))
    } else if meta.tables.iter().all(|t| t.kind != "word") {
        Some("这张表里一张词频表都没有，查不了词".to_string())
    } else {
        None
    };

    let binding = binding_of(&meta, current);
    TableItem {
        name,
        path,
        meta: Some(meta),
        error,
        binding,
        active,
        in_library,
        origin,
    }
}

// ===========================================================================
// 绑定判定与词典链解析
// ===========================================================================

/// 认出「schema v3 之前的老布局」，返回一句可操作的说法。
///
/// 判据是**磁盘上真的有老布局的目录**，而不是只看 `schema_version`
/// （那只是个提示性字段，用户手改过 meta 也不能因此把一个能用的目录判死）：
///
/// * 有 `domains\` 目录 —— v2 的表组住在这儿；
/// * 或 `full\word`、`full\char` 这种**目录** —— v2 把 `<表组>/<类型>` 当路径用，
///   于是 `full/word.vfr` 变成了 `full/word/` 下面还有东西。
///
/// v3 的布局是 `<表组>/word.vfr`，两者一眼可分。
fn legacy_layout_error(meta: &Meta, dir: &Path) -> Option<String> {
    let has_domains_dir = dir.join("domains").is_dir();
    let nested = meta
        .tables
        .iter()
        .any(|t| !t.path.is_empty() && dir.join(&t.path).join(&t.kind).is_dir());
    if !has_domains_dir && !nested {
        return None;
    }
    Some(format!(
        "这是 schema v{} 的老布局产物（表按 full/word、domains/表组/word 这样的路径组织）。\
         现在每个**表组**各占一个目录、彼此平级，所以这份产物读不了，也无法就地迁移。\
         请用同一套语料库与词典重新统计一次。",
        meta.schema_version
    ))
}

/// 判断一张表记录的词典链对现在的词典库还成不成立。
///
/// * **一条可校验的记录都没有**（v1 老产物，或记录里全是没指纹的自由文本）→
///   [`Binding::Legacy`]。这**不是错误**：当年词典编在 exe 里，根本没有记录的必要。
///   注意判据是"有没有指纹"而不是"记录列表空不空" —— v1 的 `dict` 字段反序列化
///   出来是一条没有指纹的记录，列表非空，但它说明不了任何事。
/// * 每条记录都能按指纹或名字在当前词典里找到 → [`Binding::Ok`]
/// * 找到了但指纹不同 → [`Binding::Drifted`]（典型场景：用户改了那份词典）
/// * 找不到 → [`Binding::Missing`]（用户把它删了）
///
/// 只要有 Drifted 或 Missing 就属于"这张表的频次可能已经对不上分词口径"，
/// 界面必须显眼提示并给一键重算。**不能静默照用。**
pub fn binding_of(meta: &Meta, current: &[DictItem]) -> Binding {
    if !meta.tokenizer.has_verifiable_dict() {
        return Binding::Legacy;
    }
    let recorded = meta.tokenizer.resolved_dicts();

    let mut changed = Vec::new();
    let mut missing = Vec::new();
    for rec in &recorded {
        match find_dict(rec, current) {
            Some((item, exact)) => {
                if !exact {
                    changed.push(item.dict.name.clone());
                }
            }
            None => missing.push(rec.name.clone()),
        }
    }

    if !missing.is_empty() {
        Binding::Missing { missing }
    } else if !changed.is_empty() {
        Binding::Drifted { changed }
    } else {
        Binding::Ok
    }
}

/// 把一张表记录的词典链解析成**当前数据文件夹里**的实际路径。
///
/// 返回的 `paths` 按记录顺序排列，供 [`vocfreq_core::tokenize::Tokenizer::from_dicts`]
/// 重建分词器 —— 顺序不能乱，否则同名条目的覆盖结果会变。
///
/// 只要能按指纹匹配上就按指纹匹配：用户把词典改个名、挪个子目录，表照样能用。
pub fn resolve_chain(recorded: &[DictRef], current: &[DictItem]) -> Vec<PathBuf> {
    recorded
        .iter()
        .filter_map(|rec| find_dict(rec, current).map(|(item, _)| PathBuf::from(&item.dict.path)))
        .collect()
}

/// 在当前词典里找一条记录对应的词典。
///
/// 返回 `(命中项, 指纹是否精确一致)`。匹配顺序：**指纹 → 名字**。
///
/// 故意**不**回退到记录里的绝对路径：词典统一住在数据文件夹的 `dicts\` 里，
/// 追一个可能早已失效、也可能指向别处同名文件的旧路径，会让"绑定判定"和
/// "实际解析"给出互相矛盾的结论（一边说 Missing、一边又解析成功了）。
/// 用户把词典放在文件夹之外时，导入进来就是了。
fn find_dict<'a>(rec: &DictRef, current: &'a [DictItem]) -> Option<(&'a DictItem, bool)> {
    // 1) 指纹一致（最可靠）
    if rec.is_verifiable() {
        if let Some(it) = current
            .iter()
            .find(|it| it.usable() && it.dict.same_content(rec) == Some(true))
        {
            return Some((it, true));
        }
    }
    // 2) 名字一致（词典被改过 → 指纹不同，但仍是"同一份"，算 Drifted）
    current
        .iter()
        .find(|it| it.usable() && !it.dict.name.is_empty() && it.dict.name == rec.name)
        .map(|it| (it, false))
}

// ===========================================================================
// 来源标记
// ===========================================================================

/// 来源标记存在一个同名 sidecar 文件里（`<名字>.origin`）。
///
/// 为什么不塞进词典文件本身：那会在词典里加一行 `#` 注释，用户手改一次就可能
/// 弄丢；而且"来源"是**安装包与用户操作**的属性，不是词典内容的属性 ——
/// 内容改了指纹就该变，来源却不该因为改了一个字就变成 Imported。
fn origin_sidecar(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(".origin");
    PathBuf::from(s)
}

pub fn mark_origin(p: &Path, origin: Origin) {
    let name = match origin {
        Origin::Seeded => "seeded",
        Origin::Imported => "imported",
        Origin::Scanned => "scanned",
        Origin::Unknown => "unknown",
    };
    let _ = std::fs::write(origin_sidecar(p), name);
}

pub fn origin_of_file(p: &Path) -> Origin {
    match std::fs::read_to_string(origin_sidecar(p)) {
        Ok(s) => match s.trim() {
            "seeded" => Origin::Seeded,
            "imported" => Origin::Imported,
            "scanned" => Origin::Scanned,
            _ => Origin::Unknown,
        },
        Err(_) => Origin::Unknown,
    }
}

// ===========================================================================
// 表名
// ===========================================================================

/// 把用户给的表名洗成安全的目录名。
///
/// Windows 上还有一批保留字（`CON`/`PRN`/`AUX`/`NUL`/`COM1`…）不能当目录名，
/// 直接建会失败。这里统一加前缀避开，而不是让用户对着一个系统报错发呆。
pub fn sanitize_table_name(raw: &str) -> String {
    let mut s: String = raw
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() {
        s = "未命名".into();
    }
    let upper = s.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|r| upper == *r)
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        s = format!("_{s}");
    }
    // 目录名太长在某些文件系统上会失败，留点余量
    if s.chars().count() > 64 {
        s = s.chars().take(64).collect();
    }
    s
}

/// 一组词典按文件名排序后作为默认词典链。
///
/// 返回 `BTreeMap` 是为了顺序确定 —— 词典链的顺序会影响同名条目的覆盖结果，
/// 顺序飘了，同一批文件就会跑出不同的表。
pub fn default_chain(dicts: &[DictItem]) -> Vec<PathBuf> {
    let mut m: BTreeMap<&str, &DictItem> = BTreeMap::new();
    for it in dicts.iter().filter(|d| d.usable()) {
        m.insert(it.file_name.as_str(), it);
    }
    m.values().map(|it| PathBuf::from(&it.dict.path)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 落一个临时 `.dict` 文件，`Drop` 时删掉。
    ///
    /// 不复用 `vocfreq_core::dict::testutil`：那个模块是 `#[cfg(test)]` 的，
    /// 跨 crate 看不见（桌面端把 core 当外部 crate 编）。这里只需要"写个文件"。
    struct TempDict(PathBuf);

    impl TempDict {
        fn new(name: &str, body: &str) -> Self {
            use std::io::Write as _;
            let mut p = std::env::temp_dir();
            p.push(format!("vocfreq-lib-test-{}-{name}.dict", std::process::id()));
            let mut f = std::fs::File::create(&p).expect("建临时词典");
            f.write_all(body.as_bytes()).expect("写临时词典");
            f.flush().expect("刷临时词典");
            TempDict(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDict {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// 造一个数据文件夹，里面放指定的词典文件。
    fn lib_with_dicts(name: &str, dicts: &[(&str, &str)]) -> (PathBuf, Library, Vec<TempDict>) {
        let root = std::env::temp_dir().join(format!("vocfreq-lib-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lib = Library::new(&root);
        lib.ensure().unwrap();

        let mut keep = Vec::new();
        for (file, body) in dicts {
            let t = TempDict::new(&format!("{name}-{file}"), body);
            std::fs::copy(t.path(), lib.dict_path(file)).unwrap();
            keep.push(t);
        }
        (root, lib, keep)
    }

    #[test]
    fn list_dicts_reads_reports_and_never_skips_broken_files() {
        let (root, lib, _keep) = lib_with_dicts(
            "listing",
            &[
                ("好.dict", "甲 10\n乙 20\n"),
                ("坏.dict", "丙 不是数字\n"),
            ],
        );
        let dicts = lib.list_dicts();
        assert_eq!(dicts.len(), 2, "坏文件也必须列出来，用户得看见它坏了");

        let good = dicts.iter().find(|d| d.file_name == "好.dict").unwrap();
        assert!(good.usable());
        assert_eq!(good.dict.entries, 2);
        assert!(good.dict.is_verifiable());

        let bad = dicts.iter().find(|d| d.file_name == "坏.dict").unwrap();
        assert!(!bad.usable(), "坏文件应带 error");
        assert!(bad.error.as_deref().unwrap().contains("第 1 行"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn default_chain_is_ordered_by_file_name() {
        let (root, lib, _keep) = lib_with_dicts(
            "order",
            &[("b.dict", "甲 1\n"), ("a.dict", "乙 1\n")],
        );
        let chain = default_chain(&lib.list_dicts());
        let names: Vec<String> = chain
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.dict", "b.dict"], "顺序必须确定，否则产物会飘");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn binding_ok_when_hashes_match() {
        let (root, lib, _keep) = lib_with_dicts("bind-ok", &[("主.dict", "甲 10\n")]);
        let current = lib.list_dicts();
        let rec = vec![current[0].dict.clone()];
        // 造一份 meta：只用到 tokenizer.dicts
        let meta = meta_with_dicts(&rec);
        assert_eq!(binding_of(&meta, &current), Binding::Ok);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn binding_legacy_for_v1_products_without_fingerprints() {
        let (root, lib, _keep) = lib_with_dicts("bind-legacy", &[("主.dict", "甲 10\n")]);
        let current = lib.list_dicts();
        // v1：dict 是一句自由文本，没有指纹
        let meta: Meta = serde_json::from_str(&format!(
            r#"{{"schema_version":1,"generated_at":"","tool_version":"","corpus_root":"","elapsed_ms":0,
                 "tokenizer":{{"engine":"jieba-rs","version":"0.11","hmm":false,
                   "dict":"builtin(jieba dict.txt, 349046 entries)","user_dict":null,
                   "min_len":1,"max_len":64,"keep_latin":true,"keep_digit":false,"skip_single_char":false}},
                 "totals":{{"files":0,"bytes":0,"lines":0,"paras":0,"tokens":0,"bad_lines":0}},
                 "domains":[],"tables":[],"tier_names":[]}}"#
        ))
        .expect("v1 的 meta.json 必须还能反序列化");
        // 无从判断 → Legacy，而不是报"不一致"
        assert_eq!(binding_of(&meta, &current), Binding::Legacy);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn binding_drifted_when_same_name_but_different_content() {
        let (root, lib, _keep) = lib_with_dicts("bind-drift", &[("主.dict", "甲 10\n")]);
        let current = lib.list_dicts();
        // 记录里是"主"，但当时的内容不同（换了个指纹）
        let mut rec = current[0].dict.clone();
        rec.sha256 = "0".repeat(64);
        let meta = meta_with_dicts(&[rec]);
        match binding_of(&meta, &current) {
            Binding::Drifted { changed } => assert_eq!(changed, vec!["主".to_string()]),
            other => panic!("同名不同内容应判为 Drifted，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn binding_missing_when_dict_was_deleted() {
        let (root, lib, _keep) = lib_with_dicts("bind-missing", &[("主.dict", "甲 10\n")]);
        let current = lib.list_dicts();
        let mut rec = current[0].dict.clone();
        rec.name = "已经删掉的那份".into();
        rec.sha256 = "1".repeat(64);
        let meta = meta_with_dicts(&[rec]);
        match binding_of(&meta, &current) {
            Binding::Missing { missing } => assert_eq!(missing, vec!["已经删掉的那份".to_string()]),
            other => panic!("删掉的词典应判为 Missing，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_chain_matches_by_hash_after_rename() {
        let (root, lib, _keep) = lib_with_dicts("rename", &[("旧名.dict", "甲 10\n")]);
        let current = lib.list_dicts();
        // 记录里叫"旧名"，指纹一致；用户后来把文件改成了"新名.dict"
        std::fs::rename(lib.dict_path("旧名.dict"), lib.dict_path("新名.dict")).unwrap();
        let current2 = lib.list_dicts();
        let chain = resolve_chain(&[current[0].dict.clone()], &current2);
        assert_eq!(chain.len(), 1, "改名不该让表失效");
        assert!(chain[0].ends_with("新名.dict"), "应解析到改名后的文件");
        // 而且判为 Ok（指纹一致），不是 Drifted
        assert_eq!(binding_of(&meta_with_dicts(&[current[0].dict.clone()]), &current2), Binding::Ok);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_never_overwrites_and_rejects_bad_files() {
        let (root, lib, _keep) = lib_with_dicts("import", &[("我的.dict", "甲 10\n")]);

        // 源文件必须**真的叫「我的.dict」**才能测到重名分支
        let src_dir = std::env::temp_dir().join(format!("vocfreq-imp-{}-src", std::process::id()));
        let _ = std::fs::remove_dir_all(&src_dir);
        std::fs::create_dir_all(&src_dir).unwrap();
        let same = src_dir.join("我的.dict");
        std::fs::write(&same, "乙 20\n").unwrap();

        let landed = lib.import_dict(&same).unwrap();
        assert_eq!(landed, "我的 (2).dict", "重名必须加后缀");
        assert_eq!(
            std::fs::read_to_string(lib.dict_path("我的.dict")).unwrap(),
            "甲 10\n",
            "原有那份的内容不能被改动"
        );
        assert_eq!(
            std::fs::read_to_string(lib.dict_path("我的 (2).dict")).unwrap(),
            "乙 20\n"
        );

        // 坏文件不该被导进来占地
        let bad = src_dir.join("坏.dict");
        std::fs::write(&bad, "甲 不是数字\n").unwrap();
        assert!(lib.import_dict(&bad).is_err());
        assert!(!lib.dict_path("坏.dict").exists(), "坏文件不能落地");

        // 非 .dict 扩展名也拒绝（否则 README 之类也会被当词典）
        let wrong = src_dir.join("其实不是词典.txt");
        std::fs::write(&wrong, "甲 10\n").unwrap();
        assert!(lib.import_dict(&wrong).is_err());

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_dict_and_table_remove_files() {
        let (root, lib, _keep) = lib_with_dicts("delete", &[("甲.dict", "甲 10\n")]);
        assert!(lib.dict_path("甲.dict").exists());
        lib.delete_dict("甲.dict").unwrap();
        assert!(!lib.dict_path("甲.dict").exists());
        assert!(lib.delete_dict("甲.dict").is_err(), "删第二次应报错而不是静默成功");

        let t = lib.table_dir("某表");
        std::fs::create_dir_all(&t).unwrap();
        std::fs::write(t.join("meta.json"), "{}").unwrap();
        lib.delete_table("某表").unwrap();
        assert!(!t.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn table_name_sanitizing_covers_windows_reserved_names() {
        assert_eq!(sanitize_table_name("a/b:c"), "a_b_c");
        assert_eq!(sanitize_table_name("  尾部点...  "), "尾部点");
        assert_eq!(sanitize_table_name(""), "未命名");
        // Windows 保留设备名不能当目录名
        assert_eq!(sanitize_table_name("CON"), "_CON");
        assert_eq!(sanitize_table_name("com1"), "_com1");
        assert_eq!(sanitize_table_name("CONS"), "CONS", "不是保留名的不要误伤");
        assert_eq!(sanitize_table_name(&"长".repeat(100)).chars().count(), 64);
    }

    #[test]
    fn origin_is_recorded_and_read_back() {
        let (root, lib, _keep) = lib_with_dicts("origin", &[("甲.dict", "甲 10\n")]);
        mark_origin(&lib.dict_path("甲.dict"), Origin::Seeded);
        let it = lib.list_dicts().into_iter().find(|d| d.file_name == "甲.dict").unwrap();
        assert_eq!(it.origin, Origin::Seeded);
        // 没有 sidecar 就是 Unknown，不是猜
        std::fs::write(lib.dict_path("乙.dict"), "乙 10\n").unwrap();
        let it2 = lib.list_dicts().into_iter().find(|d| d.file_name == "乙.dict").unwrap();
        assert_eq!(it2.origin, Origin::Unknown);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_tables_flags_broken_and_missing_tables() {
        let (root, lib, _keep) = lib_with_dicts("tables", &[("甲.dict", "甲 10\n")]);

        // 一张结构正常的表（v1 meta，因此 binding=Legacy；这里只验证"能被列出来"
        // 与"不报结构性错误"）。布局必须是 v3 的：`<表组>/<类型>.vfr`。
        let ok = lib.table_dir("完整表");
        std::fs::create_dir_all(ok.join("full")).unwrap();
        std::fs::write(ok.join("meta.json"), v3_meta_json(&["full"])).unwrap();
        std::fs::write(ok.join("full").join("word.vfr"), b"x").unwrap();
        std::fs::write(ok.join("full").join("char.vfr"), b"x").unwrap();

        // 一张 meta 在、但 .vfr 缺了的表
        let half = lib.table_dir("半截表");
        std::fs::create_dir_all(half.join("full")).unwrap();
        std::fs::write(half.join("meta.json"), v3_meta_json(&["full"])).unwrap();
        std::fs::write(half.join("full").join("word.vfr"), b"x").unwrap();
        // 缺 char.vfr

        // 一张 meta.json 坏掉的表
        let broken = lib.table_dir("坏表");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("meta.json"), "{ 不是 json").unwrap();

        // **老布局**（schema v2：多一层 domains\ 目录）
        let legacy = lib.table_dir("老布局表");
        std::fs::create_dir_all(legacy.join("domains").join("news")).unwrap();
        std::fs::write(legacy.join("meta.json"), v3_meta_json(&["full"])).unwrap();

        let tables = lib.list_tables(&lib.list_dicts(), Some("完整表"));
        assert_eq!(tables.len(), 4, "四种情况都要列出来");

        let t = tables.iter().find(|t| t.name == "完整表").unwrap();
        assert!(t.error.is_none(), "结构正常的表不该被标错：{:?}", t.error);
        assert!(t.active, "应认出激活的那张");
        assert_eq!(t.binding, Binding::Legacy, "v1 meta 无从校验词典");

        let t = tables.iter().find(|t| t.name == "半截表").unwrap();
        assert!(
            t.error.as_deref().unwrap().contains("full/char"),
            "缺文件要点名是哪一张：{:?}",
            t.error
        );

        let t = tables.iter().find(|t| t.name == "坏表").unwrap();
        assert!(t.error.as_deref().unwrap().contains("解析失败"));

        // 老布局必须给出一句**可执行**的话，而不是「文件不存在」
        let t = tables.iter().find(|t| t.name == "老布局表").unwrap();
        let msg = t.error.as_deref().expect("老布局必须被标出来");
        assert!(msg.contains("老布局"), "{msg}");
        assert!(msg.contains("重新统计"), "要告诉用户怎么办：{msg}");

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---------------------------------------------------------------- 测试辅助

    /// 一份 **v3 新布局**的 meta：每个表组各一条 word/char 记录。
    fn v3_meta_json(scopes: &[&str]) -> String {
        let pct = [0.0026, 0.026, 0.132, 0.526, 1.32, 3.95];
        let tiers: Vec<serde_json::Value> = pct
            .iter()
            .enumerate()
            .map(|(i, _)| {
                serde_json::json!({
                    "name": vocfreq_core::rank::TIER_NAMES[i],
                    "max_rank": (i as u64 + 1) * 100,
                })
            })
            .chain(std::iter::once(serde_json::json!({
                "name": vocfreq_core::rank::TIER_NAMES[6],
                "max_rank": u64::MAX,
            })))
            .collect();
        let mut tables: Vec<serde_json::Value> = Vec::new();
        for s in scopes {
            for kind in ["word", "char"] {
                tables.push(serde_json::json!({
                    "path": s, "kind": kind, "entries": 1, "total_tokens": 1, "vfr_bytes": 1,
                    "tiers": tiers, "tier_stats": [],
                    "min_count": 1, "tier_pct": pct, "source_tables": [],
                }));
            }
        }
        serde_json::json!({
            "schema_version": 3, "generated_at": "", "tool_version": "",
            "corpus_root": "", "elapsed_ms": 0,
            "tokenizer": {
                "engine": "jieba-rs", "version": "0.11", "hmm": false,
                "dict": "builtin(jieba dict.txt, 349046 entries)", "user_dict": null,
                "min_len": 1, "max_len": 64,
                "keep_latin": true, "keep_digit": false, "skip_single_char": false,
            },
            "totals": {"files":0,"bytes":0,"lines":0,"paras":0,"tokens":0,"bad_lines":0},
            "domains": [], "tables": tables, "tier_names": vocfreq_core::rank::TIER_NAMES,
            "tier_keys": vocfreq_core::rank::TIER_KEYS,
        })
        .to_string()
    }

    fn meta_with_dicts(dicts: &[DictRef]) -> Meta {
        let tok = serde_json::json!({
            "engine": "jieba-rs", "version": "0.11", "hmm": false,
            "dicts": dicts,
            "min_len": 1, "max_len": 64,
            "keep_latin": true, "keep_digit": false, "skip_single_char": false,
        });
        let v = serde_json::json!({
            "schema_version": 2, "generated_at": "", "tool_version": "",
            "corpus_root": "", "elapsed_ms": 0, "tokenizer": tok,
            "totals": {"files":0,"bytes":0,"lines":0,"paras":0,"tokens":0,"bad_lines":0},
            "domains": [], "tables": [], "tier_names": []
        });
        serde_json::from_value(v).expect("造 meta")
    }
}
