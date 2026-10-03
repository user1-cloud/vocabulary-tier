//! `vocfreq` 命令行的单元测试。
//!
//! 单独成文件，而不是内联在 `main.rs` 里：内联会让 `mod tests` 之后还有函数
//! （`clippy::items_after_test_module`），而 `main.rs` 里 `mod` 必须写在所有
//! 条目之前。放子模块里正合适 —— `use super::*` 照样能拿到私有的
//! `resolve_one` / `chain_from_meta`。

use super::*;
use vocfreq_core::dict::DictRef;

/// 造一个临时目录。
fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("vocfreq-cli-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// 往 `dir` 里写一份词典，返回它的身份（含真实 sha256）。
fn write_dict(dir: &Path, file_name: &str, body: &str) -> DictRef {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(file_name);
    std::fs::write(&p, body).unwrap();
    dict::read_dict(&p).unwrap().dict
}

#[test]
fn resolve_one_prefers_the_recorded_path_when_it_still_exists() {
    let root = tmp("recorded-path");
    let real = write_dict(&root, "别处.dict", "甲 10\n");
    // 记录里有个还活着的绝对路径 → 直接用它，不去翻 dicts\
    let found = resolve_one(&real, &None).expect("应命中记录里的路径");
    assert_eq!(found, PathBuf::from(&real.path));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_one_uses_the_data_folder_convention_when_path_is_blank() {
    // **这是"装完之后那张预置表能不能用"的关键路径。**
    //
    // 出厂预置表在 meta.json 里刻意不记 path（记了会把构建机的目录结构泄露出厂产物，
    // 而且装到用户机器上那个路径根本不存在）。词典在 <数据文件夹>\dicts\，
    // 表在 <数据文件夹>\tables\<名字>\，所以必须能靠"dicts\ 与 tables\ 同级"
    // 这条约定 + 指纹把它找回来。没有这条，用户装完就会发现 segment 直接罢工。
    let root = tmp("convention");
    let data = root.join("data");
    let recorded = write_dict(&data.join("dicts"), "预制词典.dict", "甲 10\n乙 20\n");

    // 模拟出厂产物：path 清空
    let mut shipped = recorded.clone();
    shipped.path = String::new();

    let table_dir = data.join("tables").join("预制表");
    std::fs::create_dir_all(&table_dir).unwrap();
    let dicts_dir = Some(data.join("dicts"));

    let found = resolve_one(&shipped, &dicts_dir).expect("应靠 dicts\\ 约定找回来");
    assert_eq!(
        found.file_name().unwrap().to_string_lossy(),
        "预制词典.dict"
    );

    // 整条链也要能重建出来
    let meta = meta_json_with(&shipped);
    let chain = chain_from_meta(&table_dir, &meta);
    assert_eq!(chain.len(), 1, "chain_from_meta 必须能把这条链补回来");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_one_matches_by_hash_after_the_dict_is_renamed() {
    // 用户把词典改个名（内容没变）不该让表失效
    let root = tmp("renamed");
    let data = root.join("data");
    let recorded = write_dict(&data.join("dicts"), "原名.dict", "甲 10\n");
    std::fs::rename(
        data.join("dicts").join("原名.dict"),
        data.join("dicts").join("新名.dict"),
    )
    .unwrap();

    let found = resolve_one(&recorded, &Some(data.join("dicts"))).expect("指纹应认得出改名");
    assert_eq!(found.file_name().unwrap().to_string_lossy(), "新名.dict");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_one_falls_back_to_name_when_the_content_changed() {
    // 内容改了 → 指纹对不上，但仍是"同一份"，按名字找回来（调用方会警告漂移）
    let root = tmp("drifted");
    let data = root.join("data");
    let recorded = write_dict(&data.join("dicts"), "主词典.dict", "甲 10\n");
    std::fs::write(data.join("dicts").join("主词典.dict"), "甲 999\n").unwrap();

    let found = resolve_one(&recorded, &Some(data.join("dicts"))).expect("按名字兜底");
    assert!(found.ends_with("主词典.dict"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_one_returns_none_when_nothing_matches() {
    let root = tmp("nothing");
    let data = root.join("data");
    std::fs::create_dir_all(data.join("dicts")).unwrap();
    let mut rec = DictRef::legacy("某份早就不在的词典");
    rec.name = "查无此库".into();
    assert!(resolve_one(&rec, &Some(data.join("dicts"))).is_none());
    // 连 dicts 目录都没有时也不能 panic
    assert!(resolve_one(&rec, &None).is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// 造一份只用到 `tokenizer.dicts` 的 meta，供 `chain_from_meta` 测试。
fn meta_json_with(dicts: &DictRef) -> vocfreq_core::artifact::TokenizerMeta {
    let v = serde_json::json!({
        "engine": "jieba-rs", "version": "0.11", "hmm": false,
        "dicts": [dicts],
        "min_len": 1, "max_len": 64,
        "keep_latin": true, "keep_digit": false, "skip_single_char": false,
    });
    serde_json::from_value(v).expect("造 tokenizer meta")
}

// ---------------------------------------------------------------- 扫描 → 相加 全链路

/// 造一个两表组的小语料库。
fn write_corpus(root: &Path) {
    for (domain, lines) in [
        (
            "news",
            vec![
                r#"{"段落":[{"内容":"中国 人工智能 中国"}]}"#,
                r#"{"段落":[{"内容":"人工智能 发展 中国"}]}"#,
            ],
        ),
        (
            "wiki",
            vec![
                r#"{"段落":[{"内容":"宇宙 中国 宇宙"}]}"#,
                r#"{"段落":[{"内容":"文学 中国 宇宙"}]}"#,
            ],
        ),
    ] {
        let dir = root.join(domain);
        std::fs::create_dir_all(&dir).unwrap();
        let body = lines.join("\n");
        std::fs::write(dir.join("part.jsonl"), body).unwrap();
    }
}

/// 一份最小词典：让上面那几句话的词能整词切出来（不然全是单字，词频表没什么可比的）。
fn write_fixture_dict(path: &Path) {
    std::fs::write(
        path,
        "中国 1000000 n\n人工智能 1000000 n\n宇宙 1000000 n\n文学 1000000 n\n发展 1000000 v\n",
    )
    .unwrap();
}

/// 全链路：`scan` 写出扁平表组布局 → `compose` 把两个表组相加 → 与 `full` 逐条一致。
///
/// 这条测试覆盖的是本次改动最容易出错的三个接缝：
/// 1. 磁盘布局是不是「每个表组一个目录」（不是从前的 `full/` + `domains/`）；
/// 2. `meta.tables[].path` 是不是**纯表组名**；
/// 3. 相加是不是真的等于全量扫描（而这次是"扫描产出的表相加"，不是手造的数据）。
///
/// ⚠ `full` 现在是 [`ScanConfig::write_full`] 的显式产物（默认不产，见
/// `scan_produces_no_full_by_default`）。这里显式打开它，因为**它正是本测试的
/// 对照基准** —— 拿"表组相加"去和"一次性全量扫描的 full"逐条比对。
#[test]
fn scan_writes_flat_scopes_and_compose_matches_full() {
    let root = tmp("scan-compose");
    let corpus = root.join("corpus");
    let out = root.join("data");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus(&corpus);
    let dict_path = root.join("fixture.dict");
    write_fixture_dict(&dict_path);

    let mut cfg = ScanConfig::new(&corpus, &out);
    cfg.dicts = vec![dict_path.clone()];
    cfg.write_tsv = true;
    // 只有"要一份全量对照基准"时才需要它
    cfg.write_full = true;
    let meta = scan::scan(&cfg, &|_| {}).expect("扫描应当成功");

    // 布局：`<out>/<表组>/{word,char}.vfr`，`full` 只是其中一个
    for scope in ["full", "news", "wiki"] {
        assert!(
            out.join(scope).join("word.vfr").is_file(),
            "缺少 {scope}/word.vfr"
        );
        assert!(
            out.join(scope).join("char.vfr").is_file(),
            "缺少 {scope}/char.vfr"
        );
    }
    assert!(!out.join("domains").exists(), "不该再有 domains/ 这一层");
    assert!(
        !out.join("full").join("word").exists(),
        "表组名后面不该再套类型目录"
    );

    // meta：path 是纯表组名，tier_pct 有 6 个值
    for t in &meta.tables {
        assert!(
            !t.path.contains('/'),
            "{} 的 path 应该是纯表组名",
            t.key()
        );
        assert_eq!(t.tier_pct.len(), 6, "{} 缺少前%上界", t.key());
        assert_eq!(t.tiers.len(), 7);
    }
    assert_eq!(
        meta.scopes(),
        vec!["full", "news", "wiki"],
        "full 必须排在最前"
    );

    // 相加：news + wiki == full
    let written = vocfreq_core::compose::compose(&vocfreq_core::compose::ComposeSpec {
        from: out.clone(),
        products: Vec::new(),
        sources: vec!["news/word".into(), "wiki/word".into()],
        scope: "相加".into(),
        kind: Some("word".into()),
        out: None,
    })
    .expect("相加应当成功");
    assert_eq!(
        written[0].entries, 5,
        "中国/人工智能/宇宙/文学/发展 = 5 个不重复的词"
    );

    let ds = Dataset::open(&out).expect("相加后产物仍应能打开");
    let composed = ds.table("相加", "word").expect("新表组应当登记进 meta");
    let full = ds.table("full", "word").expect("full 应当存在");
    assert_eq!(composed.entries, full.entries, "条目数必须与全量扫描一致");
    assert_eq!(
        composed.total_tokens, full.total_tokens,
        "token 总量必须一致"
    );
    for h in full.vfr.positions() {
        let got = composed
            .lookup(&h.word)
            .unwrap_or_else(|| panic!("相加丢了词 {}", h.word));
        assert_eq!(got.count, h.count, "词 {} 的频次不符", h.word);
        assert_eq!(got.rank, h.rank, "词 {} 的排名不符", h.word);
    }
    // 来源可追溯，且它是**一级公民**：能当主表组
    let t = ds.meta.table("相加", "word").unwrap();
    assert!(t.source_tables.contains(&"news/word".to_string()));
    let ds2 = Dataset::open_with_primary(&out, Some("相加")).unwrap();
    assert_eq!(ds2.primary_scope, "相加");
    assert_eq!(ds2.primary_table("word").unwrap().scope, "相加");
    // 单字要回落：相加只加了词频表，字表仍在 full 里，不能因此让整句变"未收录"
    assert!(
        ds2.primary_table("char").is_some(),
        "主表组缺字表时必须回落到别的表"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// **`scan` 默认不产 `full`。**
///
/// 它是本次改造的核心行为变化：全量语料大到一块盘放不下时只能一个表组一个表组地扫成
/// **多份**产物，那种工作流里每次 `scan` 都顺手写一遍 `full` 纯是浪费（写几百 MB，
/// 而且因为整体覆盖，前几次写的全被冲掉）。`full` 改为由
/// 「`merge` 合流 + `compose` 相加」得到。
#[test]
fn scan_produces_no_full_by_default() {
    let root = tmp("scan-no-full");
    let corpus = root.join("corpus");
    let out = root.join("data");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus(&corpus);
    let dict_path = root.join("fixture.dict");
    write_fixture_dict(&dict_path);

    let mut cfg = ScanConfig::new(&corpus, &out);
    cfg.dicts = vec![dict_path];
    cfg.write_tsv = false;
    let meta = scan::scan(&cfg, &|_| {}).expect("扫描应当成功");

    assert!(
        !out.join("full").exists(),
        "默认不该产出 full 表组目录（它由合流 + 相加得到）"
    );
    assert_eq!(meta.scopes(), vec!["news", "wiki"], "只该有各表组自己的表");
    for t in &meta.tables {
        assert_ne!(t.path, "full", "meta 里也不该登记 full");
    }
    // 表组照常产出，`full` 的位置不影响它们
    for scope in ["news", "wiki"] {
        assert!(
            out.join(scope).join("word.vfr").is_file(),
            "缺少 {scope}/word.vfr"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 表组扫成**两份独立产物** → `merge` 合流 → `compose` 相加出 `full`。
///
/// 这条测试走的是本次改造要支持的**真实工作流**（`docs/DATA_LAYOUT.md` §七），
/// 并复用 `scan_writes_flat_scopes_and_compose_matches_full` 的对照思路：
/// 用一次 `--full` 的全量扫描当基准，逐条比对合流产物里相加出来的 `full`。
///
/// ⚠ 两次表组 `scan` 必须用**同一份词典**（`merge` 的硬门槛就是它）。
#[test]
fn scan_by_domain_then_merge_then_compose_matches_one_shot_scan() {
    let root = tmp("merge-workflow");
    let corpus = root.join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus(&corpus);
    let dict_path = root.join("fixture.dict");
    write_fixture_dict(&dict_path);

    // 1) 一个表组一份产物（各扫一次，互不相邻 —— 合流拒绝互相嵌套的源）
    let mut parts: Vec<PathBuf> = Vec::new();
    for domain in ["news", "wiki"] {
        let out = root.join(format!("part-{domain}"));
        let mut cfg = ScanConfig::new(&corpus, &out);
        cfg.dicts = vec![dict_path.clone()];
        cfg.only_domains = vec![domain.to_string()];
        cfg.write_tsv = false;
        let meta = scan::scan(&cfg, &|_| {}).expect("按表组扫描应当成功");
        assert_eq!(meta.scopes(), vec![domain], "按表组扫描只该产出该表组");
        parts.push(out);
    }

    // 2) 合流成一份标准产物
    let merged_dir = root.join("merged");
    let report = vocfreq_core::merge::merge(&vocfreq_core::merge::MergeSpec {
        from: parts.clone(),
        out: merged_dir.clone(),
        scope: None,
    })
    .expect("词典链一致时合流应当成功");
    assert_eq!(report.scopes, vec!["news", "wiki"]);

    // 3) 相加出 full（一次调用把两类都加出来：一个表组只有一份目录，
    //    因此词频表与字表必须在同一次 compose 里产出）
    vocfreq_core::compose::compose(&vocfreq_core::compose::ComposeSpec {
        from: merged_dir.clone(),
        products: Vec::new(),
        sources: vec![
            "news/word".into(),
            "wiki/word".into(),
            "news/char".into(),
            "wiki/char".into(),
        ],
        scope: "full".into(),
        kind: None,
        out: None,
    })
    .expect("合流之后相加应当成功");

    // 4) 对照基准：一次全量扫描（显式要 full）的那份产物
    let one_shot = root.join("one-shot");
    let mut cfg = ScanConfig::new(&corpus, &one_shot);
    cfg.dicts = vec![dict_path];
    cfg.write_full = true;
    cfg.write_tsv = false;
    scan::scan(&cfg, &|_| {}).expect("全量扫描应当成功");

    // 5) 逐条一致：条目数、token 总量、每个词的频次与排名
    let a = Dataset::open(&merged_dir).expect("合流产物必须能打开（桌面端零改动的依据）");
    let b = Dataset::open(&one_shot).expect("全量产物必须能打开");
    for kind in ["word", "char"] {
        let got = a.table("full", kind).expect("合流产物里应当有 full");
        let want = b.table("full", kind).expect("基准产物里应当有 full");
        assert_eq!(got.entries, want.entries, "{kind} 条目数与全量扫描不一致");
        assert_eq!(
            got.total_tokens, want.total_tokens,
            "{kind} token 总量与全量扫描不一致"
        );
        for h in want.vfr.positions() {
            let g = got
                .lookup(&h.word)
                .unwrap_or_else(|| panic!("{kind} 相加丢了 {}", h.word));
            assert_eq!(g.count, h.count, "{kind} {} 的频次不符", h.word);
            assert_eq!(g.rank, h.rank, "{kind} {} 的排名不符", h.word);
        }
    }
    // 合流产物里各表组表也在，桌面端能像普通产物一样按表组对比
    assert!(a.table("news", "word").is_some());
    assert!(a.table("wiki", "char").is_some());

    let _ = std::fs::remove_dir_all(&root);
}

/// **词典链 / 分词口径不一致 → 明确报错、拒绝合流。**
///
/// 这是合流的硬门槛：跨产物合计的正确性完全依赖"各源表用同一词典、同一分词口径"。
/// 实测里最危险的一种是**两份同名但内容不同的词典**：名字一模一样，只有指纹不同。
/// 本测试覆盖它，以及两种"明确拒绝"的形态（分词口径不同、无指纹不可校验）。
#[test]
fn merge_rejects_products_whose_dict_chains_or_options_differ() {
    let root = tmp("merge-reject");
    let corpus = root.join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus(&corpus);
    let dict_path = root.join("fixture.dict");
    write_fixture_dict(&dict_path);

    // 扫两次，产物 A 与 B 除词典链之外完全一样
    let mut parts: Vec<PathBuf> = Vec::new();
    let mut metas: Vec<vocfreq_core::artifact::Meta> = Vec::new();
    for domain in ["news", "wiki"] {
        let out = root.join(format!("part-{domain}"));
        let mut cfg = ScanConfig::new(&corpus, &out);
        cfg.dicts = vec![dict_path.clone()];
        cfg.only_domains = vec![domain.to_string()];
        cfg.write_tsv = false;
        metas.push(scan::scan(&cfg, &|_| {}).expect("按表组扫描应当成功"));
        parts.push(out);
    }

    // A) 词典被换过：内容不同 → 指纹不同。合流必须拒绝。
    let a_meta_path = parts[0].join("meta.json");
    let swapped = root.join("换过的词典.dict");
    std::fs::write(&swapped, "中国 1 n\n").unwrap();
    let mut tampered = metas[0].clone();
    // 只换内容、**保持名字一样** —— 这正是最难靠肉眼发现的那种情况：
    // 「同一份词典」这个判断只有内容指纹能给出。
    let mut new_ref = vocfreq_core::dict::read_dict(&swapped).unwrap().dict;
    new_ref.name = tampered.tokenizer.dicts[0].name.clone();
    new_ref.id = tampered.tokenizer.dicts[0].id.clone();
    tampered.tokenizer.dicts[0] = new_ref;
    vocfreq_core::artifact::write_meta(&a_meta_path, &tampered).unwrap();

    let out = root.join("merged-reject");
    let e = vocfreq_core::merge::merge(&vocfreq_core::merge::MergeSpec {
        from: parts.clone(),
        out: out.clone(),
        scope: None,
    })
    .expect_err("词典内容不同时合流必须被拒绝");
    let msg = e.to_string();
    assert!(msg.contains("词典链"), "错误里要指出是词典链的问题：{msg}");
    assert!(
        msg.contains("内容指纹不同"),
        "错误里要说清是哪个字段不同：{msg}"
    );
    assert!(
        !out.exists(),
        "被拒绝时绝不能留下半截产物：{}",
        out.display()
    );

    // B) 分词口径不同（HMM 开关）→ 同样拒绝，且点名到字段
    let mut modified = metas[1].clone();
    modified.tokenizer.hmm = !modified.tokenizer.hmm;
    vocfreq_core::artifact::write_meta(&parts[1].join("meta.json"), &modified).unwrap();
    // 先把 A 改回与 B 原本一致的词典链，确保这次撞的是 HMM 而不是词典
    vocfreq_core::artifact::write_meta(&a_meta_path, &metas[0]).unwrap();
    let e = vocfreq_core::merge::merge(&vocfreq_core::merge::MergeSpec {
        from: parts.clone(),
        out: out.clone(),
        scope: None,
    })
    .expect_err("分词口径不同时合流必须被拒绝");
    let msg = e.to_string();
    assert!(msg.contains("hmm"), "错误里要点名到字段：{msg}");
    assert!(!out.exists(), "被拒绝时绝不能留下半截产物");

    // C) 两份都没有指纹（老产物）→ 不可校验，同样拒绝：
    //    "不知道是否同一份"不等于"是同一份"。
    let mut legacy = metas[1].clone();
    legacy.tokenizer.dicts.clear();
    vocfreq_core::artifact::write_meta(&parts[1].join("meta.json"), &legacy).unwrap();
    let mut legacy0 = metas[0].clone();
    legacy0.tokenizer.dicts.clear();
    vocfreq_core::artifact::write_meta(&a_meta_path, &legacy0).unwrap();
    let e = vocfreq_core::merge::merge(&vocfreq_core::merge::MergeSpec {
        from: parts.clone(),
        out: out.clone(),
        scope: None,
    })
    .expect_err("无指纹的产物不可校验，合流必须被拒绝");
    assert!(e.to_string().contains("指纹"), "{e}");

    let _ = std::fs::remove_dir_all(&root);
}

/// `scan` 的 `--no-domains` 语义：只产出全量表组。
#[test]
fn scan_can_skip_per_scope_tables() {
    let root = tmp("scan-nodomains");
    let corpus = root.join("corpus");
    let out = root.join("data");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus(&corpus);
    let dict_path = root.join("fixture.dict");
    write_fixture_dict(&dict_path);

    let mut cfg = ScanConfig::new(&corpus, &out);
    cfg.dicts = vec![dict_path];
    cfg.skip_domain_tables = true;
    // 只产全量表组 = 「不要表组」+「要 full」，两个开关缺一不可
    cfg.write_full = true;
    cfg.write_tsv = false;
    let meta = scan::scan(&cfg, &|_| {}).expect("扫描应当成功");

    assert!(out.join("full").join("word.vfr").is_file());
    assert!(!out.join("news").exists(), "选了跳过就不该有表组");
    assert_eq!(meta.scopes(), vec!["full"]);
    let _ = std::fs::remove_dir_all(&root);
}
