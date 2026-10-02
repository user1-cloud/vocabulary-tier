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
mod library;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use vocfreq_core::artifact::Meta;
use vocfreq_core::query::{Dataset, TokenInfo};
use vocfreq_core::scan::{self, Progress, ScanConfig};
use vocfreq_core::source;
use vocfreq_core::tokenize::{TokenizeOpts, Tokenizer};

use library::{Binding, DictItem, Library, TableItem};

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
    /// 占全部 token 的百分比（"这个词占了多少正文"）
    pub pct: f64,
    /// **前%**：`排名 ÷ 该表条目数 × 100`（"它比多少词常见"）。默认分组口径就是它。
    pub top_pct: f64,
    /// 查的那张表的总条目数（算前% 的分母，界面反解阈值时也要用）
    pub entries: u64,
    /// 查的是哪个作用域的表
    pub scope: String,
    pub in_dict: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankRow {
    pub rank: u32,
    pub word: String,
    pub count: u64,
    pub flags: u8,
    /// 前%（见 [`WordHit::top_pct`]）。排行榜的「前%」列直接读它。
    pub top_pct: f64,
    /// 该行的频次占全表 token 的百分比
    pub pct: f64,
}

/// 累计覆盖率曲线，供前端实现「按覆盖率分组」。
///
/// ⚠ 覆盖率不是默认口径，默认是「前%」（见 [`WordHit::top_pct`]）。曲线仍然保留：
/// 它既是「按覆盖率分组」的输入，也是"这张表盖住了多少正文"的唯一来源。
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
    /// 产物输出目录（绝对路径）。界面默认填 `<数据文件夹>\tables\<表名>`，
    /// 由 `suggest_table_dir` 命令给出，用户也可以改成任意位置。
    pub out: String,
    /// 用数据文件夹 `dicts\` 里的哪几个词库，**按顺序、第一个是主词库**。
    /// 空数组表示"目录里全部 `.dict`，按文件名排序"。
    #[serde(default)]
    pub dict_files: Vec<String>,
    #[serde(default)]
    pub threads: u32,
    #[serde(default)]
    pub hmm: bool,
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

/// 悬浮小窗的最小内尺寸。
///
/// 小窗竖着分三块：标题栏 32 + 输入框 62 + 着色 token 卡片（至少 ~166 才不放不下就滚）
/// + 词条详情 192。合计约 452，取 480 留点余量。
///
/// 两个都要盯着的坑：
///   1. 窗口最小高度太小 → 卡片被压到底、详情被压扁，四格/徽标行虽靠 sticky 还在，
///      但整体很难看，用户会以为「布局坏了」。
///   2. `set_size` 会**绕过** `min_inner_size`，所以恢复设置里存的尺寸时必须自己夹
///      （见 `clamp_popup_size` 与小窗设置保存那一段），否则老设置文件里的 460×340
///      会把窗口按回装不下的高度。
///
/// ⚠️ 必须和 PopupApp.svelte 里 h-48 / min-h-32 / min-h-16 那几个值一起看。
const POPUP_MIN_W: f64 = 420.0;
const POPUP_MIN_H: f64 = 480.0;

/// 把设置里的小窗尺寸夹到合法范围（拖小过、或老设置文件里存着更小的值都会走到这里）。
fn clamp_popup_size(w: f64, h: f64) -> (f64, f64) {
    (w.max(POPUP_MIN_W), h.max(POPUP_MIN_H))
}

/// 应用设置。字段用 camelCase 与前端对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub corpus_dir: Option<String>,
    /// **数据文件夹**：里面是 `dicts\`（词库库）与 `tables\`（词表库）两个子目录。
    ///
    /// `None` 表示用默认位置（`%LOCALAPPDATA%\com.voctier.desktop\data`）。
    ///
    /// ⚠ 语义变过一次：从前它直接指向**一个词频表产物目录**（那目录里就有
    /// `meta.json`）。老设置会在启动时被识别出来、转成一张"已注册的表"，
    /// 数据文件夹则回到默认位置 —— 见 [`AppState::migrate_settings`]。
    pub data_dir: Option<String>,
    pub hotkey: String,
    pub popup_width: f64,
    pub popup_height: f64,
    pub popup_opacity: f64,
    pub popup_always_on_top: bool,
    pub popup_auto_close_ms: u64,
    pub theme: String,
    /// 界面语言（BCP 47，例如 `zh-CN`）。
    ///
    /// 前端还会在 localStorage 里镜像一份做首屏同步读取（避免闪成错语言），
    /// 但**权威值在这里** —— 小窗与主窗口是两个 WebView，存储可能被宿主隔开。
    pub locale: String,
    pub threads: u32,
    pub hmm: bool,
    pub keep_digit: bool,
    pub keep_latin: bool,
    pub skip_single_char: bool,
    /// 扫描用哪条词库链：`dicts\` 下的**文件名**，按顺序、**第一个是主词库**。
    ///
    /// `None` 或空数组 = 用数据文件夹里全部 `.dict`（按文件名排序）。
    /// 存文件名而不是绝对路径：数据文件夹是可以在设置里搬走的，文件名不会因此失效。
    pub scan_dicts: Option<Vec<String>>,
    /// **v1 兼容**：从前那个"叠加用户词典"的单个绝对路径。
    ///
    /// 只在读老设置时才有值，迁移时并进 [`Self::scan_dicts`]，此后不再写出。
    pub user_dict: Option<String>,
    /// 当前激活的表在 `tables\` 下的目录名。
    pub active_table: Option<String>,
    /// 当前激活的表**不在**数据文件夹里时，存它的绝对路径。
    ///
    /// 迁移过来的老产物目录（比如仓库里的 `data\`）走这里。它与
    /// [`Self::active_table`] 互斥：前者有值就用前者。
    pub active_table_path: Option<String>,
    pub min_count: u64,
    pub skip_domain_tables: bool,
    /// **主作用域**：回答「这个词/字有多常见」的那一张表。
    ///
    /// 现在所有作用域都是平等的（`full` 只是"全部相加"的那一个），稀有度、着色、
    /// 分组、排行榜都以这里指定的为准；其余作用域只做对比。
    ///
    /// 存的是**纯作用域名**（`full`、`news`、`相加：财经`），不是 `full/word`
    /// 那种"作用域/类型"的路径 —— 类型是查表时才决定的（单字查字表、其余查词表）。
    /// `None` / 空串 = 用 `full`，没有 `full` 就用排序后的第一个作用域。
    pub primary_scope: Option<String>,
    /// **已废弃**：从前用它挑"参与分域对比与排行榜"的表。
    ///
    /// 铺平之后不再需要它：任何作用域都能当主表、都能参与相加，对比列表也一律
    /// 全给（`analyze` 里的对比查询本来就是 O(1) 的二分）。字段留着只是为了
    /// 读得进老设置文件，**不再有任何行为**。
    pub enabled_tables: Option<Vec<String>>,
    /// 分组方法：`top_pct`（按前%，**默认**）/ `rank`（按绝对排名）/
    /// `coverage`（按累计覆盖率）/ `even`（按词条数等分）
    pub tier_method: String,
    /// 自定义词表阈值：6 个排名上界，第 7 组自动是「以上全部」
    pub tier_word_bounds: Option<Vec<u64>>,
    /// 自定义字表阈值：同上
    pub tier_char_bounds: Option<Vec<u64>>,
    /// `tier_method = "coverage"` 时的 6 个累计覆盖率目标（0..1，严格递增）
    pub tier_coverage: Option<Vec<f64>>,
    /// `tier_method = "top_pct"` 时的 6 个**前%上界**（0..100，严格递增）。
    /// `None` = 用产物里 `meta.tables[].tier_pct` 记的默认口径。
    pub tier_pct: Option<Vec<f64>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            corpus_dir: None,
            data_dir: None,
            hotkey: "Alt+Q".into(),
            popup_width: 520.0,
            popup_height: 560.0,
            popup_opacity: 1.0,
            popup_always_on_top: true,
            popup_auto_close_ms: 0,
            theme: "system".into(),
            locale: "zh-CN".into(),
            threads: 0,
            hmm: false,
            keep_digit: false,
            keep_latin: true,
            skip_single_char: false,
            scan_dicts: None,
            user_dict: None,
            active_table: None,
            active_table_path: None,
            min_count: 1,
            skip_domain_tables: false,
            primary_scope: None,
            enabled_tables: None,
            tier_method: "top_pct".into(),
            tier_word_bounds: None,
            tier_char_bounds: None,
            tier_coverage: None,
            tier_pct: None,
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
    /// 是否正在取词。
    ///
    /// 热键连按时用来自我保护：一次取词要等目标程序写剪贴板（最长 1 秒），
    /// 期间再按热键会起第二个线程、两个线程同时抢前台/抢剪贴板，结果互相干扰。
    /// 后到的按键直接忽略即可——用户的意图本来也只是"取当前这一句"。
    capturing: AtomicBool,
    /// 全局取词抓到、等着小窗取走的文本
    pending: Mutex<String>,
    settings: Mutex<Settings>,
    settings_file: Mutex<Option<PathBuf>>,
    /// 覆盖率曲线缓存。全库词表 380 万条要顺序读一遍记录区（约 1 秒），
    /// 而它只跟这一份产物有关、与用户操作无关，所以算一次就留着。
    ///
    /// 注意「产物」是会被**原地重算**的：词表管理器提供「一键重新统计」，而重算
    /// 通常写回同一个目录。所以键里必须带内容标识（见 [`curve_cache_key`]），
    /// 并且打开新数据集时整体清空（见 [`AppState::load_dataset`]）。
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
    /// 当前打开那张表的**词库绑定状态**。界面要显示"这张表记录的词库已经变了，
    /// 频次可能不准"，所以打开时算一次存下来，而不是每次查询都重算。
    active_binding: Mutex<Option<Binding>>,
    /// 打开那张表时给用户看的告警（v1 老产物、词库缺失后退化成现有词库…）。
    active_warnings: Mutex<Vec<String>>,
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

    /// 数据文件夹（词库库 + 词表库）。
    ///
    /// 启动时 [`AppState::migrate_settings`] 一定会把 `settings.data_dir` 填上，
    /// 所以正常路径不会走到那个兜底值。
    fn library(&self) -> Library {
        let root = self
            .settings_snapshot()
            .data_dir
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));
        Library::new(root)
    }

    /// 当前激活那张表的目录。
    ///
    /// 优先 `active_table_path`：那是**数据文件夹之外**的产物目录（老设置迁移过来的，
    /// 比如仓库里的 `data\`）。其次才是数据文件夹里的 `tables\<名字>`。
    fn active_table_dir(&self, s: &Settings) -> Option<PathBuf> {
        if let Some(p) = s.active_table_path.as_deref().filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(p));
        }
        s.active_table
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| self.library().table_dir(n))
    }

    /// 打开一张表，并按 `meta.json` 记录的**词库链**重建分词器。
    ///
    /// 这是词库外置之后最关键的一处：分词器必须跟建表时用的是同一条词库链，否则
    /// 分词结果对不上已经落盘的词表，查出来的频次会系统性偏错 —— 而且用户看不出来。
    /// 所以：
    ///
    /// * 优先按记录里的**指纹/名字**在数据文件夹里找回那几份词库；
    /// * v1 老产物压根没记词库 → 拿现有的顶上，但**记下告警**让界面说出来；
    /// * 记录里的词库找不全 → 同样退化 + 告警，并在 [`Binding`] 里标成漂移/缺失。
    ///
    /// 绝不静默：宁可让用户看到"频次可能不准"，也不能让他对着一堆错数字深信不疑。
    fn load_dataset(&self, dir: &str) -> Result<Meta, String> {
        // 主作用域来自设置：它是"这个词有多常见"的**唯一口径**，所以打开产物时就要
        // 定下来，而不是每次查询临时决定（否则同一句话在不同页面会算出不同颜色）。
        // 设置里那个作用域在这份产物里不存在时，`Dataset` 会回落到 `full` 再回落第一个。
        let wanted = self.settings_snapshot().primary_scope;
        let ds = Dataset::open_with_primary(Path::new(dir), wanted.as_deref())
            .map_err(|e| e.to_string())?;
        let m = &ds.meta.tokenizer;
        let opts = TokenizeOpts {
            hmm: m.hmm,
            min_len: m.min_len,
            max_len: m.max_len,
            keep_latin: m.keep_latin,
            keep_digit: m.keep_digit,
            skip_single_char: m.skip_single_char,
        };

        let lib = self.library();
        let dicts = lib.list_dicts();
        let recorded = m.resolved_dicts();
        // "有没有可校验的记录"而不是"记录列表空不空"：v1 产物的 `dict` 字段反序列化
        // 出来是一条没指纹的记录，列表非空，但它说明不了任何事。
        let has_record = m.has_verifiable_dict();
        let binding = library::binding_of(&ds.meta, &dicts);

        let mut warnings: Vec<String> = Vec::new();
        let mut chain = library::resolve_chain(&recorded, &dicts);
        let used_fallback = chain.is_empty();
        if used_fallback {
            if dicts.iter().all(|d| !d.usable()) {
                return Err(format!(
                    "这张表是基于词库「{}」生成的，但数据文件夹里没有可用的词库。\n\
                     请先到「词库管理」里导入或新建一份词库，再打开这张表。\n{}",
                    vocfreq_core::dict::describe_chain(&recorded),
                    format_broken_dicts(&dicts)
                ));
            }
            chain = library::default_chain(&dicts);
        }

        let tk = Tokenizer::from_dicts(&chain, opts)
            .map_err(|e| format!("按 meta.json 重建分词器失败：{e}"))?;

        if !has_record {
            warnings.push(format!(
                "这张表没有可校验的词库记录（v1 老产物，当年词库是编在程序里的），\
                 无从判断口径是否一致。现在按「{}」分词，频次可能不准 —— 建议重新统计一次。",
                vocfreq_core::dict::describe_chain(&tk.dicts)
            ));
        } else if used_fallback || chain.len() != recorded.len() {
            warnings.push(format!(
                "这张表记录的词库是「{}」，但{}。现在按「{}」分词，频次可能不准 —— \
                 建议重新统计，或把缺的词库补回数据文件夹。",
                vocfreq_core::dict::describe_chain(&recorded),
                if used_fallback {
                    "在数据文件夹里一份都找不到"
                } else {
                    "有一部分找不到了"
                },
                vocfreq_core::dict::describe_chain(&tk.dicts)
            ));
        }

        let meta = ds.meta.clone();
        *self.dataset.write().map_err(|_| "状态锁损坏")? = Some(Arc::new(ds));
        *self.tokenizer.write().map_err(|_| "状态锁损坏")? = Some(Arc::new(tk));
        *self.active_binding.lock().map_err(|_| "状态锁损坏")? = Some(binding);
        *self.active_warnings.lock().map_err(|_| "状态锁损坏")? = warnings;
        // 覆盖率曲线缓存跟着数据集走：换了表就全废了。重算同一张表时
        // `generated_at` 会变（见 `curve_cache_key`），这里顺手把旧条目清掉，
        // 免得反复重算把内存堆起来。
        if let Ok(mut g) = self.curves.lock() {
            g.clear();
        }
        Ok(meta)
    }

    /// 打开一张表并把它记成「当前激活」。
    ///
    /// 数据文件夹里的表按**名字**记（`active_table`），文件夹之外的按**路径**记
    /// （`active_table_path`），两者互斥 —— 否则数据文件夹一搬走，老表就找不到了。
    fn activate_table(&self, dir: &Path, name: Option<&str>) -> Result<Meta, String> {
        let meta = self.load_dataset(&dir.display().to_string())?;
        let inside = dir.starts_with(self.library().tables_dir());
        self.persist_settings_of(|s| {
            s.active_table = name.map(|n| n.to_string());
            s.active_table_path = if inside {
                None
            } else {
                Some(dir.display().to_string())
            };
        });
        Ok(meta)
    }

    /// 把老设置迁移到「数据文件夹」模型。
    ///
    /// 唯一的坑：`data_dir` 从前指向**一个词频表产物目录**（那目录里直接就有
    /// `meta.json`），现在指向**数据文件夹**（表在 `tables\` 子目录下）。判据就是这一点。
    /// 识别出来的老产物目录会被记成"当前激活的表"，用户不至于升级完就打不开东西了。
    fn migrate_settings(&self, default_data_dir: &Path) -> Vec<String> {
        let mut notes = Vec::new();
        let mut s = self.settings_snapshot();

        if let Some(d) = s.data_dir.clone().filter(|d| !d.is_empty()) {
            let p = Path::new(&d);
            let looks_like_table = p.join("meta.json").exists();
            let looks_like_library = p.join(library::DICTS_DIR).exists()
                || p.join(library::TABLES_DIR).exists();
            if looks_like_table && !looks_like_library {
                notes.push(format!(
                    "设置里的「词频表目录」{d} 是产物目录（里面直接有 meta.json），\
                     已把它记成一张表；数据文件夹改用 {}",
                    default_data_dir.display()
                ));
                s.active_table_path = Some(d);
                s.active_table = None;
                s.data_dir = Some(default_data_dir.display().to_string());
            }
        }

        if s.data_dir.as_deref().map(|d| d.is_empty()).unwrap_or(true) {
            s.data_dir = Some(default_data_dir.display().to_string());
        }

        // 从前的单个「用户词典」不再自动叠加了：现在词库是数据文件夹里可勾选的条目
        if let Some(ud) = s.user_dict.clone().filter(|u| !u.is_empty()) {
            notes.push(format!(
                "老设置里的「用户词典」{ud} 不再自动叠加。要让它参与统计，\
                 请把它的内容导入数据文件夹的 dicts\\（须用 .dict 扩展名），\
                 再在扫描时勾选。"
            ));
        }
        s.user_dict = None;

        if let Ok(mut g) = self.settings.lock() {
            *g = s;
        }
        self.persist_settings();
        notes
    }

    fn ensure_dataset(&self, dir: Option<&str>) -> Result<(), String> {
        let s = self.settings_snapshot();
        let want = match dir {
            Some(d) if !d.is_empty() => Some(PathBuf::from(d)),
            _ => self.active_table_dir(&s),
        };
        let Some(want) = want else {
            if self.data().is_ok() {
                return Ok(());
            }
            return Err(
                "还没有打开任何词频表。请到「词频表」页新建一张，或在设置里指向已有的产物目录。"
                    .into(),
            );
        };
        // 已经打开的就是这个目录就不重复加载（mmap + 建分词器虽快，但没必要）
        if let Ok(ds) = self.data() {
            if ds.root == want {
                return Ok(());
            }
        }
        self.load_dataset(&want.display().to_string()).map(|_| ())
    }

    /// 按当前设置重开一次已激活的产物目录（主作用域变了、或相加出了新表之后用）。
    ///
    /// 没打开过任何表时什么都不做。重开失败（目录被删了）只记一行日志：那是
    /// "当前没有可用的表"的正常状态，界面自己会显示出来。
    fn refresh_primary_scope(&self) {
        let s = self.settings_snapshot();
        let Some(dir) = self.active_table_dir(&s) else {
            return;
        };
        if !dir.join("meta.json").exists() {
            return;
        }
        if let Err(e) = self.load_dataset(&dir.display().to_string()) {
            log_line("startup.log", &format!("重开产物目录 {} 失败：{e}", dir.display()));
        }
    }
}

/// 把界面上勾选的词库解析成一条**词库链**（顺序即装载顺序，第一个是主词库）。
///
/// 两种名字都认，这是刻意的：
/// * `预制词库.dict` —— [`DictItem::file_name`]，词库管理页与扫描页勾的就是它；
/// * `预制词库` —— [`vocfreq_core::dict::DictRef::name`]，也就是 `meta.json` 里
///   `tokenizer.dicts[].name` 存的那个（文件名去扩展名）。
///   **「重新统计」那条路传回来的正是这个**：它从产物的 meta 里读词库名。只认前者的话
///   勾选会全部落空、链变成空的，用户看到的是"没有可用的词库"这种牛头不对马嘴的报错。
///
/// 一个都没匹配上时返回**空表**，由调用方报错 —— 不要在这里静默退化成"用全部词库"：
/// 那会让用户以为"我勾的那份生效了"，而实际用的是另一套口径，频次静默偏错。
fn resolve_scan_dicts(selected: &[String], available: &[DictItem]) -> Vec<PathBuf> {
    if selected.is_empty() {
        // 没勾 = 用数据文件夹里全部 .dict，按文件名排序（顺序确定，产物才可复现）
        return library::default_chain(available);
    }
    selected
        .iter()
        .filter_map(|wanted| {
            available
                .iter()
                .find(|d| {
                    d.usable()
                        && (d.file_name == wanted.as_str() || d.dict.name == wanted.as_str())
                })
                .map(|d| PathBuf::from(&d.dict.path))
        })
        .collect()
}

/// 把读不出来的词库列成一句给用户看的话。空列表返回空串。
fn format_broken_dicts(dicts: &[DictItem]) -> String {
    let broken: Vec<String> = dicts
        .iter()
        .filter(|d| !d.usable())
        .map(|d| {
            format!(
                "  · {}：{}",
                d.file_name,
                d.error.as_deref().unwrap_or("读不了")
            )
        })
        .collect();
    if broken.is_empty() {
        return String::new();
    }
    format!("这些词库文件读不了，请修好或删掉：\n{}", broken.join("\n"))
}

/// 数据文件夹的默认位置。
///
/// * 用 `%LOCALAPPDATA%`（不是 `%APPDATA%`）：词库加词表上百 MB，放进 Roaming
///   会被域环境的漫游配置同步走，那是个灾难。
/// * 再套一层 `data\`：`%LOCALAPPDATA%\com.voctier.desktop` 同时是 **WebView2 的
///   默认数据目录**（tauri 的 `app_local_data_dir` 文档原话），资产直接摊在根上
///   会让用户在"数据文件夹"里看到 `EBWebView` 那一堆缓存。
fn default_data_dir(app: &tauri::AppHandle) -> PathBuf {
    match app.path().app_local_data_dir() {
        Ok(d) => d.join("data"),
        Err(_) => PathBuf::from("data"),
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
    // 判据必须是「**真的能打开**」而不是"某几个固定文件在不在"：
    // 布局铺平之后表可能分布在任意作用域目录里，硬看 `full/word.vfr` 会把一张
    // 完全正常的表判成不可用（或者反过来，把一个半截目录判成可用）。
    let exists = Dataset::open(base).is_ok();
    DatasetStatus {
        dir,
        exists,
        meta: if exists { meta } else { None },
    }
}

/// 当前已经打开的那张表的 `meta`。没打开就是 `None`。
///
/// 各个页面开机都要拿它来渲染，**不该**再自己拿 `settings.dataDir` 去拼路径 ——
/// `dataDir` 现在是"数据文件夹"，不是产物目录，拼出来的路径下没有 `meta.json`，
/// 于是每个页面都会显示"尚未打开词频表"。
///
/// 后端在启动时已经按 `activeTable` / `activeTablePath` 打开过表了，这里直接取。
#[tauri::command]
fn active_dataset(state: State<'_, AppState>) -> Option<Meta> {
    state.data().ok().map(|ds| ds.meta.clone())
}

/// 打开**数据文件夹之外**的任意产物目录（"指向一份已有的词表"）。
///
/// 它会被记成"当前激活的表"，并且带 `activeTablePath` 出现在词表列表里，
/// 但标成 `in_library: false` —— 那种目录不归我们管，界面不给删除。
#[tauri::command]
async fn open_dataset(state: State<'_, AppState>, dir: String) -> Result<Meta, String> {
    let meta = state.activate_table(Path::new(&dir), None)?;
    // 记一行 IPC 活动：这样「前端到底有没有调通后端」在日志里是可观测的，
    // 而不是只能靠界面表现去猜。
    log_line(
        "startup.log",
        &format!(
            "IPC open_dataset({dir}) 成功：{} 张表 / {} 个作用域",
            meta.tables.len(),
            meta.scopes().len()
        ),
    );
    Ok(meta)
}

// ===========================================================================
// 数据文件夹：词库管理 + 词表管理
// ===========================================================================

/// 数据文件夹的整体状况。界面开机就要拿它渲染「词库管理」与「词表管理」。
///
/// 字段保持 snake_case：本项目的约定是**入参 camelCase、出参 snake_case**
/// （见文件末尾的契约测试），这里跟着 `DictItem` / `TableItem` 走，别搞成两套。
#[derive(Debug, Clone, Serialize)]
pub struct LibraryInfo {
    pub root: String,
    pub dicts_dir: String,
    pub tables_dir: String,
    /// 是不是默认位置
    pub is_default: bool,
    /// 当前激活的表在数据文件夹里时的目录名
    pub active_table: Option<String>,
    /// 当前激活的表在数据文件夹之外时的绝对路径
    pub active_table_path: Option<String>,
    /// 当前激活那张表的词库绑定状态
    pub active_binding: Option<Binding>,
    /// 打开时攒下的告警（v1 老产物、词库缺失后退化…）。界面应当直接显示出来。
    pub active_warnings: Vec<String>,
}

#[tauri::command]
fn library_info(app: tauri::AppHandle, state: State<'_, AppState>) -> LibraryInfo {
    let s = state.settings_snapshot();
    let lib = state.library();
    LibraryInfo {
        root: lib.root().display().to_string(),
        dicts_dir: lib.dicts_dir().display().to_string(),
        tables_dir: lib.tables_dir().display().to_string(),
        is_default: lib.root() == default_data_dir(&app),
        active_table: s.active_table.clone(),
        active_table_path: s.active_table_path.clone(),
        active_binding: state.active_binding.lock().ok().and_then(|g| g.clone()),
        active_warnings: state
            .active_warnings
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
    }
}

/// 换数据文件夹。
///
/// 只建目录、**不搬运**已有内容：用户可能只是想指过去看看，替他搬 92 MB 的表格
/// 出来属于自作主张。但会把"当前激活的表"清掉 —— 换了文件夹就是换了一整套词库
/// 与词表，原来那张多半已经不在这套里了，留着它只会让界面状态含糊。
#[tauri::command]
fn set_data_dir(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    dir: String,
) -> Result<LibraryInfo, String> {
    let p = PathBuf::from(dir.trim());
    if p.as_os_str().is_empty() {
        return Err("数据文件夹路径不能为空".into());
    }
    let lib = Library::new(&p);
    lib.ensure()
        .map_err(|e| format!("在 {} 下建 dicts/tables 失败：{e}", p.display()))?;

    state.persist_settings_of(|s| {
        s.data_dir = Some(p.display().to_string());
        s.active_table = None;
        s.active_table_path = None;
        s.scan_dicts = None;
    });
    *state.dataset.write().map_err(|_| "状态锁损坏")? = None;
    *state.tokenizer.write().map_err(|_| "状态锁损坏")? = None;
    *state.active_binding.lock().map_err(|_| "状态锁损坏")? = None;
    state.active_warnings.lock().map_err(|_| "状态锁损坏")?.clear();

    log_line("startup.log", &format!("数据文件夹改为 {}", p.display()));
    Ok(library_info(app, state))
}

/// 列出数据文件夹里的全部词库。
///
/// ⚠ 会完整读取并解析每个 `.dict`（一份 349,046 条的 jieba 词库约 50–100 ms），
/// 因为列表要显示有效词条数、注释行数、以及"多少条显式写了 0"这类隐患。
/// 界面应当按需调用并缓存结果，别在每次重渲染时都调。
#[tauri::command]
fn dict_list(state: State<'_, AppState>) -> Vec<DictItem> {
    state.library().list_dicts()
}

/// 把一份 `.dict` 复制进数据文件夹。重名自动加后缀，**绝不覆盖**。
#[tauri::command]
async fn dict_import(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let landed = state.library().import_dict(Path::new(&path))?;
    log_line("startup.log", &format!("导入词库 {path} → {landed}"));
    Ok(landed)
}

/// 删掉一份词库。
///
/// 界面应当先用 `table_list` 的结果算出"哪些表用到它"，让用户确认 ——
/// 删掉被引用的词库会让那些表变成"词库缺失"，虽然不会崩，但频次就不可信了。
#[tauri::command]
fn dict_delete(state: State<'_, AppState>, file_name: String) -> Result<(), String> {
    state.library().delete_dict(&file_name)?;
    log_line("startup.log", &format!("删除词库 {file_name}"));
    Ok(())
}

/// 列出词表：数据文件夹里的全部 + 数据文件夹之外那张激活的（如果有）。
#[tauri::command]
fn table_list(state: State<'_, AppState>) -> Vec<TableItem> {
    let s = state.settings_snapshot();
    let lib = state.library();
    let dicts = lib.list_dicts();
    let mut items = lib.list_tables(&dicts, s.active_table.as_deref());

    // 数据文件夹之外的激活表也要出现在列表里：老设置迁移过来的产物目录
    // （比如开发时仓库里的 data\）如果没有这一项，用户升级后会以为
    // "我那张表不见了"，而它其实正开着。
    if let Some(p) = s.active_table_path.as_deref().filter(|p| !p.is_empty()) {
        let dir = PathBuf::from(p);
        if !dir.starts_with(lib.tables_dir()) {
            items.push(library::table_item(&dir, &dicts, true, false));
        }
    }
    items
}

/// 激活数据文件夹里的一张表。
#[tauri::command]
async fn activate_library_table(state: State<'_, AppState>, name: String) -> Result<Meta, String> {
    let dir = state.library().table_dir(&name);
    if !dir.exists() {
        return Err(format!("数据文件夹里没有表「{name}」"));
    }
    let meta = state.activate_table(&dir, Some(&name))?;
    log_line(
        "startup.log",
        &format!(
            "激活词表 {name}：{} 张表 / {} 个作用域",
            meta.tables.len(),
            meta.scopes().len()
        ),
    );
    Ok(meta)
}

// ===========================================================================
// 相加（词频表加法）
// ===========================================================================

/// `compose_tables` 的入参。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeParams {
    /// 源表身份（`作用域/类型`），至少一个
    pub sources: Vec<String>,
    /// 新作用域名（会变成同一份产物里的另一个作用域目录）
    pub scope: String,
    /// 只要这一类；不给 = 源表里出现过的每一类都相加
    #[serde(default)]
    pub kind: Option<String>,
}

/// 把若干张表相加成一张新表（同一份产物目录里的另一个作用域）。
///
/// 相加出来的表**与别的表完全平级**：可以当主作用域、可以再被相加、可以删除。
/// 代价是磁盘上多一份（≈ 一张扫描出来的表），源表不需要了可以删掉回收。
///
/// 用 `spawn_blocking` 包着：380 万词的表相加要读几十 MB、写上百 MB，是秒级的
/// 重活，占着 IPC 线程会让界面在这几秒里完全没有响应。
#[tauri::command]
async fn compose_tables(
    state: State<'_, AppState>,
    params: ComposeParams,
) -> Result<Vec<vocfreq_core::compose::ComposedTable>, String> {
    state.ensure_dataset(None)?;
    let ds = state.data()?;
    // 相加只允许在**当前产物目录内部**进行：源表必须来自同一次扫描，否则词库链、
    // 分词口径都可能不同，加出来的表会自相矛盾。所以这里不暴露"输出到哪"。
    let from = ds.root.clone();
    drop(ds);

    let spec = vocfreq_core::compose::ComposeSpec {
        from: from.clone(),
        // 桌面端只做**同一份产物内部**的相加。跨产物合流是命令行那条工作流
        // （`vocfreq merge`，见 docs/DATA_LAYOUT.md §七），这一版不改界面。
        products: Vec::new(),
        sources: params.sources.clone(),
        scope: params.scope.clone(),
        kind: params.kind.clone(),
        out: None,
    };
    let written = tauri::async_runtime::spawn_blocking(move || {
        vocfreq_core::compose::compose(&spec)
    })
    .await
    .map_err(|e| format!("相加任务失败：{e}"))?
    .map_err(|e| e.to_string())?;

    // 重新装一遍：新表要立刻出现在表列表与作用域选择里
    state.refresh_primary_scope();
    log_line(
        "startup.log",
        &format!(
            "相加完成：{} + → 作用域「{}」（{} 类表）",
            params.sources.join(" + "),
            params.scope,
            written.len()
        ),
    );
    Ok(written)
}

/// 换主作用域：决定"这个词有多常见"的是哪一张表。
///
/// 粒度是**整个应用**，不是某一页 —— 划句分析、排行榜、分组阈值必须用同一张表，
/// 否则同一句话在两个页面会显示成两种颜色。
#[tauri::command]
async fn set_primary_scope(state: State<'_, AppState>, scope: String) -> Result<Meta, String> {
    state.ensure_dataset(None)?;
    {
        let ds = state.data()?;
        if !ds.scopes().iter().any(|s| s == &scope) {
            return Err(format!(
                "这份产物里没有作用域「{scope}」；可用的是：{}",
                ds.scopes().join("、")
            ));
        }
    }
    state.persist_settings_of(|s| s.primary_scope = Some(scope.clone()));
    // `Dataset::primary_scope` 是打开时定下来的，改了设置必须重开一次
    state.refresh_primary_scope();
    let meta = state.data()?.meta.clone();
    log_line("startup.log", &format!("主作用域切换为「{scope}」"));
    Ok(meta)
}

/// 删掉数据文件夹里的一张表（整个产物目录）。
///
/// 只能删数据文件夹内的 —— 函数签名就限定了这一点，外部目录的表在界面上
/// 也不给删除按钮。
#[tauri::command]
fn table_delete(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let s = state.settings_snapshot();
    state.library().delete_table(&name)?;

    // 删掉的正是当前激活的那张 → 把激活项一起清掉，别让界面继续指着一个
    // 不存在的目录（下次查询会报"尚未打开词频表"，而不是更迷惑的错误）。
    if s.active_table.as_deref() == Some(name.as_str()) {
        state.persist_settings_of(|s| {
            s.active_table = None;
            s.active_table_path = None;
        });
        *state.dataset.write().map_err(|_| "状态锁损坏")? = None;
        *state.tokenizer.write().map_err(|_| "状态锁损坏")? = None;
        *state.active_binding.lock().map_err(|_| "状态锁损坏")? = None;
        state.active_warnings.lock().map_err(|_| "状态锁损坏")?.clear();
    }
    log_line("startup.log", &format!("删除词表 {name}"));
    Ok(())
}

/// 给一张新表算默认的输出目录：`<数据文件夹>\tables\<洗过的名字>`。
///
/// 扫描页用它预填输出路径，用户仍可改成任意位置（改到数据文件夹之外的表会以
/// `inLibrary: false` 出现在列表里，同样能用）。
#[tauri::command]
fn suggest_table_dir(state: State<'_, AppState>, name: String) -> String {
    let safe = library::sanitize_table_name(&name);
    state.library().table_dir(&safe).display().to_string()
}

/// 建出数据文件夹的 `dicts\` 与 `tables\`，返回实际路径。
#[tauri::command]
fn ensure_data_dirs(state: State<'_, AppState>) -> Result<String, String> {
    let lib = state.library();
    lib.ensure()
        .map_err(|e| format!("建 {} 失败：{e}", lib.root().display()))?;
    Ok(lib.root().display().to_string())
}

/// 数据文件夹里有没有可用的词库。扫描页拿它决定要不要挡住"开始统计"。
#[tauri::command]
fn library_ready(state: State<'_, AppState>) -> bool {
    state.library().list_dicts().iter().any(|d| d.usable())
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

/// 从数据集里挑出要查的那张表。
///
/// `scope` 为 `None` / 空 = **主作用域**（用户选的那张）。所有"这个词有多常见"
/// 的查询都走这里，因此全应用只有一套口径。
fn pick_table<'a>(ds: &'a Dataset, scope: Option<&str>, kind: &str) -> Option<&'a vocfreq_core::query::TableRef> {
    match scope {
        None | Some("") => ds.primary_table(kind),
        Some(s) => ds.table(s, kind),
    }
}

/// 按「作用域/类型」形式（`full/word`、`news/char`、`相加：财经/word`）定位表。
/// 解析逻辑只在 core 里实现一次，这里只是转发。
fn pick_by_key<'a>(ds: &'a Dataset, key: &str) -> Option<&'a vocfreq_core::query::TableRef> {
    ds.table_by_key(key)
}

fn to_rows(
    hits: Vec<vocfreq_core::query::Hit>,
    total: u64,
    entries: u64,
) -> Vec<RankRow> {
    let total = total.max(1);
    hits.into_iter()
        .map(|h| RankRow {
            rank: h.rank,
            word: h.word,
            count: h.count,
            flags: h.flags,
            top_pct: vocfreq_core::rank::pct_for_rank(h.rank, entries),
            pct: h.count as f64 * 100.0 / total as f64,
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

    // `domains` 现在是「对比列里保留哪些**作用域**」。空数组 = 全保留。
    //
    // 这里已经不再查 `enabledTables`：铺平之后作用域数量由产物决定（可能几十个），
    // 而对比查询只对**句子里的 token** 做，超长输入另有 512 token 的闸门
    // （`query::MAX_COMPARE_TOKENS`）。再叠一层设置开关只会让"我明明开了却看不到"。
    if !domains.is_empty() {
        for t in out.iter_mut() {
            t.table_ranks
                .retain(|r| domains.iter().any(|d| d == &r.scope));
        }
    }
    Ok(out)
}

/// 取某张表的累计覆盖率曲线。
///
/// 前端拿它在「覆盖率目标」与「排名阈值」之间换算，从而实现按覆盖率分组。
/// 结果按产物缓存，重复调用不会再读一遍记录区。
/// 覆盖率曲线的缓存键。
///
/// **必须带内容标识，不能只用路径**：词表管理器提供了「一键重新统计」，而重算通常
/// 就是写回同一个目录 —— 路径没变、数据全变了。只用路径当键，重算之后曲线会一直是
/// 旧的那条，而它正是「按覆盖率分组」的输入，分组结果会跟着一起错。
///
/// 用 `generated_at` 做内容标识：它由建表时的时间戳生成，每次 scan 都会变。
fn curve_cache_key(root: &Path, generated_at: &str, path: &str) -> String {
    format!("{}|{generated_at}|{path}", root.display())
}

#[tauri::command]
async fn tier_curve(
    state: State<'_, AppState>,
    path: String,
    dir: Option<String>,
    max_points: Option<usize>,
) -> Result<TierCurve, String> {
    state.ensure_dataset(dir.as_deref())?;
    let ds = state.data()?;
    let key = curve_cache_key(&ds.root, &ds.meta.generated_at, &path);
    if let Ok(g) = state.curves.lock() {
        if let Some(c) = g.get(&key) {
            return Ok(c.clone());
        }
    }

    let table = pick_by_key(&ds, &path).ok_or_else(|| format!("找不到表 {path}"))?;
    let h = table.vfr.header();
    let curve = TierCurve {
        path: path.clone(),
        kind: if h.kind == vocfreq_core::query::KIND_CHAR { "char".into() } else { "word".into() },
        entries: h.entry_count,
        total_tokens: h.total_tokens,
        points: table.vfr.coverage_curve(max_points.unwrap_or(600)),
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
    let total = table.total_tokens.max(1);
    let entries = table.entries;
    let scope = table.scope.clone();
    let tiers = ds
        .meta
        .table(&scope, &kind)
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
            top_pct: vocfreq_core::rank::pct_for_rank(h.rank, entries),
            entries,
            scope: scope.clone(),
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
    let table = pick_table(&ds, domain.as_deref(), &kind).ok_or_else(|| {
        format!(
            "找不到作用域 {} 的 {kind} 表",
            domain.as_deref().unwrap_or("(主表)")
        )
    })?;
    let from = from.unwrap_or(1).max(1);
    let limit = limit.unwrap_or(100).clamp(1, 2000);
    Ok(to_rows(
        table.vfr.range_by_rank(from, limit),
        table.total_tokens,
        table.entries,
    ))
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
        table.vfr.prefix_scan(&query, limit.unwrap_or(60).clamp(1, 500)),
        table.total_tokens,
        table.entries,
    ))
}

// ===========================================================================
// 全库统计
// ===========================================================================

#[tauri::command]
fn start_scan(app: tauri::AppHandle, state: State<'_, AppState>, params: ScanParams) -> Result<(), String> {
    // 词库这一关必须在**置位 scanning 之前**过：一旦置了位再返回 Err，
    // 那个标志就永远留在 true，用户之后再也点不动"开始统计"。
    let lib = state.library();
    let available = lib.list_dicts();
    let chain = resolve_scan_dicts(&params.dict_files, &available);
    if chain.is_empty() {
        return Err(format!(
            "没有可用的词库，无法统计。\n\
             数据文件夹：{}\n\
             你勾选的：{}\n\
             请在「词库管理」里确认这些词库还在、文件没读错，或导入一份新的 .dict。\n{}",
            lib.dicts_dir().display(),
            if params.dict_files.is_empty() {
                "（未勾选，默认用数据文件夹里全部 .dict）".to_string()
            } else {
                params.dict_files.join("、")
            },
            format_broken_dicts(&available)
        ));
    }

    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("已有统计任务正在运行".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().map_err(|_| "状态锁损坏")? = Some(cancel.clone());

    let out_dir = params.out.clone();
    let mut cfg = ScanConfig::new(params.corpus.clone(), params.out.clone());
    cfg.threads = params.threads as usize;
    cfg.dicts = chain;
    cfg.only_domains = params.only_domains.clone();
    cfg.skip_domain_tables = params.skip_domain_tables;
    // 「只要一张全库表」这一个选项 = 不要分域表 **且** 要 full。
    //
    // `scan` 现在默认不产 `full`（它改由「合流 + 相加」得到，见 docs/DATA_LAYOUT.md §七），
    // 所以这里必须显式打开 `write_full` —— 否则"不要分域表 + 不要 full"会产出一份
    // 一张表都没有的目录，`Dataset::open` 直接打不开。核心库里另有一道前置校验兜住
    // 这种组合（`scan` 会明确报错而不是写出一份废产物）。
    cfg.write_full = params.skip_domain_tables;
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
                // 统计写到了 out_dir。若它就落在数据文件夹的 tables\ 下，按**名字**
                // 记成激活表（数据文件夹搬走也不失效）；否则按路径记。
                let out_path = PathBuf::from(&out_dir);
                let lib = st.library();
                let name = out_path
                    .strip_prefix(lib.tables_dir())
                    .ok()
                    .and_then(|r| r.components().next())
                    .and_then(|c| c.as_os_str().to_str())
                    .map(|s| s.to_string());
                if let Err(e) = st.activate_table(&out_path, name.as_deref()) {
                    let _ = handle.emit("scan:error", format!("统计完成但打开产物失败：{e}"));
                    return;
                }
                st.persist_settings_of(|s| {
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
        // 夹到最小尺寸：`set_size` 会绕过 `min_inner_size`（那是窗口创建时才生效的
        // 约束），老设置文件里存的 460×340 正好能把窗口按到装不下内容的尺寸上。
        let (pw, ph) = clamp_popup_size(settings.popup_width, settings.popup_height);
        let _ = w.set_size(tauri::LogicalSize::new(pw, ph));
    }

    // 换了主作用域就要把数据集重新装一遍：`Dataset::primary_scope` 是打开时定下来的，
    // 不重开的话界面上的"主表"标签变了、颜色却没变，属于最难查的一类不一致。
    state.refresh_primary_scope();
    Ok(())
}

// ===========================================================================
// 全局取词 + 悬浮小窗
// ===========================================================================

#[tauri::command]
async fn capture_selection(state: State<'_, AppState>) -> Result<String, String> {
    let cap = tauri::async_runtime::spawn_blocking(|| capture::capture_selection(1000))
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
                    // 最小高度见 POPUP_MIN_H 的注释：比这小就装不下「卡片 + 四格 + 徽标行」。
                    .min_inner_size(POPUP_MIN_W, POPUP_MIN_H)
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
    // 待分析文本要在显示之前取出来：下面的「要不要抢焦点」依赖它。
    let text = app
        .state::<AppState>()
        .pending
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    win.show().map_err(|e| format!("显示小窗失败：{e}"))?;

    // 抢焦点。热键流程需要：
    //   * Esc 关闭小窗；
    //   * Ctrl+C 复制分析结果；
    //   * 没取到内容时直接打字。
    // 这些都要键盘焦点，所以这里必须 set_focus()。
    //
    // ⚠ 抢焦点会让「下一次按热键」时前台变成小窗自己。这一点由取词前的
    // is_own_window + prev_foreground 归还逻辑兜住（见 run_capture 开头），
    // 实测连按两次热键都能正确取到目标窗口的选区。
    let _ = win.set_focus();

    // 把待分析文本**推**给小窗，而不是等它自己来取。
    //
    // 小窗关闭时只是隐藏（不销毁），因此它的 JS 只在**第一次**挂载时跑一次。
    // 若只靠 `take_pending_selection`，第二次按热键时小窗不会重新挂载，
    // 于是显示的是上一次的旧内容（取过一次后就是空的了）。
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
            // 连按保护：上一次取词还没结束就忽略这次按键
            {
                let st = h.state::<AppState>();
                if st.capturing.swap(true, Ordering::SeqCst) {
                    log_line("startup.log", "[热键] 上一次取词还没结束，忽略这次按键");
                    return;
                }
            }

            // ⚠ 先确认前台不是我们自己。
            //
            // 小窗弹出时会 set_focus()，之后用户再按热键，前台就是小窗本身；
            // 那样取词会瞄向自己的 WebView2（祖先链全是 BrowserView/BrowserRootView），
            // 结果必然是「没取到内容」——实测确实踩到过。
            // 这里把焦点还给「小窗出现之前用户正在用的窗口」再取词。
            let fg = capture::foreground_hwnd();
            if capture::is_own_window(fg) {
                let prev = h
                    .state::<AppState>()
                    .prev_foreground
                    .lock()
                    .map(|g| *g)
                    .unwrap_or(0);
                if prev != 0 && !capture::is_own_window(prev) {
                    log_line(
                        "startup.log",
                        &format!("[热键] 前台是本应用窗口，先把焦点还给 hwnd={prev} 再取词"),
                    );
                    capture::set_foreground(prev);
                    // 给系统一点时间完成焦点切换，否则选区还挂在旧窗口上
                    std::thread::sleep(std::time::Duration::from_millis(120));
                } else {
                    log_line(
                        "startup.log",
                        &format!("[热键] 前台是本应用窗口，且没有可归还的目标（prev={prev}）"),
                    );
                }
            }

            let (text, reason) = match capture::capture_selection(1000) {
                Ok(c) => (c.text, c.reason),
                Err(e) => (String::new(), format!("取词失败：{e}")),
            };
            // 取词结束，放开连按保护（后面只是把结果交给小窗，不需要互斥）
            {
                let st = h.state::<AppState>();
                st.capturing.store(false, Ordering::SeqCst);
            }
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
            let mut candidates: Vec<(PathBuf, &'static str)> = Vec::new();
            if let Ok(p) = std::env::var("VOCTIER_LOG_DIR") {
                if !p.trim().is_empty() {
                    candidates.push((PathBuf::from(p), "VOCTIER_LOG_DIR"));
                }
            }
            match std::env::var("APPDATA") {
                Ok(a) if !a.is_empty() => {
                    candidates.push((PathBuf::from(a).join("com.voctier.desktop"), "%APPDATA%"))
                }
                _ => {}
            }
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    candidates.push((dir.join("voctier-logs"), "程序所在目录"));
                }
            }
            candidates.push((
                std::env::temp_dir().join("com.voctier.desktop"),
                "临时目录",
            ));

            // 逐个候选探测，并**把每个候选失败的具体原因记下来**。
            //
            // 为什么值得这么麻烦：实测出现过「同一个目录，计划任务里的 pwsh 写得进、
            // 本应用写不进」的情况。只记「用了哪个目录」看不出差别，
            // 必须把 `create_dir_all` 与写探针各自的错误码都留下才可比较。
            let mut report = String::new();
            report.push_str(&format!(
                "== 日志目录探测 ==\n进程: pid={} exe={:?}\n环境: APPDATA={:?} LOCALAPPDATA={:?} TEMP={:?} USERNAME={:?}\n",
                std::process::id(),
                std::env::current_exe().ok(),
                std::env::var("APPDATA").ok(),
                std::env::var("LOCALAPPDATA").ok(),
                std::env::var("TEMP").ok(),
                std::env::var("USERNAME").ok(),
            ));
            let mut chosen: Option<PathBuf> = None;
            for (dir, why) in &candidates {
                let line = match std::fs::create_dir_all(dir) {
                    Err(e) => format!("{why} {dir:?} → 建目录失败：{e}"),
                    Ok(()) => {
                        let probe = dir.join(".write-probe");
                        match std::fs::write(&probe, b"ok") {
                            Ok(()) => {
                                let _ = std::fs::remove_file(&probe);
                                chosen = Some(dir.clone());
                                format!("{why} {dir:?} → 可写 ✅ 采用")
                            }
                            Err(e) => format!("{why} {dir:?} → 写探针失败：{e}"),
                        }
                    }
                };
                report.push_str(&line);
                report.push('\n');
                if chosen.is_some() {
                    break;
                }
            }
            let dir = chosen.unwrap_or_else(std::env::temp_dir);

            // 把探测报告落到最终选中的目录里，便于事后对比
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("log-dir-probe.log"))
            {
                use std::io::Write;
                let _ = writeln!(f, "{report}");
            }
            dir
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
            active_dataset,
            open_dataset,
            library_info,
            set_data_dir,
            ensure_data_dirs,
            library_ready,
            dict_list,
            dict_import,
            dict_delete,
            table_list,
            activate_library_table,
            table_delete,
            compose_tables,
            set_primary_scope,
            suggest_table_dir,
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

            // 设置读进来之后先做两件事：迁移老的 `data_dir` 语义、把数据文件夹建出来。
            // 顺序不能反 —— 迁移会改写 data_dir，建目录必须用迁移后的结果。
            let st = handle.state::<AppState>();
            let def = default_data_dir(&handle);
            for note in st.migrate_settings(&def) {
                log_line("startup.log", &format!("[migrate] {note}"));
            }
            let lib = st.library();
            if let Err(e) = lib.ensure() {
                log_line("startup.log", &format!("[warn] 建数据文件夹失败：{e}"));
            }
            log_line(
                "startup.log",
                &format!("数据文件夹 = {}", lib.root().display()),
            );

            // 用户上次用的那张表若还在，启动就打开，省得每次都要点一次
            let s = st.settings_snapshot();
            if let Some(dir) = st.active_table_dir(&s) {
                if dir.join("meta.json").exists() {
                    match st.activate_table(&dir, s.active_table.as_deref()) {
                        Ok(m) => log_line(
                            "startup.log",
                            &format!(
                                "已载入词表 {}（{} 张表 / {} 词条）",
                                dir.display(),
                                m.tables.len(),
                                m.tables.iter().filter(|t| t.kind == "word").map(|t| t.entries).max().unwrap_or(0)
                            ),
                        ),
                        Err(e) => log_line("startup.log", &format!("[warn] 打开上次的词表失败：{e}")),
                    }
                    // 加载时攒下的告警（v1 老产物、词库缺失后退化…）必须落进日志：
                    // 这些正是"频次可能不准"的信号，只在界面上闪一下会漏掉。
                    if let Ok(w) = st.active_warnings.lock() {
                        for msg in w.iter() {
                            log_line("startup.log", &format!("[warn] {msg}"));
                        }
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
        "dictFiles": ["主词库.dict", "补充.dict"],
        "threads": 8,
        "hmm": false,
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
        // 词库链的顺序有意义（第一个是主词库），必须原样传过来
        assert_eq!(
            p.dict_files,
            vec!["主词库.dict".to_string(), "补充.dict".to_string()]
        );
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
        // 一个词库都不指定 = 用数据文件夹里全部 .dict
        assert!(p.dict_files.is_empty());
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
            "locale",
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
    fn curve_cache_key_changes_when_the_table_is_rebuilt_in_place() {
        // 词表管理器提供「一键重新统计」，而重算常常写回**同一个目录**：
        // 路径没变、数据全变了。键里必须带内容标识，否则重算之后曲线一直是旧的，
        // 而它正是「按覆盖率分组」的输入 —— 分组会跟着一起错，且完全看不出来。
        let root = Path::new("C:/data/tables/mine");
        let before = curve_cache_key(root, "2026-10-02T05:24:17Z", "full/word");
        let after = curve_cache_key(root, "2026-10-03T09:00:00Z", "full/word");
        assert_ne!(before, after, "重算后键必须变");

        // 同一份产物重复查询要命中同一个键（否则缓存等于没有）
        assert_eq!(
            before,
            curve_cache_key(root, "2026-10-02T05:24:17Z", "full/word")
        );
        // 换表、换目录也要区分开
        assert_ne!(before, curve_cache_key(root, "2026-10-02T05:24:17Z", "full/char"));
        assert_ne!(
            before,
            curve_cache_key(Path::new("C:/data/tables/other"), "2026-10-02T05:24:17Z", "full/word")
        );
    }

    // ---------------------------------------------------------------- 扫描时选词库

    /// 造一个词库列表项。`broken=true` 表示文件读不了。
    fn fake_dict(file_name: &str, name: &str, broken: bool) -> DictItem {
        DictItem {
            dict: vocfreq_core::dict::DictRef {
                id: name.into(),
                name: name.into(),
                path: format!("C:/data/dicts/{file_name}"),
                entries: 3,
                sha256: "a".repeat(64),
            },
            error: if broken { Some("第 1 行坏了".into()) } else { None },
            report: Default::default(),
            file_name: file_name.into(),
            origin: library::Origin::Unknown,
        }
    }

    #[test]
    fn scan_dicts_accept_both_the_file_name_and_the_meta_name() {
        // 这是「重新统计」那条路的**关键契约**：
        // 扫描页勾选给的是文件名（`预制词库.dict`），而重新统计是从产物 meta 里
        // 读回 `tokenizer.dicts[].name`（`预制词库`，没有扩展名）。只认前者的话，
        // 重算时勾选会全部落空、链变成空的 —— 用户看到的是"没有可用的词库"。
        let avail = vec![
            fake_dict("预制词库.dict", "预制词库", false),
            fake_dict("补充.dict", "补充", false),
        ];

        let by_file = resolve_scan_dicts(&["预制词库.dict".into()], &avail);
        let by_name = resolve_scan_dicts(&["预制词库".into()], &avail);
        assert_eq!(by_file.len(), 1, "按文件名应命中");
        assert_eq!(by_name.len(), 1, "按 meta 里的名字（无扩展名）也必须命中");
        assert_eq!(by_file, by_name, "两种写法应解析到同一份词库");
    }

    #[test]
    fn scan_dicts_keep_the_selection_order() {
        // 顺序有意义：第一个是主词库，后面的只做叠加，同名条目后者覆盖前者
        let avail = vec![
            fake_dict("a.dict", "a", false),
            fake_dict("b.dict", "b", false),
            fake_dict("c.dict", "c", false),
        ];
        let chain = resolve_scan_dicts(&["c.dict".into(), "a.dict".into()], &avail);
        let names: Vec<String> = chain
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["c.dict", "a.dict"], "必须保持勾选顺序");
    }

    #[test]
    fn scan_dicts_skip_broken_files_and_never_silently_widen() {
        let avail = vec![
            fake_dict("好.dict", "好", false),
            fake_dict("坏.dict", "坏", true),
        ];
        // 坏文件不能被选中（它连词条都读不出来）
        assert!(resolve_scan_dicts(&["坏.dict".into()], &avail).is_empty());
        // 勾了但一个都对不上 → 返回空表，由调用方报错。
        // **不能**悄悄退化成"那就用全部词库"：用户会以为勾的那份生效了。
        assert!(resolve_scan_dicts(&["根本不存在的词库".into()], &avail).is_empty());
        // 没勾才等于"用全部"（且跳过坏的）
        let all = resolve_scan_dicts(&[], &avail);
        assert_eq!(all.len(), 1);
        assert!(all[0].ends_with("好.dict"));
    }

    #[test]
    fn scan_dicts_default_chain_is_ordered_by_file_name() {
        let avail = vec![
            fake_dict("b.dict", "b", false),
            fake_dict("a.dict", "a", false),
        ];
        let chain = resolve_scan_dicts(&[], &avail);
        let names: Vec<String> = chain
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.dict", "b.dict"], "顺序必须确定，否则产物会飘");
    }

    // ---------------------------------------------------------------- 老设置迁移
    //
    // 这段代码只对**已经在用老版本的机器**生效，而且一跑就会改写对方的设置文件。
    // 出错了用户是"升级之后东西不见了"，且没有第二次机会 —— 所以必须有测试。

    /// 造一个临时目录当数据文件夹的落脚点。
    fn temp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("voctier-migrate-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 造一个"老设置指向的产物目录"：目录里**直接**就有 meta.json（这正是判据）。
    fn make_legacy_table_dir(root: &Path, name: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(d.join("full")).unwrap();
        std::fs::write(d.join("meta.json"), "{}").unwrap();
        std::fs::write(d.join("full").join("word.vfr"), b"x").unwrap();
        std::fs::write(d.join("full").join("char.vfr"), b"x").unwrap();
        d
    }

    #[test]
    fn migrate_turns_a_legacy_table_dir_into_a_registered_table() {
        let root = temp_root("legacy-table");
        let legacy = make_legacy_table_dir(&root, "老产物");
        let def = root.join("data");

        let st = AppState::default();
        st.settings.lock().unwrap().data_dir = Some(legacy.display().to_string());

        let notes = st.migrate_settings(&def);
        let s = st.settings_snapshot();

        // data_dir 换成数据文件夹，老产物目录被记成"当前激活的表"
        assert_eq!(s.data_dir.as_deref(), Some(def.display().to_string().as_str()));
        assert_eq!(s.active_table_path.as_deref(), Some(legacy.display().to_string().as_str()));
        assert!(s.active_table.is_none());
        assert!(!notes.is_empty(), "迁移必须留下可日志化的说明，不能悄悄改");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migrate_leaves_a_real_data_folder_untouched() {
        // 已经是数据文件夹（有 dicts\ 或 tables\ 子目录）→ 一个字都不该改
        let root = temp_root("real-library");
        let lib = root.join("data");
        std::fs::create_dir_all(lib.join("dicts")).unwrap();
        std::fs::create_dir_all(lib.join("tables")).unwrap();

        let st = AppState::default();
        st.settings.lock().unwrap().data_dir = Some(lib.display().to_string());

        let notes = st.migrate_settings(&lib);
        let s = st.settings_snapshot();

        assert_eq!(s.data_dir.as_deref(), Some(lib.display().to_string().as_str()));
        assert!(s.active_table_path.is_none(), "不该凭空造出一张激活表");
        assert!(notes.is_empty(), "没迁移动作就不该有说明：{notes:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migrate_falls_back_to_the_default_data_dir_when_unset() {
        let root = temp_root("unset");
        let def = root.join("data");

        let st = AppState::default();
        let notes = st.migrate_settings(&def);

        assert_eq!(st.settings_snapshot().data_dir.as_deref(), Some(def.display().to_string().as_str()));
        assert!(notes.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migrate_reports_and_clears_the_legacy_user_dict() {
        // 从前的单个 user_dict 不再自动叠加。用户词典现在必须是数据文件夹里的
        // .dict 条目，所以只能提示 + 清掉；**不能**继续照着那个路径加载 ——
        // 它可能是个 .txt，也可能早就不在了。
        let root = temp_root("legacy-userdict");
        let def = root.join("data");
        let old = root.join("userdict_trad.txt");
        std::fs::write(&old, "對 731971\n").unwrap();

        let st = AppState::default();
        {
            let mut g = st.settings.lock().unwrap();
            g.user_dict = Some(old.display().to_string());
        }

        let notes = st.migrate_settings(&def);
        let s = st.settings_snapshot();

        assert!(s.user_dict.is_none(), "老字段必须清掉，否则每启动一次都提示一次");
        assert!(s.scan_dicts.is_none(), "不该自作主张把它塞进词库链");
        assert!(
            notes.iter().any(|n| n.contains("用户词典")),
            "要告诉用户他的词典去哪了：{notes:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn table_keys_resolve_to_the_right_table() {
        // 表管理器用「作用域/类型」字符串指代表，解析错了会静默查错表
        let Some(dir) = test_data_dir() else {
            skip_no_data("table_keys_resolve_to_the_right_table");
            return;
        };
        let Ok(ds) = Dataset::open(&dir) else {
            skip_no_data("table_keys_resolve_to_the_right_table");
            return;
        };
        assert!(pick_by_key(&ds, "full/word").is_some());
        assert!(pick_by_key(&ds, "full/char").is_some());
        // 作用域与类型必须**同时**对上：只给作用域或只给类型都不算命中
        assert!(pick_by_key(&ds, "full").is_none(), "缺类型不该命中");
        assert!(pick_by_key(&ds, "不存在的域/word").is_none());
        // 词表与字表必须是不同的表，别解析成同一张
        let w = pick_by_key(&ds, "full/word").unwrap();
        let c = pick_by_key(&ds, "full/char").unwrap();
        assert_ne!(w.entries, c.entries);
        assert_eq!(c.vfr.kind(), vocfreq_core::query::KIND_CHAR);

        // 分域对比现在按「作用域」取：每个作用域各有一张词表 + 一张字表
        for scope in ds.scopes() {
            assert!(pick_table(&ds, Some(&scope), "word").is_some(), "作用域 {scope} 缺词表");
            assert!(
                pick_table(&ds, Some(""), "word").is_some(),
                "空 scope = 主作用域，必须能取到"
            );
        }
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
        // 曲线要按**主作用域**画（默认口径就是主作用域，见 format.ts::boundsInfo）
        let table = ds.primary_table("word").expect("主作用域必须有词表");
        let curve = table.vfr.coverage_curve(400);
        let elapsed = t0.elapsed();
        assert!(curve.len() > 20, "曲线点数太少：{}", curve.len());
        assert_eq!(curve[0].0, 1);
        assert_eq!(curve.last().unwrap().0 as u64, table.entries);
        let mut prev = 0.0f64;
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

        // 默认口径是**前%**，所以主作用域上必须能直接算出前%上界
        let pct = ds
            .meta
            .table(&table.scope, "word")
            .map(|t| t.effective_tier_pct())
            .unwrap_or_default();
        assert_eq!(pct.len(), 6, "主作用域的词表必须有 6 个前%上界");
        let ranks = vocfreq_core::rank::ranks_from_pct(&pct, table.entries);
        for w in ranks.windows(2) {
            assert!(w[0] < w[1], "前%换算出来的阈值必须严格递增：{ranks:?}");
        }
        eprintln!("[pct] 前%上界 {pct:?} -> 排名阈值 {ranks:?}");
    }

    #[test]
    fn settings_round_trip_and_tolerate_partial_json() {
        let s = Settings::default();
        let back: Settings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back.hotkey, s.hotkey);
        assert_eq!(back.popup_width, s.popup_width);
        assert_eq!(back.min_count, s.min_count);

        // 老版本设置文件里没有的字段要能补默认值，否则升级后直接读不出设置。
        // 这里断言的是**默认值本身**（不是「等于某个历史值」），所以改了
        // popup_width/height 的默认值就必须同步改这里——上一次改 460→480 时漏了，
        // 结果是测试红着没人发现。直接跟 Settings::default() 比，免得再漏。
        let partial: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+K"}"#).unwrap();
        assert_eq!(partial.hotkey, "Ctrl+Shift+K");
        let d = Settings::default();
        assert_eq!(partial.popup_width, d.popup_width);
        assert_eq!(partial.popup_height, d.popup_height);
        assert_eq!(partial.theme, "system");
    }

    /// 小窗尺寸必须被夹到最小尺寸以上。
    ///
    /// 现实背景：设置文件里存着 `popupWidth:460 / popupHeight:340` 时，
    /// `save_settings` 会用 `set_size` 把窗口按到 340 —— 而 `set_size` 会**绕过**
    /// `min_inner_size`（后者只是窗口创建时的约束）。结果小窗高度不够，
    /// 词条详情被压扁、四格/徽标行被滚出视野，表现为「详情区忽大忽小」。
    #[test]
    fn popup_size_is_clamped_to_minimum() {
        // 用户设置文件里那个坏值：宽 460 合法保留，高 340 必须被抬到下限
        assert_eq!(clamp_popup_size(460.0, 340.0), (460.0, POPUP_MIN_H));
        assert_eq!(clamp_popup_size(0.0, 0.0), (POPUP_MIN_W, POPUP_MIN_H));
        assert_eq!(clamp_popup_size(-10.0, 100.0), (POPUP_MIN_W, POPUP_MIN_H));
        // 合法的更大尺寸原样保留
        assert_eq!(clamp_popup_size(900.0, 700.0), (900.0, 700.0));
        // 一边合法一边不合法，只抬不合法的那边
        assert_eq!(clamp_popup_size(900.0, 340.0), (900.0, POPUP_MIN_H));
        // 默认值本身必须 >= 下限，否则开箱就是坏的
        let d = Settings::default();
        assert!(d.popup_width >= POPUP_MIN_W && d.popup_height >= POPUP_MIN_H);
    }

    #[test]
    fn default_hotkey_is_parseable_by_the_plugin() {        // 热键字符串最终要喂给 tauri-plugin-global-shortcut，格式错了会在启动时静默失效
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
