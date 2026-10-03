//! **跨产物合流**：把 N 份独立产物并成一份。
//!
//! ## 为什么需要它
//!
//! 全量语料（实测 140 GB）放不下同一块磁盘，只能**一个表组一个表组**地扫。而 `scan`
//! 的产物是「一份产物 = `meta.json` + 若干平级表组目录」，每扫一次都会整体覆盖
//! `meta.json` —— 分次扫出来的几份产物谁也不知道对方存在。于是需要一步显式的合流：
//!
//! ```text
//! vocfreq scan --corpus 语料\news --out 分片\A        # 一份产物，只有 news 表组
//! vocfreq scan --corpus 语料\wiki --out 分片\B        # 另一份产物，只有 wiki 表组
//! vocfreq merge --from-data 分片\A --from-data 分片\B --scope 全库 --out 词频表\主表
//! vocfreq compose --from-data 词频表\主表 --source news/word --source wiki/word --scope full
//! ```
//!
//! 合流出来的仍是一份**标准 v3 单产物**（`meta.json` + 平级表组目录），对桌面端
//! 完全透明 —— 它没有"这是一份合流产物"这种概念，也不需要知道。
//!
//! ## 硬门槛：词典链与分词口径必须完全一致
//!
//! 合流只是把各产物的表组目录摆在一起、把 `tables[]` 与 `domains[]` 并起来，
//! **不做任何求解或取舍**。它成立的前提是各源表用的是同一套词典、同一套分词口径
//! （`docs/DATA_LAYOUT.md` §三.3）—— 否则合流出来的产物里，同一个词在不同表组里
//! 的频次来自不同的切分规则，排名自相矛盾，而且**没人看得出来**。
//!
//! 所以这里逐份校验 [`TokenizerMeta`] 的每一个字段与整条词典链的指纹：
//! 任何一处不一致都**拒绝合流**，并指出是哪一份产物、哪个字段不同
//! （见 [`crate::compose::check_mergeable`]）。宁可拒绝，也不静默合并。
//!
//! ## 与 `compose` 的分工
//!
//! * `merge`：**并列**多份产物的表组（不改变任何频次）；
//! * [`crate::compose`]：单份产物**内部**把若干表组**相加**成一张新表。
//!
//! 合流之后用 `compose` 把各表组相加，就得到 `full` —— 数学上与"一次性全量扫描"
//! 逐条相等（`compose.rs` 顶上那段推导）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::artifact::{self, Meta, SCHEMA_VERSION, SourceScope, TableMeta};
use crate::compose::{check_mergeable, describe_product};
use crate::query::Dataset;
use crate::rank;
use crate::{Error, Result, VERSION};
/// 一次合流请求。
#[derive(Debug, Clone)]
pub struct MergeSpec {
    /// 源产物目录，至少两份
    pub from: Vec<PathBuf>,
    /// 目标父目录。值为 `None` 时它同时就是目标产物目录
    pub out: PathBuf,
    /// 目标产物目录名
    pub scope: Option<String>,
}

/// 合流结果，供 CLI 打印。字段名与 [`crate::compose::ComposedTable`] 同一风格。
#[derive(Debug, Clone, Serialize)]
pub struct MergedProduct {
    /// 目标产物目录
    pub out: String,
    /// 合起来的源产物目录
    pub from: Vec<String>,
    /// 目标产物里的表组（`full` 优先，其余按名字）
    pub scopes: Vec<String>,
    pub tables: u64,
    pub entries: u64,
    pub total_tokens: u64,
    pub bytes: u64,
    pub elapsed_ms: u64,
}

/// 把 N 份源产物合流成一份新产物。
///
/// 全部校验都在**动手写盘之前**做完（词典链、分词口径、表组名冲突、目标目录是否
/// 可写、源目录之间是否互相嵌套），随后先在临时目录里铺完整份产物、最后一步整体改名
/// 到位。因此任何一步失败都不会留下半截产物 —— 目标目录要么不存在，要么是完整的。
pub fn merge(spec: &MergeSpec) -> Result<MergedProduct> {
    let t0 = Instant::now();
    if spec.from.len() < 2 {
        return Err(Error::Other(format!(
            "合流至少要两份源产物，现在只给了 {} 份。\
             单份产物内部相加用 `compose`（它才是把若干张表加起来的那个命令）。",
            spec.from.len()
        )));
    }

    // 1) 打开每一份源产物（`Dataset::open` 已经校验过 schema 版本与每张表是否都在）
    let mut opened: Vec<(PathBuf, Dataset)> = Vec::with_capacity(spec.from.len());
    for p in &spec.from {
        let ds = Dataset::open(p)
            .map_err(|e| Error::Format(format!("打不开源产物 {}：{e}", p.display())))?;
        opened.push((p.clone(), ds));
    }
    let products: Vec<&Dataset> = opened.iter().map(|(_, d)| d).collect();
    let labels: Vec<String> = opened
        .iter()
        .map(|(p, d)| describe_product(&p.display().to_string(), d))
        .collect();

    // 2) 目标位置先定下来、先判掉"不能写"，再去动磁盘
    let final_dir = match &spec.scope {
        Some(s) => {
            let scope = artifact::sanitize_scope(s);
            if scope.is_empty() {
                return Err(Error::Other("目标产物名不能为空".into()));
            }
            spec.out.join(scope)
        }
        None => spec.out.clone(),
    };
    if final_dir.exists() {
        // 与「永不覆盖」同一条原则（`library::import_dict` / `compose` 都是这个规矩）。
        // 合流可能拷几百 MB，悄悄盖掉用户已有的产物代价太大。
        return Err(Error::Other(format!(
            "{} 已经存在了。换一个目标名，或先把旧的那份删掉/改名。",
            final_dir.display()
        )));
    }
    for (p, _) in &opened {
        let src = canonical_or_self(p);
        if canonical_or_self(&final_dir) == src {
            return Err(Error::Other(format!(
                "目标产物目录 {} 就是源产物 {} —— 合流必须写到别处，\
                 否则会在读源表的同时覆盖它。",
                final_dir.display(),
                p.display()
            )));
        }
        // 目标在源产物**里面**同样危险（临时目录与被拷的表混在一起）
        if canonical_or_self(&final_dir).starts_with(&src) {
            return Err(Error::Other(format!(
                "目标产物目录 {} 在源产物 {} 内部。请把目标放到源产物外面。",
                final_dir.display(),
                p.display()
            )));
        }
    }
    // 源产物之间互相嵌套时，平铺出来的表组目录会互相盖，直接拒绝
    for (i, (a, _)) in opened.iter().enumerate() {
        for (b, _) in opened.iter().skip(i + 1) {
            let (ca, cb) = (canonical_or_self(a), canonical_or_self(b));
            if ca == cb {
                return Err(Error::Other(format!(
                    "{} 被当成了两份源产物，请去掉重复的那一份",
                    a.display()
                )));
            }
            if ca.starts_with(&cb) || cb.starts_with(&ca) {
                return Err(Error::Other(format!(
                    "源产物 {} 与 {} 互相嵌套，合流时表组目录会互相覆盖。\
                     请先把它们挪成互不相邻的目录。",
                    a.display(),
                    b.display()
                )));
            }
        }
    }

    // 3) **硬门槛**：词典链与分词口径必须完全一致；表组名不许撞车
    check_mergeable(&products, &labels)?;
    let merged_tables = merge_table_refs(&products, &labels)?;

    // 4) 先在临时目录里铺完整份产物，成功后再整体改名到位
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stage = stage_dir(&final_dir);
    let _ = std::fs::remove_dir_all(&stage);
    let built = (|| -> Result<(u64, u64, u64)> {
        std::fs::create_dir_all(&stage)?;
        let (entries, tokens, bytes) = copy_tables(&merged_tables, &stage)?;
        artifact::write_meta(
            &stage.join("meta.json"),
            &build_meta(&opened, &merged_tables),
        )?;
        Ok((entries, tokens, bytes))
    })();
    let (entries, total_tokens, bytes) = match built {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
    };
    if let Err(e) = std::fs::rename(&stage, &final_dir) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(Error::Io(e));
    }

    // 自检：产物必须真的能打开。读不了就说明铺出来的东西是坏的，直接拆掉，
    // 绝不把一份打不开的目录留在用户的数据文件夹里。顺带从这份已打开的实例里取
    // `scopes` —— 再开一次等于白白多 mmap 一遍全部 `.vfr`。
    let scopes = match Dataset::open(&final_dir) {
        Ok(ds) => ds.meta.scopes(),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&final_dir);
            return Err(Error::Format(format!(
                "合流产物自检失败（{}），已删除：{e}",
                final_dir.display()
            )));
        }
    };

    Ok(MergedProduct {
        out: final_dir.display().to_string(),
        from: spec.from.iter().map(|p| p.display().to_string()).collect(),
        scopes,
        tables: merged_tables.len() as u64,
        entries,
        total_tokens,
        bytes,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

/// 一份源产物里的一张表在目标产物里的落点。
struct MergedTable {
    /// 目标产物里的表组目录名（= 源产物的表组名，合流不重命名）
    scope: String,
    kind: String,
    /// 这张表的文件在哪个源产物目录里。合流**不重写任何表**，所以照搬即可。
    source_root: PathBuf,
    meta: TableMeta,
}

/// 把各源产物的表平铺成一张清单：**表组名不许跨产物重复**。
///
/// 表组就是磁盘上的目录名，两份产物都有 `news` 的话，谁盖谁都是静默丢数据，
/// 所以宁可拒绝。同一个表组内部的重复登记（同一份产物里 `news/word` 出现两次）
/// 也一并拒绝 —— 那是产物本身坏了。
fn merge_table_refs(products: &[&Dataset], labels: &[String]) -> Result<Vec<MergedTable>> {
    let mut out: Vec<MergedTable> = Vec::new();
    // 每份产物内部：表组 → 已登记的类型。用来抓"同一份产物里登记了两次"。
    let mut kinds_within: BTreeMap<usize, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    // 全局：表组 → 拥有它的产物下标。用来抓跨产物撞名。
    let mut owner: BTreeMap<String, Vec<usize>> = BTreeMap::new();

    for (pi, ds) in products.iter().enumerate() {
        for t in &ds.meta.tables {
            let within = kinds_within.entry(pi).or_default();
            let kinds = within.entry(t.path.clone()).or_default();
            if kinds.iter().any(|k| k == &t.kind) {
                return Err(Error::Other(format!(
                    "表组「{}」的 {} 表在同一份产物里登记了两次（{}）。\
                     这份产物本身是坏的，先用 `vocfreq info` 看一眼再合流。",
                    t.path, t.kind, labels[pi]
                )));
            }
            kinds.push(t.kind.clone());

            let owners = owner.entry(t.path.clone()).or_default();
            if !owners.contains(&pi) {
                owners.push(pi);
            }
            out.push(MergedTable {
                scope: t.path.clone(),
                kind: t.kind.clone(),
                source_root: products[pi].root.clone(),
                meta: t.clone(),
            });
        }
    }

    // 跨产物撞名：把撞了的所有产物一次列出来，别让用户改一个又撞一个
    let mut clashes: Vec<String> = Vec::new();
    for (scope, who) in &owner {
        if who.len() > 1 {
            clashes.push(format!(
                "「{scope}」出现在 {}",
                who.iter()
                    .map(|i| labels[*i].as_str())
                    .collect::<Vec<_>>()
                    .join(" 与 ")
            ));
        }
    }
    if !clashes.is_empty() {
        return Err(Error::Other(format!(
            "表组名撞车，拒绝合流（两个同名表组只能留一个，静默丢数据比报错糟得多）：\n  {}\n\
             换一份源产物，或先在其中一份里把表组改个名。",
            clashes.join("\n  ")
        )));
    }

    out.sort_by(|a, b| (&a.scope, &a.kind).cmp(&(&b.scope, &b.kind)));
    Ok(out)
}

/// 把清单里每一张 `.vfr` 从它的源产物拷进目标产物。
///
/// 返回 `(条目数合计, token 合计, 字节合计)`。`.tsv` 是可选的可读产物，
/// 源里没有就不拷 —— 所有读表路径都只认 `.vfr`。
fn copy_tables(tables: &[MergedTable], target: &Path) -> Result<(u64, u64, u64)> {
    let mut entries = 0u64;
    let mut tokens = 0u64;
    let mut bytes = 0u64;
    for t in tables {
        let src = t.source_root.join(&t.scope).join(format!("{}.vfr", t.kind));
        let dst_dir = target.join(&t.scope);
        std::fs::create_dir_all(&dst_dir)?;
        let dst = dst_dir.join(format!("{}.vfr", t.kind));
        std::fs::copy(&src, &dst).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("拷贝 {} 失败: {e}", src.display()),
            ))
        })?;
        let tsv = t.source_root.join(&t.scope).join(format!("{}.tsv", t.kind));
        if tsv.exists() {
            let _ = std::fs::copy(&tsv, dst_dir.join(format!("{}.tsv", t.kind)));
        }
        entries += t.meta.entries;
        tokens += t.meta.total_tokens;
        bytes += std::fs::metadata(&dst)
            .map(|m| m.len())
            .unwrap_or(t.meta.vfr_bytes);
    }
    Ok((entries, tokens, bytes))
}

/// 目标产物的 `meta.json`。
///
/// * `tokenizer` 直接沿用第一份源产物的 —— 全部源产物已被校验为**完全一致**；
/// * `tables[]` 并起来，各表的 `tiers` / `tier_stats` 原样保留（它们描述的是
///   源表自己的条目数与分档，合流没有动这些表，重算只会把信息改坏）；
/// * `domains[]`（语料切片清单，描述的是**语料**不是表）按名字并起来、计数相加；
/// * `totals` 相加；
/// * `corpus_root` 只在各源完全相同时沿用，否则换成一句中性描述 ——
///   合流产物本来就可能对应好几块语料，硬抄第一份的路径是假信息。
fn build_meta(opened: &[(PathBuf, Dataset)], tables: &[MergedTable]) -> Meta {
    let first = &opened[0].1.meta;

    let mut domains: Vec<SourceScope> = Vec::new();
    for (_, ds) in opened {
        for d in &ds.meta.domains {
            match domains.iter_mut().find(|x| x.name == d.name) {
                Some(x) => {
                    x.files += d.files;
                    x.bytes += d.bytes;
                }
                None => domains.push(d.clone()),
            }
        }
    }
    domains.sort_by(|a, b| a.name.cmp(&b.name));

    let mut totals = crate::Totals::default();
    for (_, ds) in opened {
        totals.add(&ds.meta.totals);
    }

    let same_corpus = opened
        .iter()
        .all(|(_, d)| d.meta.corpus_root == first.corpus_root);

    Meta {
        schema_version: SCHEMA_VERSION,
        generated_at: artifact::iso8601_now(),
        tool_version: VERSION.to_string(),
        corpus_root: if same_corpus {
            first.corpus_root.clone()
        } else {
            let mut scopes: Vec<&str> = tables.iter().map(|t| t.scope.as_str()).collect();
            scopes.sort_unstable();
            scopes.dedup();
            format!(
                "合流产物：{} 份产物 / {} 个表组 / {} 个语料切片",
                opened.len(),
                scopes.len(),
                domains.len()
            )
        },
        elapsed_ms: 0,
        tokenizer: first.tokenizer.clone(),
        totals,
        domains,
        tables: tables.iter().map(|t| t.meta.clone()).collect(),
        tier_names: rank::TIER_NAMES.iter().map(|s| s.to_string()).collect(),
        tier_keys: rank::TIER_KEYS.iter().map(|s| s.to_string()).collect(),
    }
}

/// 临时目录的名字：与目标同父目录，保证最后的 `rename` 是同一个文件系统上的原子操作。
fn stage_dir(final_dir: &Path) -> PathBuf {
    let base = final_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "merged".into());
    final_dir.with_file_name(format!(".{base}.merging-{}", std::process::id()))
}

fn canonical_or_self(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}
