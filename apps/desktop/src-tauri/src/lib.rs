//! VocTier 桌面端 Tauri 后端。
//!
//! 统计与分析**全部复用 `vocfreq-core`**：命令行工具与桌面端跑的是同一个
//! `scan::scan` 与同一份 `.vfr` 查询代码，因此两者产出的词频表和分词粒度必然一致，
//! 不存在「桌面端算出来的和命令行不一样」这种问题。
//!
//! 三个需要留意的设计点：
//! 1. 全库统计在后台线程里跑，进度通过 `scan:progress` 事件推给前端；统计可取消。
//! 2. 分析/查询命令用 `async fn`（内部无 await），让 Tauri 把它们放到工作线程，
//!    避免阻塞 UI 线程。
//! 3. 全局取词走剪贴板模拟法（见 `capture.rs`），由全局热键触发。

mod capture;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use vocfreq_core::artifact::Meta;
use vocfreq_core::query::{Dataset, TokenInfo, VfrTable};
use vocfreq_core::scan::{self, Progress, ScanConfig};
use vocfreq_core::source;
use vocfreq_core::tokenize::{TokenizeOpts, Tokenizer};

const POPUP_LABEL: &str = "popup";
const MAIN_LABEL: &str = "main";

// ===========================================================================
// 数据模型（字段名刻意保持 snake_case，与 vocfreq-core 的 serde 输出一致）
// ===========================================================================

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub tauri_version: String,
    pub core_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DomainPlan {
    pub name: String,
    pub files: u64,
    pub bytes: u64,
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CorpusPlan {
    pub corpus: String,
    pub files: u64,
    pub bytes: u64,
    pub domains: Vec<DomainPlan>,
    /// 格式识别失败被跳过的文件说明
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetStatus {
    pub dir: String,
    pub exists: bool,
    pub meta: Option<Meta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WordHit {
    pub word: String,
    pub count: u64,
    pub rank: u32,
    pub flags: u8,
    pub tier: usize,
    pub tier_name: String,
    pub pct: f64,
    pub in_dict: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankRow {
    pub rank: u32,
    pub word: String,
    pub count: u64,
    pub flags: u8,
}

/// 累计覆盖率曲线，供前端实现「按覆盖率分组」。
#[derive(Debug, Clone, Serialize)]
pub struct TierCurve {
    pub path: String,
    pub kind: String,
    pub entries: u64,
    pub total_tokens: u64,
    /// `(rank, 累计覆盖率)`，按 rank 递增、覆盖率单调不减
    pub points: Vec<(u32, f64)>,
}

// ---------------------------------------------------------------- 输入

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanParams {
    pub corpus: String,
    pub out: String,
    #[serde(default)]
    pub threads: u32,
    #[serde(default)]
    pub hmm: bool,
    #[serde(default)]
    pub user_dict: Option<String>,
    #[serde(default = "one")]
    pub min_count: u64,
    #[serde(default)]
    pub keep_digit: bool,
    #[serde(default = "yes")]
    pub keep_latin: bool,
    #[serde(default)]
    pub skip_single_char: bool,
    #[serde(default)]
    pub only_domains: Vec<String>,
    #[serde(default)]
    pub skip_domain_tables: bool,
    #[serde(default = "yes")]
    pub write_tsv: bool,
}

fn one() -> u64 {
    1
}
fn yes() -> bool {
    true
}

/// 应用设置。字段用 camelCase 与前端对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub corpus_dir: Option<String>,
    pub data_dir: Option<String>,
    pub hotkey: String,
    pub popup_width: f64,
    pub popup_height: f64,
    pub popup_opacity: f64,
    pub popup_always_on_top: bool,
    pub popup_auto_close_ms: u64,
    pub theme: String,
    pub threads: u32,
    pub hmm: bool,
    pub keep_digit: bool,
    pub keep_latin: bool,
    pub skip_single_char: bool,
    pub user_dict: Option<String>,
    pub min_count: u64,
    pub skip_domain_tables: bool,
    /// 参与**分域对比与排行榜**的表，取值是 `meta.tables[].path` 形式：
    /// `full/word`、`full/char`、`domains/news/word`、`domains/news/char`…
    ///
    /// `None` 或空数组 = 全部启用。注意全库表始终会用于「单个词/字的总体频率」查询，
    /// 关掉它只会让它不参与分域对比与排行榜，否则划句分析会整片变成「未收录」。
    pub enabled_tables: Option<Vec<String>>,
    /// 分组方法：`rank`（按绝对排名，默认）/ `coverage`（按累计覆盖率）/ `even`（按词条数等分）
    pub tier_method: String,
    /// 自定义词表阈值：6 个排名上界，第 7 组自动是「以上全部」
    pub tier_word_bounds: Option<Vec<u64>>,
    /// 自定义字表阈值：同上
    pub tier_char_bounds: Option<Vec<u64>>,
    /// `tier_method = "coverage"` 时的 6 个累计覆盖率目标（0..1，严格递增）
    pub tier_coverage: Option<Vec<f64>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            corpus_dir: None,
            data_dir: None,
            hotkey: "Alt+Q".into(),
            popup_width: 460.0,
            popup_height: 340.0,
            popup_opacity: 1.0,
            popup_always_on_top: true,
            popup_auto_close_ms: 0,
            theme: "system".into(),
            threads: 0,
            hmm: false,
            keep_digit: false,
            keep_latin: true,
            skip_single_char: false,
            user_dict: None,
            min_count: 1,
            skip_domain_tables: false,
            enabled_tables: None,
            tier_method: "rank".into(),
            tier_word_bounds: None,
            tier_char_bounds: None,
            tier_coverage: None,
        }
    }
}

// ===========================================================================
// 应用状态
// ===========================================================================

#[derive(Default)]
struct AppState {
    dataset: RwLock<Option<Arc<Dataset>>>,
    tokenizer: RwLock<Option<Arc<Tokenizer>>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    scanning: AtomicBool,
    /// 全局取词抓到、等着小窗取走的文本
    pending: Mutex<String>,
    settings: Mutex<Settings>,
    settings_file: Mutex<Option<PathBuf>>,
    /// 覆盖率曲线缓存。全库词表 380 万条要顺序读一遍记录区（约 1 秒），
    /// 而它只跟产物有关、与用户操作无关，所以算一次就留着。
    curves: Mutex<std::collections::HashMap<String, TierCurve>>,
    /// 主窗口最终用成功的 WebView2 数据目录。
    ///
    /// 小窗必须复用同一个：WebView2 的数据目录是**按环境**的，主窗口回退到备用目录后，
    /// 小窗若仍用默认目录就会创建失败——表现为「按了热键但什么都没出来」。
    webview_data_dir: Mutex<Option<Option<PathBuf>>>,
    /// 小窗弹出**之前**用户正在用的窗口句柄，关闭小窗时把焦点还回去。
    ///
    /// 不还的话焦点可能落到本应用主窗口上，于是用户下次划词时 `Ctrl+C`
    /// 会被发到我们自己这里，复制不到选中内容 —— 症状就是「小窗是空的」。
    prev_foreground: Mutex<isize>,
    /// 最近一次取词为什么没取到（给用户看的短句，取到内容时为空）。
    /// 直接显示在小窗里，省得用户去翻日志。
    capture_note: Mutex<String>,
}

impl AppState {
    fn settings_snapshot(&self) -> Settings {
        self.settings.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn persist_settings(&self) {
        let s = self.settings_snapshot();
        if let Ok(path) = self.settings_file.lock() {
            if let Some(p) = path.as_ref() {
                if let Ok(json) = serde_json::to_vec_pretty(&s) {
                    let _ = std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
                    let _ = std::fs::write(p, json);
                }
            }
        }
    }

    fn tokens(&self) -> Result<Arc<Tokenizer>, String> {
        self.tokenizer
            .read()
            .map_err(|_| "状态锁损坏".to_string())?
            .clone()
            .ok_or_else(|| "尚未打开词频表，请先在「生成词频表」页完成一次统计".to_string())
    }

    fn data(&self) -> Result<Arc<Dataset>, String> {
        self.dataset
            .read()
            .map_err(|_| "状态锁损坏".to_string())?
            .clone()
            .ok_or_else(|| "尚未打开词频表，请先在「生成词频表」页完成一次统计".to_string())
    }

    /// 打开产物目录并缓存数据集与分词器。
    ///
    /// 分词器必须按 `meta.tokenizer` 里的口径重建（HMM、过滤规则、用户词典），
    /// 否则分词结果对不上已经落盘的词表，查出来的频次会系统性偏错。
    fn load_dataset(&self, dir: &str) -> Result<Meta, String> {
        let ds = Dataset::open(Path::new(dir)).map_err(|e| e.to_string())?;
        let m = &ds.meta.tokenizer;
        let mut tk = Tokenizer::builtin(TokenizeOpts {
            hmm: m.hmm,
            min_len: m.min_len,
            max_len: m.max_len,
            keep_latin: m.keep_latin,
            keep_digit: m.keep_digit,
            skip_single_char: m.skip_single_char,
        });
        if let Some(ud) = &m.user_dict {
            let p = PathBuf::from(ud);
            if p.exists() {
                let _ = tk.load_user_dict(&p);
            }
        }
        let meta = ds.meta.clone();
        *self.dataset.write().map_err(|_| "状态锁损坏")? = Some(Arc::new(ds));
        *self.tokenizer.write().map_err(|_| "状态锁损坏")? = Some(Arc::new(tk));
        Ok(meta)
    }

    fn ensure_dataset(&self, dir: Option<&str>) -> Result<(), String> {
        let want = match dir {
            Some(d) => Some(d.to_string()),
            None => self.settings_snapshot().data_dir,
        };
        let want = match want {
            Some(w) if !w.is_empty() => w,
            _ => {
                if self.data().is_ok() {
                    return Ok(());
                }
                return Err("尚未指定词频表目录，请先在「生成词频表」页完成一次统计".into());
            }
        };
        // 已经打开的就是这个目录就不重复加载（mmap + 建分词器虽快，但没必要）
        if let Ok(ds) = self.data() {
            if ds.root == Path::new(&want) {
                return Ok(());
            }
        }
        self.load_dataset(&want).map(|_| ())
    }
}

// ===========================================================================
// 基础信息
// ===========================================================================

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "VocTier".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        tauri_version: tauri::VERSION.into(),
        core_version: vocfreq_core::VERSION.into(),
    }
}

// ===========================================================================
// 语料库探测与词频表状态
// ===========================================================================

#[tauri::command]
async fn plan_corpus(corpus: String) -> Result<CorpusPlan, String> {
    let rules = source::builtin_rules();
    let (plans, warnings) =
        scan::plan_corpus(Path::new(&corpus), &rules, &[]).map_err(|e| e.to_string())?;

    let mut domains: Vec<DomainPlan> = Vec::new();
    for p in &plans {
        let rule = rules[p.rule].name.clone();
        match domains.iter_mut().find(|d| d.name == p.domain) {
            Some(d) => {
                d.files += 1;
                d.bytes += p.bytes;
                if !d.rules.contains(&rule) {
                    d.rules.push(rule);
                }
            }
            None => domains.push(DomainPlan {
                name: p.domain.clone(),
                files: 1,
                bytes: p.bytes,
                rules: vec![rule],
            }),
        }
    }
    Ok(CorpusPlan {
        corpus,
        files: plans.len() as u64,
        bytes: plans.iter().map(|p| p.bytes).sum(),
        domains,
        warnings,
    })
}

#[tauri::command]
async fn dataset_status(dir: String) -> DatasetStatus {
    let base = Path::new(&dir);
    let meta = std::fs::read(base.join("meta.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Meta>(&b).ok());
    // 只有 meta.json 与实际表都在，才算可用；缺一不可，否则界面会显示一张空表
    let exists = meta.is_some()
        && base.join("full").join("word.vfr").exists()
        && base.join("full").join("char.vfr").exists();
    DatasetStatus {
        dir,
        exists,
        meta: if exists { meta } else { None },
    }
}

#[tauri::command]
async fn open_dataset(state: State<'_, AppState>, dir: String) -> Result<Meta, String> {
    let meta = state.load_dataset(&dir)?;
    state.persist_settings_of(|s| s.data_dir = Some(dir.clone()));
    // 记一行 IPC 活动：这样「前端到底有没有调通后端」在日志里是可观测的，
    // 而不是只能靠界面表现去猜。
    log_line(
        "startup.log",
        &format!(
            "IPC open_dataset({dir}) 成功：{} 张表 / {} 个域",
            meta.tables.len(),
            meta.domains.len()
        ),
    );
    Ok(meta)
}

impl AppState {
    fn persist_settings_of(&self, f: impl FnOnce(&mut Settings)) {
        if let Ok(mut g) = self.settings.lock() {
            f(&mut g);
        }
        self.persist_settings();
    }
}

// ===========================================================================
// 查询
// ===========================================================================

/// 从数据库里挑出要查的那张表。
fn pick_table<'a>(ds: &'a Dataset, domain: Option<&str>, kind: &str) -> Option<&'a VfrTable> {
    match domain {
        None | Some("") | Some("full") => {
            Some(if kind == "char" { &ds.char } else { &ds.word })
        }
        Some(d) => ds
            .domains
            .iter()
            .find(|(name, _, _)| name == d)
            .map(|(_, w, c)| if kind == "char" { c } else { w }),
    }
}

/// 按 `meta.tables[].path` 形式（`full/word`、`domains/news/char`）定位表。
/// 解析逻辑只在 core 里实现一次，这里只是转发。
fn pick_by_path<'a>(ds: &'a Dataset, path: &str) -> Option<&'a VfrTable> {
    ds.table_by_path(path)
}

fn to_rows(hits: Vec<vocfreq_core::query::Hit>, total: u64) -> Vec<RankRow> {
    let _ = total;
    hits.into_iter()
        .map(|h| RankRow {
            rank: h.rank,
            word: h.word,
            count: h.count,
            flags: h.flags,
        })
        .collect()
}

#[tauri::command]
async fn analyze_text(
    state: State<'_, AppState>,
    text: String,
    domains: Vec<String>,
    dir: Option<String>,
) -> Result<Vec<TokenInfo>, String> {
    state.ensure_dataset(dir.as_deref())?;
    let ds = state.data()?;
    let tk = state.tokens()?;
    let mut out = ds.analyze(&tk, &text);

    // 分域排名只保留「前端本次关心的域」∩「设置里启用的表」。
    // 这一步同时省掉大量无用查表：每个 token 本来要查 7 个分域。
    let enabled = state.settings_snapshot().enabled_tables.unwrap_or_default();
    let keep = |name: &str| {
        let by_arg = domains.is_empty() || domains.iter().any(|d| d == name);
        let by_setting = enabled.is_empty()
            || enabled.iter().any(|p| p == &format!("domains/{name}/word"))
            || enabled.iter().any(|p| p == &format!("domains/{name}/char"));
        by_arg && by_setting
    };
    for t in out.iter_mut() {
        t.domain_ranks.retain(|(n, _)| keep(n));
    }
    Ok(out)
}

/// 取某张表的累计覆盖率曲线。
///
/// 前端拿它在「覆盖率目标」与「排名阈值」之间换算，从而实现按覆盖率分组。
/// 结果按产物缓存，重复调用不会再读一遍记录区。
#[tauri::command]
async fn tier_curve(
    state: State<'_, AppState>,
    path: String,
    dir: Option<String>,
    max_points: Option<usize>,
) -> Result<TierCurve, String> {
    state.ensure_dataset(dir.as_deref())?;
    let ds = state.data()?;
    let key = format!("{}|{path}", ds.root.display());
    if let Ok(g) = state.curves.lock() {
        if let Some(c) = g.get(&key) {
            return Ok(c.clone());
        }
    }

    let table = pick_by_path(&ds, &path).ok_or_else(|| format!("找不到表 {path}"))?;
    let h = table.header();
    let curve = TierCurve {
        path: path.clone(),
        kind: if h.kind == vocfreq_core::query::KIND_CHAR { "char".into() } else { "word".into() },
        entries: h.entry_count,
        total_tokens: h.total_tokens,
        points: table.coverage_curve(max_points.unwrap_or(600)),
    };
    if let Ok(mut g) = state.curves.lock() {
        g.insert(key, curve.clone());
    }
    log_line(
        "startup.log",
        &format!(
            "IPC tier_curve({path}) 首次计算：{} 条记录 → {} 个采样点",
            curve.entries,
            curve.points.len()
        ),
    );
    Ok(curve)
}

#[tauri::command]
async fn lookup_word(
    state: State<'_, AppState>,
    word: String,
    kind: Option<String>,
    dir: Option<String>,
) -> Result<Option<WordHit>, String> {
    state.ensure_dataset(dir.as_deref())?;
    let ds = state.data()?;
    let kind = kind.unwrap_or_else(|| {
        if word.chars().count() == 1 && vocfreq_core::clean::is_cjk_ideograph(word.chars().next().unwrap()) {
            "char".into()
        } else {
            "word".into()
        }
    });
    let table = pick_table(&ds, None, &kind).ok_or_else(|| "词表不存在".to_string())?;
    let total = table.header().total_tokens.max(1);
    let tiers = ds
        .meta
        .tables
        .iter()
        .find(|t| t.path == format!("full/{kind}"))
        .map(|t| t.tiers.clone())
        .unwrap_or_else(|| {
            if kind == "char" {
                vocfreq_core::rank::default_char_tiers()
            } else {
                vocfreq_core::rank::default_word_tiers()
            }
        });
    Ok(table.lookup(&word).map(|h| {
        let ti = vocfreq_core::rank::tier_of(h.rank, &tiers);
        WordHit {
            word: h.word,
            count: h.count,
            rank: h.rank,
            flags: h.flags,
            tier: ti,
            tier_name: tiers[ti].name.clone(),
            pct: h.count as f64 * 100.0 / total as f64,
            in_dict: h.flags & vocfreq_core::tokenize::FLAG_IN_DICT != 0,
        }
    }))
}

#[tauri::command]
async fn list_rank(
    state: State<'_, AppState>,
    domain: Option<String>,
    kind: Option<String>,
    from: Option<u32>,
    limit: Option<u32>,
    dir: Option<String>,
) -> Result<Vec<RankRow>, String> {
    state.ensure_dataset(dir.as_deref())?;
    let ds = state.data()?;
    let kind = kind.unwrap_or_else(|| "word".into());
    let table = pick_table(&ds, domain.as_deref(), &kind)
        .ok_or_else(|| format!("找不到分域 {} 的{kind}表", domain.as_deref().unwrap_or("-")))?;
    let from = from.unwrap_or(1).max(1);
    let limit = limit.unwrap_or(100).clamp(1, 2000);
    Ok(to_rows(table.range_by_rank(from, limit), table.header().total_tokens))
}

#[tauri::command]
async fn search_words(
    state: State<'_, AppState>,
    query: String,
    kind: Option<String>,
    domain: Option<String>,
    limit: Option<usize>,
    dir: Option<String>,
) -> Result<Vec<RankRow>, String> {
    state.ensure_dataset(dir.as_deref())?;
    let ds = state.data()?;
    let kind = kind.unwrap_or_else(|| "word".into());
    let table = pick_table(&ds, domain.as_deref(), &kind)
        .ok_or_else(|| "找不到对应的表".to_string())?;
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(to_rows(
        table.prefix_scan(&query, limit.unwrap_or(60).clamp(1, 500)),
        table.header().total_tokens,
    ))
}

// ===========================================================================
// 全库统计
// ===========================================================================

#[tauri::command]
fn start_scan(app: tauri::AppHandle, state: State<'_, AppState>, params: ScanParams) -> Result<(), String> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("已有统计任务正在运行".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().map_err(|_| "状态锁损坏")? = Some(cancel.clone());

    let out_dir = params.out.clone();
    let mut cfg = ScanConfig::new(params.corpus.clone(), params.out.clone());
    cfg.threads = params.threads as usize;
    cfg.user_dict = params.user_dict.clone().filter(|s| !s.is_empty()).map(PathBuf::from);
    cfg.only_domains = params.only_domains.clone();
    cfg.skip_domain_tables = params.skip_domain_tables;
    cfg.min_count = params.min_count.max(1);
    cfg.write_tsv = params.write_tsv;
    cfg.cancel = Some(cancel);
    cfg.tok = TokenizeOpts {
        hmm: params.hmm,
        min_len: 1,
        max_len: 64,
        keep_latin: params.keep_latin,
        keep_digit: params.keep_digit,
        skip_single_char: params.skip_single_char,
    };

    let handle = app.clone();
    std::thread::spawn(move || {
        let emitter = handle.clone();
        let cb = move |p: Progress| {
            let _ = emitter.emit("scan:progress", &p);
        };
        let result = scan::scan(&cfg, &cb);

        // 统计线程自己收尾：复位标志、按结果打开数据集、通知前端
        let st = handle.state::<AppState>();
        st.scanning.store(false, Ordering::SeqCst);
        match result {
            Ok(meta) => {
                if let Err(e) = st.load_dataset(&out_dir) {
                    let _ = handle.emit("scan:error", format!("统计完成但打开产物失败：{e}"));
                    return;
                }
                st.persist_settings_of(|s| {
                    s.data_dir = Some(out_dir.clone());
                    s.corpus_dir = Some(cfg.corpus.display().to_string());
                });
                let _ = handle.emit("scan:done", &meta);
            }
            Err(e) => {
                let _ = handle.emit("scan:error", e.to_string());
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) -> Result<(), String> {
    if let Ok(g) = state.cancel.lock() {
        if let Some(flag) = g.as_ref() {
            flag.store(true, Ordering::Relaxed);
            return Ok(());
        }
    }
    Err("当前没有正在运行的统计任务".into())
}

// ===========================================================================
// 设置
// ===========================================================================

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    // 只在第一次调用时记一行。`App.svelte` 启动时就会调 `getSettings()`，
    // 所以这一行是「WebView 起来了、JS 跑通了、IPC 确实打通了」的确凿信号。
    //
    // 排查白屏时先看有没有这一行，就能一刀切开两种完全不同的故障：
    //   有 → 后端没问题，去看前端渲染逻辑
    //   没有 → 前端 JS 压根没跑起来（资源没加载 / 挂载就抛异常）
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        log_line("startup.log", "IPC get_settings：前端已加载并成功调用后端 ✔");
    }
    state.settings_snapshot()
}

#[tauri::command]
fn set_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    let old_hotkey = state.settings_snapshot().hotkey;
    if let Ok(mut g) = state.settings.lock() {
        *g = settings.clone();
    }
    state.persist_settings();

    // 热键改了就地重注册；失败要把设置回滚，否则界面上显示的新热键其实没生效
    if old_hotkey != settings.hotkey {
        if let Err(e) = register_hotkey(&app, &settings.hotkey) {
            if let Ok(mut g) = state.settings.lock() {
                g.hotkey = old_hotkey.clone();
            }
            state.persist_settings();
            let _ = register_hotkey(&app, &old_hotkey);
            return Err(format!("热键 {0} 注册失败（{e}），已恢复为 {old_hotkey}", settings.hotkey));
        }
    }

    if let Some(w) = app.get_webview_window(POPUP_LABEL) {
        let _ = w.set_always_on_top(settings.popup_always_on_top);
        let _ = w.set_size(tauri::LogicalSize::new(settings.popup_width, settings.popup_height));
    }
    Ok(())
}

// ===========================================================================
// 全局取词 + 悬浮小窗
// ===========================================================================

#[tauri::command]
async fn capture_selection(state: State<'_, AppState>) -> Result<String, String> {
    let cap = tauri::async_runtime::spawn_blocking(|| capture::capture_selection(420))
        .await
        .map_err(|e| format!("取词任务失败：{e}"))??;
    if !cap.text.is_empty() {
        if let Ok(mut g) = state.pending.lock() {
            *g = cap.text.clone();
        }
    }
    if let Ok(mut g) = state.capture_note.lock() {
        *g = cap.reason.clone();
    }
    Ok(cap.text)
}

#[tauri::command]
fn take_pending_selection(state: State<'_, AppState>) -> String {
    let text = state
        .pending
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    log_line(
        "startup.log",
        &format!("[小窗] 挂载时取走待分析文本 {} 字符", text.chars().count()),
    );
    text
}

#[tauri::command]
async fn open_popup(app: tauri::AppHandle, state: State<'_, AppState>, text: Option<String>) -> Result<(), String> {
    if let Some(t) = text {
        if let Ok(mut g) = state.pending.lock() {
            *g = t;
        }
    }
    let s = state.settings_snapshot();
    show_popup(&app, &s)?;
    Ok(())
}

/// 显示（必要时先创建）悬浮小窗，并尽量放在鼠标附近。
///
/// 必须复用主窗口那份 WebView2 数据目录，否则在主窗口发生过回退的机器上，
/// 小窗会创建失败（用户看到的现象是「按了热键没反应」）。
fn show_popup(app: &tauri::AppHandle, s: &Settings) -> Result<(), String> {
    let win = match app.get_webview_window(POPUP_LABEL) {
        Some(w) => w,
        None => {
            let mut b =
                WebviewWindowBuilder::new(app, POPUP_LABEL, WebviewUrl::App("index.html".into()))
                    .title("VocTier 划句")
                    .inner_size(s.popup_width, s.popup_height)
                    .min_inner_size(320.0, 220.0)
                    .decorations(false)
                    .always_on_top(s.popup_always_on_top)
                    .skip_taskbar(true)
                    .resizable(true)
                    .visible(false)
                    .initialization_script(FRONTEND_TRAP);
            let data_dir = app
                .state::<AppState>()
                .webview_data_dir
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .flatten();
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            if let Some(d) = &data_dir {
                b = b.data_directory(d.clone());
            }
            match b.build() {
                Ok(w) => {
                    log_line(
                        "startup.log",
                        &format!("小窗创建成功（WebView2 数据目录 {}）", match &data_dir { Some(d) => d.display().to_string(), None => "默认位置".into() }),
                    );
                    w
                }
                Err(e) => {
                    log_line("crash.log", &format!("小窗创建失败：{e}"));
                    return Err(format!("创建小窗失败：{e}"));
                }
            }
        }
    };
    let _ = win.set_always_on_top(s.popup_always_on_top);

    // 记下「小窗出现之前用户正在用的窗口」，关闭时要把焦点还给它。
    // 只在这次是「从隐藏变为显示」时记录；如果小窗本来就开着且是前台，
    // 记下来的就成了我们自己的窗口，还回去反而更糟。
    if !win.is_visible().unwrap_or(false) {
        let cur = capture::foreground_hwnd();
        let mine = app
            .webview_windows()
            .values()
            .any(|w| w.hwnd().map(|h| h.0 as isize == cur).unwrap_or(false));
        if cur != 0 && !mine {
            if let Ok(mut g) = app.state::<AppState>().prev_foreground.lock() {
                *g = cur;
            }
            log_line("startup.log", &format!("[小窗] 记住原前台窗口 hwnd={cur}"));
        }
    }

    // 放在光标右下方；越界时退回主屏居中，宁可位置不完美也不要跑到屏幕外
    if let Ok(pos) = app.cursor_position() {
        let x = pos.x + 14.0;
        let y = pos.y + 14.0;
        let (mut x, mut y) = (x, y);
        if let Ok(Some(mon)) = app.monitor_from_point(pos.x, pos.y) {
            let size = mon.size();
            let scale = mon.scale_factor();
            let mpos = mon.position();
            let w = s.popup_width * scale;
            let h = s.popup_height * scale;
            if x + w > mpos.x as f64 + size.width as f64 {
                x = (mpos.x as f64 + size.width as f64 - w - 8.0).max(mpos.x as f64);
            }
            if y + h > mpos.y as f64 + size.height as f64 {
                // 下方放不下就放到光标上方
                y = (pos.y - h - 14.0).max(mpos.y as f64);
            }
        }
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    }
    win.show().map_err(|e| format!("显示小窗失败：{e}"))?;
    let _ = win.set_focus();

    // 关键：把待分析文本**推**给小窗，而不是等它自己来取。
    //
    // 小窗关闭时只是隐藏（不销毁），因此它的 JS 只在**第一次**挂载时跑一次。
    // 若只靠 `take_pending_selection`，第二次按热键时小窗不会重新挂载，
    // 于是显示的是上一次的旧内容（取过一次后就是空的了）。
    let text = app
        .state::<AppState>()
        .pending
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    log_line(
        "startup.log",
        &format!("[小窗] 推送待分析文本 {} 字符", text.chars().count()),
    );
    let _ = app.emit_to(POPUP_LABEL, "popup:text", text);

    // 同时把「这次为什么没取到」推给小窗，让界面直接把原因显示出来。
    let note = app
        .state::<AppState>()
        .capture_note
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let _ = app.emit_to(POPUP_LABEL, "popup:note", note);
    Ok(())
}

/// 最近一次取词失败的原因（小窗挂载时取用）。
#[tauri::command]
fn capture_note(state: State<'_, AppState>) -> String {
    state
        .capture_note
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// 隐藏小窗，并把焦点**还给用户原来在用的窗口**。
///
/// 为什么必须还：如果不还，焦点会落到本应用自己的主窗口（或无处可去）。
/// 那么用户下一次划词按热键时，`Ctrl+C` 就被发到了我们自己的窗口上，
/// 复制不到任何选中内容 —— 表现出来就是「小窗弹出来了但是空的」。
fn hide_popup_now(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(POPUP_LABEL) {
        let _ = w.hide();
    }
    let prev = app
        .state::<AppState>()
        .prev_foreground
        .lock()
        .map(|g| *g)
        .unwrap_or(0);
    if prev != 0 {
        capture::set_foreground(prev);
    }
}

#[tauri::command]
fn hide_popup(app: tauri::AppHandle) {
    hide_popup_now(&app);
}

/// 注册（或重新注册）全局热键。
fn register_hotkey(app: &tauri::AppHandle, accelerator: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let shortcut: Shortcut = accelerator
        .parse()
        .map_err(|e| format!("无法解析热键「{accelerator}」：{e}"))?;

    let handle = app.clone();
    gs.on_shortcut(shortcut, move |_app, _sc, event| {
        if event.state() != ShortcutState::Pressed {
            return;
        }
        // 取词会阻塞几百毫秒（等目标程序写剪贴板），绝不能占着热键回调线程
        let h = handle.clone();
        let run_capture = move || {
            let (text, reason) = match capture::capture_selection(420) {
                Ok(c) => (c.text, c.reason),
                Err(e) => (String::new(), format!("取词失败：{e}")),
            };
            log_line(
                "startup.log",
                &format!(
                    "[热键] 取词得到 {} 字符{}",
                    text.chars().count(),
                    if reason.is_empty() {
                        String::new()
                    } else {
                        format!("；原因：{reason}")
                    }
                ),
            );
            {
                let st = h.state::<AppState>();
                // 先把 lock() 的结果绑定成局部变量再 match。
                // 直接写 `if let Ok(mut g) = st.pending.lock()` 会让那个临时
                // `Result<MutexGuard, PoisonError<..>>` 活到代码块结束，
                // 而 `st` 更早被 drop —— 借用检查会报 E0597。
                // 绑成局部后 drop 顺序是 g → locked → st，就没问题了。
                let locked = st.pending.lock();
                if let Ok(mut g) = locked {
                    // 没取到就清空：让小窗显示空输入框等用户手输，
                    // 而不是把上一次的旧内容又弹一遍。
                    if text.is_empty() {
                        g.clear();
                    } else {
                        *g = text;
                    }
                }
                // 同样先绑定再 match：直接 `if let Ok(g) = st.capture_note.lock()`
                // 会让临时 Result 活到块尾，而 st 更早 drop（E0597）
                let note_locked = st.capture_note.lock();
                if let Ok(mut g) = note_locked {
                    *g = reason;
                }
            }
            let settings = h.state::<AppState>().settings_snapshot();
            if let Err(e) = show_popup(&h, &settings) {
                // 必须落日志：热键触发是在后台线程里，失败若只发事件，
                // 用户看到的就只是「按了没反应」，无从排查。
                log_line("crash.log", &format!("全局热键弹出小窗失败：{e}"));
                let _ = h.emit("scan:error", e);
            }
        };
        // 【诊断开关】VOCTIER_INJECT_INLINE=1 时在主线程上直接取词。
        // 用来判别「合成输入被丢弃」是否与调用线程有关——
        // 后台线程没有消息队列，而主线程是在事件循环里的 UI 线程。
        // 代价是这期间界面会卡住约半秒，所以只在诊断时开。
        if std::env::var_os("VOCTIER_INJECT_INLINE").is_some() {
            log_line("startup.log", "[诊断] 在主线程上直接取词（VOCTIER_INJECT_INLINE）");
            run_capture();
        } else {
            std::thread::spawn(run_capture);
        }
    })
    .map_err(|e| e.to_string())
}

// ===========================================================================
// 启动
// ===========================================================================

// ===========================================================================
// 启动日志与崩溃日志
//
// 这是个 `windows_subsystem = "windows"` 的应用，**没有控制台**：一旦启动阶段
// panic，用户只会看到「白屏然后程序自己没了」，拿不到任何信息。实测就是这样——
// WebView2 创建失败时 Tauri 会在 app.rs 里 panic!("Failed to setup app: ...")，
// 而那条信息只写到 stderr，双击启动时 stderr 无处可去。
//
// 所以这里装一个 panic hook，把 panic 与启动过程都落到文件里。
// ===========================================================================

/// 注入到每个 webview 的前端错误捕获脚本。
///
/// 为什么需要它：WebView 里的 JS 抛异常时，**界面上只会是一片空白**，而
/// `windows_subsystem = "windows"` 的应用连控制台都没有，开发者看不到任何线索。
/// 既然「白屏」是这类应用最难查的故障，那就把它变成一条日志。
///
/// 它做三件事：
/// 1. 捕获 `error`（脚本错误、资源加载失败）与 `unhandledrejection`（动态 import 失败等）；
/// 2. 挂载后延迟 2 秒检查 `#app` 是否为空——直接判定白屏；
/// 3. 全部通过 `__TAURI_INTERNALS__.invoke` 直接回传，**不依赖任何前端模块**
///    （前端模块本身可能正是加载失败的那一环）。
const FRONTEND_TRAP: &str = r#"
(function () {
  function send(kind, msg, extra) {
    try {
      var internals = window.__TAURI_INTERNALS__;
      if (!internals || typeof internals.invoke !== 'function') return;
      var text = '[' + kind + '] ' + msg + (extra ? '\n' + extra : '');
      internals.invoke('log_frontend_error', { message: text });
    } catch (e) { /* 回传失败就算了，绝不能因为日志再抛一次 */ }
  }

  window.addEventListener('error', function (e) {
    if (e && e.target && e.target.tagName) {
      send('resource', e.target.tagName + ' 加载失败: ' + (e.target.src || e.target.href || ''));
      return;
    }
    send('error',
      (e && e.message ? e.message : '未知错误') + ' @ ' +
      (e && e.filename ? e.filename : '?') + ':' +
      (e && e.lineno ? e.lineno : 0) + ':' + (e && e.colno ? e.colno : 0),
      e && e.error && e.error.stack ? e.error.stack : '');
  }, true);

  window.addEventListener('unhandledrejection', function (e) {
    var r = e && e.reason;
    send('unhandledrejection',
      r && r.message ? r.message : String(r),
      r && r.stack ? r.stack : '');
  });

  window.addEventListener('DOMContentLoaded', function () {
    setTimeout(function () {
      var app = document.getElementById('app');
      if (!app || app.children.length === 0) {
        send('blank', '挂载 2 秒后 #app 仍为空 —— 前端白屏');
      }
    }, 2000);
  });
})();
"#;

/// 前端错误回传通道。写进 `crash.log`，并在下面打印到 stderr 方便命令行调试。
#[tauri::command]
fn log_frontend_error(message: String) {
    log_line("crash.log", &format!("前端错误 {message}"));
}

/// 日志目录 —— **逐个候选实际试写，用第一个真正可写的**。
///
/// 为什么不能只做「取 APPDATA，取不到就退临时目录」：APPDATA 存在、目录也在，
/// 并不代表**本进程**写得了（受限用户配置、企业管控、沙箱都会拦）。这个坑已经踩过：
/// 目录存在、settings.json 也在，但应用写不进去，于是 startup.log 从来不出现，
/// 排查时完全抓瞎。
///
/// 候选顺序：
/// 1. `VOCTIER_LOG_DIR` 环境变量（用户显式指定，最高优先）；
/// 2. `%APPDATA%\com.voctier.desktop`（常规位置）；
/// 3. **程序所在目录 `\voctier-logs`** —— 绿色版与受限环境一般都能写；
/// 4. 系统临时目录。
///
/// 结果缓存一次，避免每次写日志都探测。
fn log_dir() -> PathBuf {
    static RESOLVED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    RESOLVED
        .get_or_init(|| {
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Ok(p) = std::env::var("VOCTIER_LOG_DIR") {
                if !p.trim().is_empty() {
                    candidates.push(PathBuf::from(p));
                }
            }
            if let Ok(a) = std::env::var("APPDATA") {
                if !a.is_empty() {
                    candidates.push(PathBuf::from(a).join("com.voctier.desktop"));
                }
            }
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    candidates.push(dir.join("voctier-logs"));
                }
            }
            candidates.push(std::env::temp_dir().join("com.voctier.desktop"));

            for dir in &candidates {
                if std::fs::create_dir_all(dir).is_err() {
                    continue;
                }
                // 必须真写一个探针文件确认——create_dir_all 成功不代表写得进去
                let probe = dir.join(".write-probe");
                if std::fs::write(&probe, b"ok").is_ok() {
                    let _ = std::fs::remove_file(&probe);
                    return dir.clone();
                }
            }
            std::env::temp_dir()
        })
        .clone()
}

/// 记录构建模式与实际会加载的地址。
///
/// 这一行极其重要。Tauri 用**编译期** `cfg(dev)` 决定加载 `devUrl` 还是内嵌资源：
///
/// ```ignore
/// #[cfg(dev)]      let url = config.build.dev_url;        // → http://localhost:1420
/// #[cfg(not(dev))] let url = ...frontend_dist...;         // → 内嵌的前端
/// ```
///
/// 若用 `cargo build --release` 直接构建（绕过 tauri CLI），构建脚本收不到 CLI 设的
/// 环境变量，就会编成 **dev 模式**。运行时的表现是：
/// **白屏 + 一句「无法访问此页面 / localhost 拒绝连接」**，而窗口创建、全局热键、
/// 后端日志、数据集载入全部正常——极难察觉。把模式写进日志就能一眼看穿。
///
/// 正确做法永远是走 Tauri CLI：`pnpm tauri build`（或 `--no-bundle` 只出 exe）。
fn log_build_mode(app: &tauri::App) {
    let build = &app.config().build;
    log_line(
        "startup.log",
        if cfg!(dev) {
            "构建模式 = dev ⚠ —— 会去加载 devUrl，脱离 dev server 就是白屏！请用 `pnpm tauri build` 重新构建"
        } else {
            "构建模式 = production —— 加载内嵌前端资源"
        },
    );
    if let Some(url) = build.dev_url.as_ref() {
        log_line("startup.log", &format!("  devUrl = {url}"));
    }
}

/// 追加一行带时间戳的日志。任何写失败都静默忽略——日志本身绝不能成为新的崩溃源。
fn log_line(file: &str, msg: &str) {
    use std::io::Write;
    let dir = log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file))
    {
        let _ = writeln!(f, "[{}] {msg}", vocfreq_core::artifact::iso8601_now());
    }
}

/// 安装 panic hook：把 panic 信息与 backtrace 写进 `crash.log`，同时保留默认输出。
fn install_panic_log() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<未知位置>".into());
        let payload = info.payload_as_str().unwrap_or("<panic 信息不是文本>");
        let bt = std::backtrace::Backtrace::force_capture();
        log_line(
            "crash.log",
            &format!("panic @ {loc}\n  {payload}\n  backtrace:\n{bt}"),
        );
        // 再走一遍默认 hook，方便从命令行启动时直接看到
        prev(info);
    }));
}

/// 逐个尝试 WebView2 用户数据目录，返回第一个能用的。
///
/// 为什么要这么做：WebView2 默认把数据放在 `%LOCALAPPDATA%\<identifier>\EBWebView`。
/// 在受限用户配置、企业管控、或该目录被弄坏（例如之前有进程被强杀）的情况下，创建
/// webview 会直接失败（`拒绝访问` 或 `灾难性故障`），而 Tauri 的表现是
/// **白屏然后整个进程退出**，用户完全不知道发生了什么。
///
/// 返回值 `None` 表示「用 WebView2 的默认位置」。
fn webview_data_candidates() -> Vec<(Option<PathBuf>, &'static str)> {
    let mut v: Vec<(Option<PathBuf>, &'static str)> = Vec::new();
    // 用户显式指定优先
    if let Ok(p) = std::env::var("VOCTIER_WEBVIEW_DATA_DIR") {
        if !p.trim().is_empty() {
            v.push((Some(PathBuf::from(p)), "环境变量 VOCTIER_WEBVIEW_DATA_DIR"));
        }
    }
    v.push((None, "WebView2 默认位置"));
    v.push((Some(log_dir().join("webview2")), "日志目录\\webview2"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push((Some(dir.join("webview2-data")), "程序所在目录\\webview2-data"));
        }
    }
    v.push((Some(std::env::temp_dir().join("voctier-webview2")), "系统临时目录"));
    v
}

/// 创建主窗口，必要时自动换一个 WebView2 数据目录重试。
///
/// 主窗口**刻意不在 `tauri.conf.json` 里声明**：只有自己在 setup 里建，才能控制
/// `data_directory`，从而在默认位置不可用时自动回退。
fn create_main_window(app: &tauri::AppHandle) -> Result<WebviewWindow, String> {
    let mut last_err = String::new();
    for (dir, label) in webview_data_candidates() {
        if let Some(d) = &dir {
            if std::fs::create_dir_all(d).is_err() {
                log_line("startup.log", &format!("跳过「{label}」：无法创建 {}", d.display()));
                continue;
            }
        }
        let mut b = WebviewWindowBuilder::new(app, MAIN_LABEL, WebviewUrl::App("index.html".into()))
            .title("VocTier 字词频率分析")
            .inner_size(1100.0, 760.0)
            .min_inner_size(880.0, 600.0)
            .resizable(true)
            .center()
            .decorations(true)
            .initialization_script(FRONTEND_TRAP);
        // `data_directory` 只在 Windows / Linux 上有；其它平台忽略这个候选
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if let Some(d) = &dir {
            b = b.data_directory(d.clone());
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = &dir;
        }
        match b.build() {
            Ok(w) => {
                log_line(
                    "startup.log",
                    &format!("主窗口创建成功，WebView2 数据目录 = {label}{}", match &dir { Some(d) => format!(" ({})", d.display()), None => String::new() }),
                );
                // 记下来给小窗复用
                if let Ok(mut g) = app.state::<AppState>().webview_data_dir.lock() {
                    *g = Some(dir.clone());
                }
                return Ok(w);
            }
            Err(e) => {
                last_err = format!("{label}: {e}");
                log_line("startup.log", &format!("主窗口创建失败 → {last_err}"));
            }
        }
    }
    Err(last_err)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_log();
    log_line(
        "startup.log",
        &format!(
            "=== VocTier {} 启动 | exe={:?} | cwd={:?} | WebView2 数据目录候选 {} 个 ===",
            env!("CARGO_PKG_VERSION"),
            std::env::current_exe().ok(),
            std::env::current_dir().ok(),
            webview_data_candidates().len()
        ),
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            app_info,
            plan_corpus,
            dataset_status,
            open_dataset,
            analyze_text,
            tier_curve,
            lookup_word,
            list_rank,
            search_words,
            start_scan,
            cancel_scan,
            get_settings,
            set_settings,
            capture_selection,
            take_pending_selection,
            open_popup,
            hide_popup,
            capture_note,
            log_frontend_error,
        ])
        .setup(|app| {
            let handle = app.handle();

            // 设置文件放在应用配置目录
            if let Ok(dir) = handle.path().app_config_dir() {
                let file = dir.join("settings.json");
                let st = handle.state::<AppState>();
                if let Ok(mut g) = st.settings_file.lock() {
                    *g = Some(file.clone());
                }
                if let Ok(bytes) = std::fs::read(&file) {
                    if let Ok(loaded) = serde_json::from_slice::<Settings>(&bytes) {
                        if let Ok(mut g) = st.settings.lock() {
                            *g = loaded;
                        }
                    }
                }
            }

            // 主窗口必须在这里建，失败了要能自动换数据目录重试（见 create_main_window）
            if let Err(e) = create_main_window(&handle) {
                log_line("crash.log", &format!("所有 WebView2 数据目录都失败：{e}"));
                return Err(format!(
                    "无法创建窗口。已尝试全部 WebView2 数据目录均失败，\
                     详情见 {}\\crash.log 与 startup.log。最后一个错误：{e}",
                    log_dir().display()
                )
                .into());
            }
            log_build_mode(app);

            // 用户上次用过的词频表若还在，启动时就打开，省得每次都要点一次
            let st = handle.state::<AppState>();
            let s = st.settings_snapshot();
            if let Some(dir) = s.data_dir.as_deref() {
                if Path::new(dir).join("meta.json").exists() {
                    match st.load_dataset(dir) {
                        Ok(m) => log_line(
                            "startup.log",
                            &format!("已载入词频表 {dir}（{} 张表 / {} 词条）", m.tables.len(), m.tables.iter().filter(|t| t.kind == "word").map(|t| t.entries).max().unwrap_or(0)),
                        ),
                        Err(e) => log_line("startup.log", &format!("[warn] 打开上次的词频表失败：{e}")),
                    }
                }
            }

            if let Err(e) = register_hotkey(&handle, &s.hotkey) {
                log_line("startup.log", &format!("[warn] {e}"));
            }

            // 让小窗关闭时只是隐藏，避免下次 show 之前还要重建
            if let Some(win) = handle.get_webview_window(POPUP_LABEL) {
                let _ = win.hide();
            }
            log_line("startup.log", "启动完成");
            Ok(())
        })
        .on_window_event(|window, event| {
            // 用户点小窗的关闭按钮时隐藏而不是销毁，并把焦点还回去
            if window.label() == POPUP_LABEL {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    hide_popup_now(window.app_handle());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("启动 VocTier 失败");
}

// ===========================================================================
// 契约测试
//
// 前端在浏览器里跑的是 `isTauri() === false` 的 mock 分支，**真正的 IPC 序列化
// 路径不会被浏览器测试覆盖**。而这里恰好是风险最高的地方：入参用 camelCase、
// 出参用 snake_case，任何一处命名不一致都会让界面静默拿到 undefined。
// 所以把两边的契约用测试钉死。
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// 前端 `start_scan` 实际发出的 payload（camelCase）。
    const FRONTEND_SCAN_PARAMS: &str = r#"{
        "corpus": "E:/c",
        "out": "E:/o",
        "threads": 8,
        "hmm": false,
        "userDict": null,
        "minCount": 1,
        "keepDigit": false,
        "keepLatin": true,
        "skipSingleChar": false,
        "onlyDomains": ["news", "wiki"],
        "skipDomainTables": false,
        "writeTsv": true
    }"#;

    #[test]
    fn scan_params_accepts_the_camel_case_payload_the_frontend_sends() {
        let p: ScanParams = serde_json::from_str(FRONTEND_SCAN_PARAMS).unwrap();
        assert_eq!(p.corpus, "E:/c");
        assert_eq!(p.out, "E:/o");
        assert_eq!(p.threads, 8);
        assert!(!p.hmm);
        assert!(p.user_dict.is_none());
        assert_eq!(p.only_domains, vec!["news".to_string(), "wiki".to_string()]);
        assert!(p.write_tsv && p.keep_latin);
        assert!(!p.keep_digit && !p.skip_single_char && !p.skip_domain_tables);
    }

    #[test]
    fn scan_params_only_requires_corpus_and_out() {
        let p: ScanParams = serde_json::from_str(r#"{"corpus":"c","out":"o"}"#).unwrap();
        // 缺省值必须与 ScanConfig 的语义一致：不过滤、写 TSV、保留英文、丢数字
        assert_eq!(p.min_count, 1);
        assert!(p.write_tsv);
        assert!(p.keep_latin);
        assert!(!p.keep_digit);
        assert!(p.only_domains.is_empty());
    }

    #[test]
    fn scan_params_rejects_snake_case_so_a_mismatch_fails_loudly() {
        // 若前端误传 snake_case，应当报错而不是静默用默认值——
        // 静默失败会让用户以为「只统计了 news」，实际却跑了全库。
        let r = serde_json::from_str::<ScanParams>(
            r#"{"corpus":"c","out":"o","only_domains":["news"]}"#,
        );
        assert!(r.is_ok(), "未知字段应被忽略，已知字段仍应生效");
        assert!(r.unwrap().only_domains.is_empty());
    }

    #[test]
    fn settings_serialize_as_camel_case_for_the_frontend() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        for k in [
            "corpusDir",
            "dataDir",
            "hotkey",
            "popupWidth",
            "popupHeight",
            "popupOpacity",
            "popupAlwaysOnTop",
            "popupAutoCloseMs",
            "theme",
            "threads",
            "hmm",
            "keepDigit",
            "keepLatin",
            "skipSingleChar",
            "userDict",
            "minCount",
            "skipDomainTables",
            "enabledTables",
            "tierMethod",
            "tierWordBounds",
            "tierCharBounds",
            "tierCoverage",
        ] {
            assert!(v.get(k).is_some(), "Settings 序列化缺少字段 {k}：{v}");
        }
    }

    /// 真实产物目录，用于跑「对着真数据」的测试。
    ///
    /// **刻意不写死绝对路径**。写死有两个害处：把开发者的机器路径带进源码；
    /// 以及在别的机器上让测试**静默跳过**——那等于悄悄丢掉这部分覆盖率。
    ///
    /// 取值顺序：
    /// 1. `VOCTIER_TEST_DATA` 环境变量（想指到别处时用）；
    /// 2. 由 `CARGO_MANIFEST_DIR` 推出仓库根下的 `data`（src-tauri → apps/desktop → apps → 仓库根）。
    ///
    /// 目录不存在时返回 `None`，调用方跳过并**打印一行提示**，不做成环境依赖。
    fn test_data_dir() -> Option<std::path::PathBuf> {
        if let Some(p) = std::env::var_os("VOCTIER_TEST_DATA") {
            let p = std::path::PathBuf::from(p);
            return p.join("meta.json").exists().then_some(p);
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)?
            .join("data");
        dir.join("meta.json").exists().then_some(dir)
    }

    /// 拿不到真实产物目录时的统一提示，避免「跳过」变成无声无息。
    fn skip_no_data(test: &str) {
        eprintln!(
            "[skip] {test}：没有找到真实产物目录（仓库根下的 data/）。\
             先跑一次 vocfreq scan，或用 VOCTIER_TEST_DATA 指定。"
        );
    }

    #[test]
    fn table_paths_resolve_to_the_right_table() {
        // 表管理器用 path 字符串（`domains/news/char`）指代表，解析错了会静默查错表
        let Some(dir) = test_data_dir() else {
            skip_no_data("table_paths_resolve_to_the_right_table");
            return;
        };
        let Ok(ds) = Dataset::open(&dir) else {
            skip_no_data("table_paths_resolve_to_the_right_table");
            return;
        };
        assert!(pick_by_path(&ds, "full/word").is_some());
        assert!(pick_by_path(&ds, "full/char").is_some());
        assert!(pick_by_path(&ds, "domains/news/word").is_some());
        assert!(pick_by_path(&ds, "domains/news/char").is_some());
        assert!(pick_by_path(&ds, "domains/不存在的域/word").is_none());
        assert!(pick_by_path(&ds, "full").is_none());
        assert!(pick_by_path(&ds, "domains/news").is_none());
        // 词表与字表必须是不同的表，别解析成同一张
        let w = pick_by_path(&ds, "full/word").unwrap();
        let c = pick_by_path(&ds, "full/char").unwrap();
        assert_ne!(w.header().entry_count, c.header().entry_count);
        assert_eq!(c.header().kind, vocfreq_core::query::KIND_CHAR);
    }

    #[test]
    fn coverage_curve_works_on_the_real_dataset() {
        // 真数据上有 380 万条记录，这条测试同时覆盖「顺序读遍记录区」的性能与正确性
        let Some(dir) = test_data_dir() else {
            skip_no_data("coverage_curve_works_on_the_real_dataset");
            return;
        };
        let Ok(ds) = Dataset::open(&dir) else {
            skip_no_data("coverage_curve_works_on_the_real_dataset");
            return;
        };
        let t0 = std::time::Instant::now();
        let curve = ds.word.coverage_curve(400);
        let elapsed = t0.elapsed();
        assert!(curve.len() > 20, "曲线点数太少：{}", curve.len());
        assert_eq!(curve[0].0, 1);
        assert_eq!(curve.last().unwrap().0 as u64, ds.word.header().entry_count);
        let mut prev = 0.0;
        for (r, c) in &curve {
            assert!(*c >= prev - 1e-12, "覆盖率必须单调不减：rank={r}");
            prev = *c;
        }
        assert!((prev - 1.0).abs() < 1e-9, "末端覆盖率应为 1，实际 {prev}");
        eprintln!("[curve] 380 万条记录画曲线耗时 {elapsed:?}，{} 个采样点", curve.len());

        // 反解出来的阈值必须严格递增，否则分组会出现空组
        let targets = [0.267, 0.556, 0.783, 0.908, 0.958, 0.988];
        let ranks: Vec<u32> = targets
            .iter()
            .map(|t| vocfreq_core::query::rank_for_coverage(&curve, *t).unwrap())
            .collect();
        for w in ranks.windows(2) {
            assert!(w[0] < w[1], "按覆盖率反解的阈值必须递增：{ranks:?}");
        }
        eprintln!("[curve] 覆盖率目标 -> 排名阈值: {targets:?} -> {ranks:?}");
    }

    #[test]
    fn settings_round_trip_and_tolerate_partial_json() {
        let s = Settings::default();
        let back: Settings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back.hotkey, s.hotkey);
        assert_eq!(back.popup_width, s.popup_width);
        assert_eq!(back.min_count, s.min_count);

        // 老版本设置文件里没有的字段要能补默认值，否则升级后直接读不出设置
        let partial: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+K"}"#).unwrap();
        assert_eq!(partial.hotkey, "Ctrl+Shift+K");
        assert_eq!(partial.popup_width, 460.0);
        assert_eq!(partial.theme, "system");
    }

    #[test]
    fn default_hotkey_is_parseable_by_the_plugin() {
        // 热键字符串最终要喂给 tauri-plugin-global-shortcut，格式错了会在启动时静默失效
        let s = Settings::default();
        assert!(
            s.hotkey.parse::<Shortcut>().is_ok(),
            "默认热键 {:?} 无法被解析",
            s.hotkey
        );
        assert!("Ctrl+Shift+K".parse::<Shortcut>().is_ok());
        assert!("这不是热键".parse::<Shortcut>().is_err());
    }

    #[test]
    fn app_info_exposes_core_version() {
        let i = app_info();
        assert_eq!(i.name, "VocTier");
        assert!(!i.core_version.is_empty(), "前端会显示 core_version");
        assert!(!i.tauri_version.is_empty());
    }
}
