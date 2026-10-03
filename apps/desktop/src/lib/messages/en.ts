import type { Messages } from './types';

/**
 * English UI copy. 由 zh-CN.ts 的 key 集合约束：漏一条就编译不过。
 *
 * 语气：面向「研究中文文本的人」的工具，简洁、专业，不搞营销腔。
 * 占位符（{count} / {dir} / {error} …）保留原名，不翻译、不改名。
 * 技术标识（token / meta.json / .vfr / .tsv / HMM / TSV / jieba-rs / VocTier /
 * schema v1）保持原样；「—」（无数据占位）不在消息表里。
 */
export const en: Messages = {
  // ---------------------------------------------------------------- 应用
  'app.title': 'VocTier — Chinese Word Frequency Analysis',
  // 品牌名不翻译（故不设 'app.name'）。

  // ---------------------------------------------------------------- 跨页面共用
  'common.browse': 'Browse…',
  'common.clear': 'Clear',
  'common.choose': 'Choose…',
  'common.cancel': 'Cancel',
  /** 并列项连接符：英文用逗号+空格 */
  'common.listSeparator': ', ',
  'common.on': 'On',
  'common.off': 'Off',
  'common.keep': 'Keep',
  'common.discard': 'Discard',
  'common.yes': 'Yes',
  'common.no': 'No',

  // ---------------------------------------------------------------- 导航
  'nav.group.analyze.label': 'Analyze',
  'nav.group.data.label': 'Data',
  'nav.group.system.label': 'System',
  'nav.wordfreq.label': 'Generate Frequency Tables',
  'nav.wordfreq.title': 'Generate Frequency Tables',
  'nav.wordfreq.description': 'Import a corpus directory, count word/character frequency, and export result tables.',
  'nav.sentences.label': 'Sentence Analysis',
  'nav.sentences.title': 'Sentence Analysis',
  'nav.sentences.description': 'Split text into sentences and inspect per-sentence word distribution and difficulty.',
  'nav.leaderboard.label': 'Leaderboard',
  'nav.leaderboard.title': 'Leaderboard',
  'nav.leaderboard.description': 'Rank words/characters by frequency, coverage, length, and more.',
  'nav.dicts.label': 'Dictionaries',
  'nav.dicts.title': 'Dictionaries',
  'nav.dicts.description':
    'Manage the dictionaries in the data folder (jieba-format .dict files): import, delete, and see every pitfall.',
  'nav.tables.label': 'Table Manager',
  'nav.tables.title': 'Table Manager',
  'nav.tables.description': 'Toggle which tables take part in domain comparison and the leaderboard, and customize the seven tier thresholds.',
  'nav.settings.label': 'Settings',
  'nav.settings.title': 'Settings',
  'nav.settings.description': 'Tokenization parameters, output format, theme, and interface preferences.',

  // ---------------------------------------------------------------- 主题与外观
  'theme.light': 'Light',
  'theme.dark': 'Dark',
  'theme.system': 'System',
  'theme.toggleHint': '{current} (click to switch to {next})',

  // ---------------------------------------------------------------- 设置页
  'settings.browserPreview':
    'Browser preview mode: settings can be edited and “saved”, but are not written to disk (kept in memory only).',
  'settings.loading': 'Loading settings…',
  'settings.readFailed': 'Failed to read settings: {error}',
  'settings.saveFailed': 'Save failed: {error}',
  'settings.saved': 'Settings saved.',
  'settings.resetHint': 'Defaults restored — remember to click “Save Settings”.',

  // 全局取词
  'settings.hotkey.title': 'Global Capture',
  'settings.hotkey.valid': 'Valid format',
  'settings.hotkey.invalid': 'Needs fixing',
  'settings.hotkey.description':
    'Pressing the hotkey simulates Ctrl+C to grab the selected text and run frequency analysis in the popup window.',
  'settings.hotkey.label': 'Global hotkey',
  // 示例键名（Alt+Q、Ctrl+Shift+Q）不翻译。
  'settings.hotkey.hintPrefix': 'Format: modifier + key, e.g. ',
  'settings.hotkey.hintSeparator': ', ',
  'settings.hotkey.hintSuffix': '.',
  'settings.testCapture': 'Test capture',
  'settings.testCaptureHint':
    'The test simulates one copy in the current foreground app and writes the result to the clipboard.',
  'settings.captureNoText': 'No selected text detected. Select some text in another program first and try again.',
  'settings.captureOkCopied': 'Captured successfully ({count} chars) and copied to the clipboard.',
  'settings.captureOkText': 'Captured: {text}',

  // 管理员权限说明。`p1Before` 结尾的空格对应原模板里 HTML 折叠出的空格，保留。
  'settings.adminNote.title': 'About “Run as administrator”',
  'settings.adminNote.p1Before':
    'Global capture works by sending Ctrl+C to the focused window. Windows UIPI (User Interface Privilege Isolation) requires: ',
  'settings.adminNote.p1Emphasis':
    'a lower-integrity process cannot send input to a higher-integrity process',
  'settings.adminNote.p1After':
    '. So when the target app (for example an editor, IDE, or terminal running as administrator) has higher privileges than VocTier, hotkey capture is silently blocked by the system — it looks like “nothing happens when you press it”.',
  'settings.adminNote.p2':
    'Fix: right-click the VocTier shortcut → “Run as administrator” so both run at the same privilege level (both elevated, or both not). This restriction comes from Windows itself and the app cannot bypass it.',

  // 悬浮小窗
  'settings.popup.title': 'Popup Window',
  'settings.popup.description': 'Size, opacity, and auto-close behavior of the popup shown after capture.',
  'settings.popup.width': 'Width (px)',
  'settings.popup.height': 'Height (px)',
  'settings.popup.autoClose': 'Auto-close (ms, 0 = never)',
  'settings.popup.minSize': 'The popup is at least 200 × 150 px.',
  'settings.popup.opacity': 'Opacity',
  'settings.popup.opacityLabel': 'Popup opacity',
  'settings.popup.alwaysOnTop': 'Always on top',
  'settings.popup.alwaysOnTopHint': 'Keep the popup above other windows',

  // 外观与界面语言
  'settings.appearance.title': 'Appearance',
  'settings.appearance.description': 'Shares state with the theme button in the top bar.',
  'settings.appearance.currentTheme': 'Currently active: {theme}',
  'settings.language.title': 'Language',
  'settings.language.description':
    'Switch the interface language. Statistics and tokenization are unaffected.',
  'settings.language.onlyOne':
    'Only Simplified Chinese is available right now; other languages will appear here once their translations are complete.',

  // 默认统计参数
  'settings.tokenize.title': 'Default Statistics',
  'settings.tokenize.description':
    'The “Generate Frequency Tables” page uses these values as its initial settings.',
  'settings.tokenize.threads': 'Default threads (0 = auto)',
  'settings.tokenize.minCount': 'Minimum frequency',
  'settings.tokenize.hmm': 'HMM new-word discovery',
  'settings.tokenize.hmmHint': 'Find contiguous hanzi combos not in the dictionary',
  'settings.tokenize.keepDigit': 'Keep number tokens',
  'settings.tokenize.keepDigitHint': 'e.g. 2024, 3.14',
  'settings.tokenize.keepLatin': 'Keep Latin tokens',
  'settings.tokenize.keepLatinHint': 'e.g. API, token',
  'settings.tokenize.skipSingleChar': 'Skip single-character words',
  'settings.tokenize.skipSingleCharHint':
    'Count multi-character words only (the char table is still produced)',
  'settings.tokenize.domainTables': 'Produce domain tables by default',
  'settings.tokenize.domainTablesHint':
    'When off, only the full-corpus word/char tables are produced — smaller output',

  // 默认目录与词典
  'settings.paths.title': 'Directories',
  'settings.paths.description':
    'Sentence Analysis reads the tables under tables\\ in the “data folder”; dictionaries live in dicts\\ of the same folder.',
  'settings.paths.dataDirBadge': 'Data folder',
  'settings.paths.corpusDir': 'Default corpus directory',
  'settings.paths.dataDir': 'Data folder',
  'settings.paths.dataDirHint':
    'It holds dicts\\ (the dictionary library) and tables\\ (the table library). Leave empty to use the system default location.',
  'settings.paths.openDataDir': 'Open data folder',
  'settings.paths.userDictDeprecatedTitle': 'The “custom dictionary” setting is deprecated',
  'settings.paths.userDictDeprecatedBody':
    'A dictionary is no longer an extra file path but a plain entry in the data folder’s dicts\\ — pick them in order on the “Generate Frequency Tables” page (the first is the primary dictionary), or import/delete them on the “Dictionaries” page.',
  'settings.paths.userDictWillMigrate':
    'The old value {path} will be migrated into the scan dictionary chain on next start-up; after that this field is never written again.',
  'settings.paths.unset': 'Not set',
  'settings.pickCorpusDir': 'Choose default corpus directory',
  'settings.pickDataDir': 'Choose data folder',

  // 保存与关于
  'settings.save.saving': 'Saving…',
  'settings.save.save': 'Save Settings',
  'settings.save.reset': 'Restore defaults',
  'settings.save.dirty': 'Unsaved changes',
  'settings.save.hotkeyInvalid': 'Invalid hotkey format. Use something like Alt+Q.',
  'settings.about.title': 'About',
  'settings.about.description': 'Version info comes from the app_info command.',
  'settings.about.app': 'App',
  'settings.about.appVersion': 'App version',
  'settings.about.infoUnavailable': 'Version info is currently unavailable.',
  'settings.about.infoUnavailableWithError': 'Version info is currently unavailable: {error}.',
  'settings.about.footnote':
    'Hotkeys, directories, and popup appearance take effect after saving; hotkey re-registration is handled by the Rust side.',

  // ---------------------------------------------------------------- 生成词频表页
  'wordfreq.browserPreview':
    'Browser preview mode: using built-in demo data; the full flow works end to end (the directory picker is unavailable — edit the path input directly).',
  'wordfreq.loading': 'Loading existing settings…',
  // 页首那句「当前打开的是哪张表」——统计完成后会自动刷新
  'wordfreq.currentTable.label': 'Current table: ',
  'wordfreq.currentTable.meta': 'generated {generated} · {tables} tables',
  'wordfreq.currentTable.none':
    'No table is open yet. Finish a statistics pass below, or activate an existing table on the “Table Manager” page.',
  'wordfreq.currentTable.manage': 'Table Manager',

  // 步骤条
  'wordfreq.step.corpus': 'Choose corpus',
  'wordfreq.step.params': 'Configure parameters',
  'wordfreq.step.run': 'Output & run',

  // ① 语料库
  'wordfreq.step1.title': '① Choose the corpus directory',
  'wordfreq.step1.description':
    'Enter or pick the corpus root; “Probe” first to detect domains, file counts, and parse rules.',
  'wordfreq.step1.corpusPlaceholder': 'e.g. D:\\corpus or /data/corpus',
  'wordfreq.step1.corpusLabel': 'Corpus directory',
  'wordfreq.step1.probe': 'Probe',
  'wordfreq.step1.probing': 'Probing…',
  // 这些句子里有加粗的值，渲染处用 splitMessage 按占位符切分。
  'wordfreq.plan.domains': 'Detected {count} domains',
  'wordfreq.plan.files': '{count} files',
  'wordfreq.plan.bytes': '{value} size',
  'wordfreq.plan.onlyThisDomain': 'Only {domain}',
  /** `{field}` 是等宽显示的 ScanParams 字段名（不翻译） */
  'wordfreq.plan.ruleHint':
    'Checking domains = count only these (corresponds to {field}); checking none counts all domains.',
  'wordfreq.plan.noRule': '(No explicit rule matched; scanning by default extensions)',
  'wordfreq.plan.col.include': 'Include',
  'wordfreq.plan.col.domain': 'Domain',
  // 「文件」「体积」两列复用上面的 stat 文案。
  'wordfreq.plan.col.rules': 'Parse rule',

  // ② 统计参数
  'wordfreq.step2.title': '② Configure statistics',
  'wordfreq.step2.description':
    'These parameters are written into meta.json and saved as the defaults for the Settings page.',
  'wordfreq.step2.threads': 'Threads (0 = auto)',
  'wordfreq.step2.minCount': 'Minimum frequency',
  // 词典链：多选，**顺序有意义**（第一个是主词典）
  'wordfreq.step2.dictChain': 'Dictionary chain',
  'wordfreq.step2.dictChainBadge': 'Order matters',
  'wordfreq.step2.dictChainHint':
    'Check the dictionaries to use; **the first one checked is the primary dictionary**. Later ones are layered on top and win on duplicate entries. Checking none means “every .dict in dicts\\, sorted by file name”.',
  'wordfreq.step2.dictPrimary': 'Primary',
  'wordfreq.step2.dictOrder': 'Layer {index}',
  'wordfreq.step2.dictCheckAria': 'Use dictionary {name}',
  'wordfreq.step2.dictEffective': 'Loaded in this order: {names}',
  'wordfreq.step2.dictEffectiveAll': 'Not specified → all usable dictionaries (sorted by file name): {names}',
  'wordfreq.step2.dictUseAll': 'Use all dictionaries',
  'wordfreq.step2.dictsLoading': 'Loading the dictionary list…',
  'wordfreq.step2.noUsableDict':
    'The data folder has no usable dictionary. Dictionaries are no longer compiled into the app, so there is no built-in fallback — import a .dict on the “Dictionaries” page first, otherwise the scan produces a useless single-character table.',
  'wordfreq.step2.goDicts': 'Dictionaries',
  'wordfreq.step2.hmm': 'Enable HMM new-word discovery',
  'wordfreq.step2.hmmHint': 'Find contiguous hanzi combos not in the dictionary',
  'wordfreq.step2.hmmAria': 'Enable HMM',
  'wordfreq.step2.keepDigit': 'Keep number tokens',
  'wordfreq.step2.keepDigitHint': 'e.g. 2024, 3.14 as separate tokens',
  'wordfreq.step2.keepLatin': 'Keep Latin tokens',
  'wordfreq.step2.keepLatinHint': 'e.g. API, token as separate tokens',
  'wordfreq.step2.skipSingleChar': 'Skip single-character words',
  'wordfreq.step2.skipSingleCharHint':
    'Count only multi-character words in the word table (the char table is still produced)',
  'wordfreq.step2.domainTables': 'Produce domain tables',
  'wordfreq.step2.domainTablesHint': 'When off, only the full-corpus tables are produced — smaller output',
  'wordfreq.step2.writeTsv': 'Write readable TSV',
  'wordfreq.step2.writeTsvHint': 'Convenient for viewing in Excel / a text editor',
  'wordfreq.step2.writeTsvAria': 'Write TSV',

  // ③ 输出与执行
  'wordfreq.step3.title': '③ Table name & output directory',
  'wordfreq.step3.description':
    'Outputs (meta.json / *.vfr / *.tsv) are written to the output directory — by default <data folder>\\tables\\<table name>, but any location works.',
  'wordfreq.step3.tableName': 'Table name',
  'wordfreq.step3.tableNamePlaceholder': 'e.g. News corpus 2024',
  'wordfreq.step3.tableNameHint':
    'The table name determines the default output directory (<data folder>\\tables\\<table name>) and is the name shown in table management.',
  'wordfreq.step3.newTable': 'New frequency table',
  'wordfreq.step3.defaultTableName': '{corpus} table',
  'wordfreq.step3.resuggestOut': 'Re-fill from table name',
  'wordfreq.step3.outPlaceholder': 'e.g. <data folder>\\tables\\my-table',
  'wordfreq.step3.outLabel': 'Output directory',
  'wordfreq.step3.checkDataset': 'Check outputs',
  'wordfreq.step3.start': 'Start',
  'wordfreq.step3.scanning': 'Running…',
  'wordfreq.step3.cancel': 'Cancel',
  'wordfreq.step3.openOutDir': 'Open output directory',
  'wordfreq.libNotReady.title': 'No usable dictionary — cannot start.',
  'wordfreq.libNotReady.body':
    'Dictionaries are no longer compiled into the app: the data folder’s dicts\\ must contain at least one readable .dict, otherwise the scan produces a useless single-character table.',
  'wordfreq.libNotReady.go': 'Go to Dictionaries',
  'wordfreq.scanFailed': 'Statistics failed: {error}',
  'wordfreq.needCorpus': 'Enter or choose a corpus directory first.',
  'wordfreq.pickCorpusDir': 'Choose corpus directory',
  'wordfreq.pickOutDir': 'Choose output directory',
  'wordfreq.settingsSaved': 'Parameters saved to settings',
  'wordfreq.settingsSaveFailed': 'Failed to save parameters: {error}',
  'wordfreq.logSubmitted': 'Statistics task submitted: {corpus} → {out}',
  'wordfreq.logCancelled': 'Cancellation requested.',
  'wordfreq.logDone': 'Statistics complete; result summary generated.',
  'wordfreq.logOpenFailed': 'Failed to load outputs (other pages may not recognize them yet): {error}',
  'wordfreq.datasetUnavailable': 'Output directory unavailable: {dir}',
  'wordfreq.datasetLoaded': 'Loaded output directory: {dir} ({tables} tables)',
  'wordfreq.datasetLoadFailed': 'Output directory detected: {dir}, but loading failed: {error}',

  // 进度、计划与日志
  'wordfreq.stat.files': 'Files',
  'wordfreq.stat.bytes': 'Size',
  'wordfreq.stat.lines': 'Lines',
  'wordfreq.stat.paras': 'Paragraphs',
  'wordfreq.stat.badLines': 'Skipped lines',
  'wordfreq.progress.preparing': 'Preparing',
  'wordfreq.progress.domain': '· current domain {domain}',
  'wordfreq.progress.bytes': 'Processed {done} / {total}',
  'wordfreq.progress.units': 'Units {done} / {total}',
  'wordfreq.planEvent.title': 'Scan plan',
  'wordfreq.planEvent.summary': '{files} files / {bytes}, domains: ',
  'wordfreq.tables.title': 'Tables written',
  'wordfreq.tables.entries': '{entries} entries · {tokens} tokens',
  'wordfreq.tables.summary': '{entries} entries · {tokens} tokens · .vfr {bytes}',
  'wordfreq.logs.title': 'Logs',

  // 结果卡片
  'wordfreq.result.title': 'Statistics complete',
  'wordfreq.result.meta':
    'Generated {generated} · elapsed {elapsed} · tool {tool} · schema v{schema}',
  'wordfreq.result.engine': 'Tokenizer {engine} {version}',
  'wordfreq.result.hmm': 'HMM {value}',
  'wordfreq.result.digits': 'Number tokens {value}',
  'wordfreq.result.latin': 'Latin tokens {value}',
  'wordfreq.result.skipSingle': 'Skipped single-char {value}',
  'wordfreq.result.dicts': 'Dictionary chain {names}',
  'wordfreq.result.done':
    'Statistics complete ✅ Now go to the “{sentences}” page and paste text to see each token’s frequency tier, or browse the full ranking on the “{leaderboard}” page.',

  // ---------------------------------------------------------------- 划句分析页
  'sentences.browserPreview':
    'Browser preview mode: using built-in demo data; “Send to popup / manual capture” require the desktop app.',
  'sentences.checking': 'Checking corpus outputs…',
  'sentences.statusFailed': 'Failed to read dataset status: {error}',

  // 没有可用产物时的引导
  'sentences.noTable.title': 'No frequency tables yet',
  'sentences.noTable.description':
    'Sentence analysis depends on the meta.json and .vfr index tables produced by “Generate Frequency Tables”. Directory checked: ',
  'sentences.noTable.noDir': '(no output directory set)',
  'sentences.noTable.step1': 'Go to “Generate Frequency Tables”, choose a corpus, and start',
  'sentences.noTable.step2': 'Return here after it finishes — new outputs load automatically',
  'sentences.noTable.go': 'Go to Generate Frequency Tables',
  'sentences.noTable.goDicts': 'Go to Dictionaries',
  'sentences.noTable.recheck': 'Re-check',

  // 产物未装载到后端
  'sentences.notLoaded.title': 'Output directory not loaded into the backend',
  'sentences.notLoaded.p1WithError': 'meta.json found ({dir}), but `open_dataset` failed: {error}.',
  'sentences.notLoaded.p1NoError': 'meta.json found ({dir}), but `open_dataset` failed.',
  'sentences.notLoaded.p2':
    'Until that command is implemented on the Rust side, sentence analysis reports “not in corpus”.',

  // 数据集概览
  'sentences.dataset.title': 'Dataset',
  'sentences.dataset.ready': 'Ready',
  'sentences.dataset.hmm': 'HMM {value}',
  'sentences.dataset.dictChain': 'Dictionary chain ({count}): {names}',
  'sentences.dataset.summary':
    'Generated {generated} · full corpus {tokens} tokens · word table {wordTable} · char table {charTable}',
  'sentences.tableEntries': '{entries} entries',

  // 文本输入
  'sentences.input.title': 'Text input',
  'sentences.input.modeAll': 'Analyze all',
  'sentences.input.modeSelection': 'Selection only',
  'sentences.input.description':
    'Paste text, or select a span in the text box; the coloring below updates in real time.',
  'sentences.input.placeholder': 'Paste the Chinese text to analyze here…',
  'sentences.input.counter': '{chars} chars · {lines} lines',
  'sentences.input.selected': '{count} chars selected',
  'sentences.input.selectHint': '(select a span in the text box)',
  'sentences.input.scopeAll': 'Scope: full text',
  'sentences.input.analyzing': 'Analyzing…',

  // 分域过滤
  'sentences.domains.title': 'Compare scopes',
  'sentences.domains.hint': 'Which scopes appear in the comparison column; none selected = all. Colouring only follows the primary table.',
  'sentences.domains.selectAll': 'Select all',
  'sentences.domains.entries': '{entries} entries',
  /** 「清空」按钮在分域过滤与操作行各出现一次，同页同义，共用一条 */
  'sentences.clear': 'Clear',

  // 操作
  'sentences.actions.sendToPopup': 'Send to popup',
  'sentences.actions.capture': 'Capture',
  'sentences.actions.copy': 'Copy all',

  // 分析结果
  'sentences.result.title': 'Results',
  'sentences.result.tokenCount': '{count} tokens',
  'sentences.result.accepted': 'Counted {count}',
  'sentences.result.skipped': 'Punct/space {count}',
  'sentences.result.unknown': 'Not in corpus {unique} unique / {total} times',
  'sentences.result.description':
    'Hover any token; the “Token detail” panel on the right shows its frequency, rank, top %, share, and domain ranks. Click a token to pin the detail.',
  'sentences.result.thresholdHint':
    'The seven tier thresholds for both tables can be customized on the “Table Manager” page; tokens here are colored by the effective thresholds.',
  'sentences.result.averageBaseline':
    'Average share-per-token baseline: word table {word} · char table {char}',
  'sentences.empty.selection': 'Select a span in the text box, or switch to “Analyze all”.',
  'sentences.empty.none': 'Nothing to analyze yet — paste some text first.',

  // 词条详情面板
  'sentences.detail.aria': 'Token detail',
  'sentences.detail.title': 'Token detail',
  'sentences.detail.pinned': 'Pinned',
  'sentences.detail.hint': 'Hover to view · click to pin',
  'sentences.detail.emptyHint':
    'Hover any token on the left; its frequency, rank, top %, share, tier, and domain ranks are fixed here.',
  'sentences.detail.pinnedHint': 'Once pinned, hovering other tokens won’t change this',
  'sentences.detail.unpin': 'Unpin',

  // 提示（toast）
  'sentences.notice.emptyNoSend': 'The text box is empty — nothing to send.',
  'sentences.notice.sentToPopup': 'Sent the full text to the popup.',
  'sentences.notice.captureEmpty': 'No globally selected text detected.',
  'sentences.notice.captureOk': 'Captured the globally selected text and re-analyzed.',
  'sentences.notice.emptyText': 'The text box is empty.',
  'sentences.notice.copied': 'Copied the full text to the clipboard.',

  // ---------------------------------------------------------------- 七组分组标签
  // key 必须与 crates/vocfreq-core/src/rank.rs::TIER_KEYS 一致（顺序也一致）。
  // 语义是词频的稀有度梯度（单调递减），不是游戏术语。
  'tier.very_common': 'Very common',
  'tier.common': 'Common',
  'tier.fairly_common': 'Fairly common',
  'tier.medium': 'Medium',
  'tier.fairly_rare': 'Fairly rare',
  'tier.rare': 'Rare',
  'tier.very_rare': 'Very rare',
  /** 第 8 种状态：语料库未收录（与「Very rare」不同，那一组是有排名的真实分组） */
  'tier.unknown': 'Not in corpus',

  // ---------------------------------------------------------------- 生效阈值回退警告
  'bounds.invalidWord':
    'The word-table thresholds must be {count} non-negative, strictly increasing integers; temporarily fell back to the meta defaults.',
  'bounds.invalidChar':
    'The char-table thresholds must be {count} non-negative, strictly increasing integers; temporarily fell back to the meta defaults.',
  'bounds.invalidPct':
    'Top-percent bounds must be {count} strictly increasing values within 0–100; temporarily fell back to this table’s default tiers.',
  'bounds.evenNoEntries':
    'This table has 0 entries, so it cannot be divided evenly; fell back to the meta defaults.',
  'bounds.evenInvalid':
    'Even division produced an invalid result (too few entries); fell back to the meta defaults.',
  'bounds.coverageInvalid':
    'Coverage targets must be strictly increasing and within 0–100%; fell back to the meta defaults.',
  'bounds.coverageNoCurve':
    'No coverage curve available yet; showing with the meta default thresholds.',
  'bounds.coverageUnsolvable':
    'These coverage targets cannot be solved into strictly increasing rank thresholds (targets too close to or beyond the curve); fell back to the meta defaults.',
  /** 带前缀的形式（「词条详情」面板用）：`{message}` 是上面某一条 */
  'bounds.fallbackNotice': 'Effective thresholds fell back: {message}',

  // ---------------------------------------------------------------- 词典管理页
  'dicts.browserPreview':
    'Browser preview mode: a demo data folder (including one broken file and one dictionary with pitfalls); importing and deleting only affect memory.',
  'dicts.loading': 'Reading the data folder…',
  'dicts.loadFailed': 'Failed to read the dictionary list: {error}',
  'dicts.origin.seeded': 'Seeded',
  'dicts.origin.imported': 'Imported',
  'dicts.origin.scanned': 'Scanned',
  'dicts.origin.unknown': 'Unknown',

  // 数据文件夹
  'dicts.dataDirTitle': 'Data folder',
  'dicts.dataDirDescription':
    'Both dictionaries (dicts\\) and tables (tables\\) live here. Changing the folder only creates directories — nothing is moved — and it clears the currently active table.',
  'dicts.defaultLocation': 'Default location',
  'dicts.readyBadge': 'Usable dictionaries',
  'dicts.notReadyBadge': 'No usable dictionary',
  'dicts.openDataDir': 'Show in file manager',
  'dicts.openDictsDir': 'Open dicts\\',
  'dicts.changeDataDir': 'Change data folder',
  'dicts.dataDirLayout': 'Dictionaries: {dicts} · Tables: {tables}',
  'dicts.dataDirChanged':
    'Data folder changed to {dir}. The active table was cleared — activate one again on the “Table Manager” page.',
  'dicts.pickDataDir': 'Choose data folder',
  'dicts.noUsableDict':
    'There is not a single readable dictionary here, so a scan cannot start — dictionaries are no longer compiled into the app and there is no built-in fallback. Import a .dict first.',
  'dicts.readyMismatch':
    'Note: the usable-dictionary check in the UI disagrees with the backend’s library_ready result. Please re-check.',

  // 词典清单
  'dicts.listTitle': 'Dictionaries',
  'dicts.countBadge': '{count} files',
  'dicts.listDescription':
    'One dictionary is one .dict file (jieba’s “word frequency tag”, one per line). It is plain text you can edit in Notepad; editing changes its content fingerprint, and tables that depend on it will report “dictionary changed”.',
  'dicts.empty': 'The data folder has no .dict file yet. Use “Import dictionary” in the top right.',
  'dicts.import': 'Import dictionary',
  'dicts.pickDictFile': 'Choose a dictionary file (.dict)',
  'dicts.dictFilter': 'jieba dictionary',
  'dicts.imported': 'Imported: {name}',
  'dicts.entries': '{count} entries',
  'dicts.brokenBadge': 'Unreadable',
  'dicts.error': 'Problem with this file: {error}',
  'dicts.revealFile': 'Show in file manager',
  'dicts.delete': 'Delete',
  'dicts.deleted': 'Deleted dictionary {name}.',
  'dicts.deletedWithUsers':
    'Deleted dictionary {name}. These tables referenced it and now report “dictionary missing”: {tables}.',
  'dicts.usedByTables': 'Referenced by: {tables}',

  // 隐患提示
  'dicts.warnFreqZero':
    '{count} entries write 0 as the weight. In jieba that column is a **probability weight**, not a frequency: a 0 means the word can **never** be segmented out — it only takes up a line. Change it to a positive number to keep the word, or delete those lines.',
  'dicts.warnFreqOmitted':
    '{count} entries omit the weight column. They are loaded via suggest_freq, which usually yields lower weights than an explicit value — the resulting order may differ from what you expect.',
  'dicts.skippedLines': 'Skipped {comments} comment lines and {blanks} blank lines.',

  // 删除确认（就地展开）
  'dicts.confirmDeleteTitle': 'Delete the dictionary “{name}”?',
  'dicts.confirmDeleteUsed':
    '{count} tables use it: {tables}. After deletion those tables report “dictionary missing”: their tokenization no longer matches build time, so their frequencies are unreliable and must be recomputed on the “Table Manager” page.',
  'dicts.confirmDeleteNote':
    'The file is deleted from the data folder directly — no recycle bin, no undo. Seeded dictionaries can be deleted too (there is no permission tier here).',
  'dicts.confirmDeleteYes': 'Delete',
  'dicts.noPermissionNote':
    'Note: a seeded dictionary is exactly the same kind of thing as one you imported — no permission tier, both can be deleted or renamed. The origin badge above only tells you where it came from.',

  // ---------------------------------------------------------------- 表管理页
  'tables.previewCapNote':
    '{skipped} thresholds exceed this table’s entry count ({entries}); clamped to the total in the preview. Actual tiering is unaffected.',

  // 浏览器预览 / 加载 / 空态 / 未装载
  'tables.browserPreview':
    'Browser preview mode: the table list and toggles use built-in demo data (memory only); tier settings are also kept in memory only.',
  'tables.loading': 'Loading output directory and settings…',
  'tables.loadFailed': 'Failed to read dataset status: {error}',
  'tables.emptyTitle': 'No tables to manage yet',
  'tables.emptyDescription':
    'The table list comes from meta.json (`meta.tables`). Produce a result on the “Generate Frequency Tables” page first.',
  'tables.goWordFreq': 'Go to Generate Frequency Tables',
  'tables.recheck': 'Re-check',
  // 产物未装载到后端（与 sentences.notLoaded.* 同构，语境不同各留一份）
  'tables.notLoaded.title': 'Output directory not loaded into the backend',
  'tables.notLoaded.p1WithError': 'meta.json found ({dir}), but `open_dataset` failed: {error}.',
  'tables.notLoaded.p1NoError': 'meta.json found ({dir}), but `open_dataset` failed.',
  'tables.notLoaded.p2': 'The table list is still manageable, but `tier_curve` may fail to return a curve.',

  // A. Frequency tables (all scopes, fully equal)
  'tables.listTitle': 'Frequency tables',
  'tables.tableCountBadge': '{count} tables',
  'tables.scopeCountBadge': '{count} scopes',
  'tables.primaryBadge': 'Primary: {scope}',
  'tables.primaryDescription':
    'Every scope has one word table and one char table, and they are **fully equal** (`full` is merely “all domains added together”). {emphasis}; sentence-analysis colouring and tiers, the leaderboard, and the tier preview all follow it, while the other tables are only shown for comparison.',
  'tables.primaryEmphasis': 'ticking “primary” decides which table answers “how common is this word”',
  'tables.equalNote':
    'Each row is just one scope of the same output — there is no full-corpus vs. domain distinction. To change the “most common” baseline, change the primary table; to merge several domains, use “Add up” below.',
  'tables.primarySet': 'Primary table switched to “{scope}”.',
  'tables.primaryFailed': 'Failed to switch the primary table to “{scope}”: {error}',
  'tables.primaryAria': 'Make {scope} the primary table',
  'tables.primaryBadgeShort': 'Primary',
  'tables.enabledCountBadge': '{count} enabled',
  'tables.saving': 'Saving…',
  'tables.quickSelect': 'Quick select',
  'tables.selectAll': 'Select all',
  'tables.selectNone': 'Select none',
  'tables.onlyFull': 'Full corpus only',
  'tables.onlyWord': 'Word table only',
  'tables.onlyChar': 'Char table only',
  'tables.resetAllEnabled': 'Reset to all enabled',
  'tables.col.scope': 'Scope',
  'tables.col.kind': 'Kind',
  'tables.col.entries': 'Entries',
  'tables.col.tokens': 'Total tokens',
  'tables.col.vfr': '.vfr size',
  'tables.col.lastCoverage': 'Last tier coverage',
  'tables.col.source': 'Source',
  'tables.col.primary': 'Primary',
  'tables.sourceScanned': 'Scanned',
  'tables.sourceComposed': 'Added up ({count})',
  'tables.countHint':
    'Note: using `--skip-domain-tables` or scanning only some domains yields fewer scopes. A full output has one word table and one char table for “all domains” plus each top-level subdirectory.',

  // B. Add up
  'tables.compose.title': 'Add up',
  'tables.compose.description':
    'Add several tables together into a new one (written into the same output directory as another peer scope). The addition is **exact**: the scan itself scans each scope and accumulates, so the sum of the scope tables equals the whole-corpus table entry for entry.',
  'tables.compose.pickLabel': 'Tables to add',
  'tables.compose.pickedBadge': '{count} selected',
  'tables.compose.selectAll': 'Select all',
  'tables.compose.clearGroup': 'Clear group',
  'tables.compose.nameLabel': 'New scope name',
  'tables.compose.namePlaceholder': 'e.g. sum:news+wiki',
  'tables.compose.defaultName': 'sum',
  'tables.compose.run': 'Add up',
  'tables.compose.running': 'Adding up…',
  'tables.compose.needPick': 'Select at least one table first.',
  'tables.compose.mixedKinds': 'Only one kind (word or char) can be added at a time. Do it in two passes.',
  'tables.compose.done': 'Created scope “{scope}”: {kinds}, {entries} entries.',
  'tables.compose.doneShort': 'Created “{scope}”',
  'tables.compose.note':
    'Cost: one extra table on disk (roughly the size of a scanned table). Delete the sources above to reclaim the space if you no longer need them; the new table can itself be added up again.',

  // A2. 词频表管理（数据文件夹 tables\）+ 词典绑定状态
  'tables.library.title': 'Table management',
  'tables.library.countBadge': '{count} tables',
  'tables.library.description':
    'The tables under tables\\ in the data folder, plus the one “pointing at an output directory outside the data folder”. Each table records the dictionary chain it was built with (including sha256 fingerprints); this is the verification result.',
  'tables.library.activeBadge': 'Active: {name}',
  'tables.library.dataDir': 'Data folder: {dir}',
  'tables.library.loadFailed': 'Failed to read the table list: {error}',
  'tables.library.empty':
    'No tables in the data folder yet. Run a statistics pass on “Generate Frequency Tables” and the output lands here.',
  'tables.library.active': 'Active',
  'tables.library.external': 'Outside the data folder',
  'tables.library.corpus': 'Corpus: {corpus}',
  'tables.library.generated': 'Generated {generated}',
  'tables.library.tableError': 'This table is incomplete: {error}',
  'tables.library.entriesUnavailable': 'Entry count unknown',
  'tables.library.activate': 'Activate',
  'tables.library.rescan': 'Re-run statistics',
  'tables.library.rescanSuggestion': 'Re-run statistics',
  'tables.library.openDir': 'Show in file manager',
  'tables.library.delete': 'Delete',
  'tables.library.activated': 'Activated “{name}”; the tokenizer was rebuilt from its recorded dictionary chain.',
  'tables.library.activateFailed': 'Failed to activate “{name}”: {error}',
  'tables.library.deleted': 'Deleted table “{name}”.',
  'tables.library.deleteFailed': 'Failed to delete “{name}”: {error}',
  'tables.library.confirmDeleteTitle': 'Delete the table “{name}”?',
  'tables.library.confirmDeleteNote':
    'The whole output directory is removed (meta.json plus every .vfr / .tsv) and cannot be undone. Tables outside the data folder get no delete button — that directory is not ours to manage.',
  'tables.library.confirmDeleteYes': 'Delete',
  'tables.library.activeBinding': 'Active table’s dictionary binding: {label}',
  'tables.library.dictChain': 'Dictionary chain: {chain}',
  'tables.library.absentDicts': ' (not found in the data folder: {names})',

  // 绑定状态（`Binding` 的四种取值）
  'tables.binding.ok': 'Dictionaries match',
  'tables.binding.legacy': 'Legacy output — no dictionary record, unverifiable',
  'tables.binding.legacyDetail':
    'This is a schema v1 output: dictionaries were compiled into the app back then, so the output only kept a free-text label and no fingerprint. It can be neither confirmed nor denied.',
  'tables.binding.drifted': 'Dictionaries changed — frequencies may be wrong',
  'tables.binding.driftedDetail':
    'Dictionaries whose content changed: {changed}. Tokenization no longer matches build time; re-running statistics is recommended.',
  'tables.binding.missing': 'Dictionaries not found',
  'tables.binding.missingDetail':
    'These dictionaries are no longer in the data folder: {missing}. Restore them or change the chain before re-running statistics.',

  // B. 分组自定义
  'tables.tierConfigTitle': 'Customize tiers',
  'tables.tierCountBadge': 'Seven tiers',
  'tables.rankUpperBound': 'Rank upper bound',
  /** 说明句：{names} 是七组名；句子里有一处 <b> 加粗，用 splitMessage 切 {emphasis} */
  'tables.tierConfigDescription':
    'Tier names are fixed ({names}); what you set here is each tier’s {emphasis}: tiers 1–6 each have an upper bound, and tier 7 is automatically “everything above”.',
  'tables.methodLabel': 'Tiering method',
  'tables.method.topPct': 'By top % (default)',
  'tables.method.rank': 'By rank',
  'tables.method.coverage': 'By coverage',
  'tables.method.even': 'Even by entries',
  'tables.whatLabel': 'What it does: ',
  'tables.whenLabel': 'When to use: ',
  'tables.methodNote.topPct.what':
    'Set a “top N%” bound per tier: top % = rank ÷ the table’s total entries × 100. Tier 1 = the top 0.0026% of entries, and so on; tier 7 is everything else.',
  'tables.methodNote.topPct.when':
    'The default. It only depends on position, not on table size, so switching tables (a domain table has tens of thousands of entries, the whole corpus millions) — or switching the primary table — keeps the same meaning. Absolute ranks cannot do that: the same bound squeezes a small table entirely into the top tier.',
  'tables.methodNote.rank.what':
    'Set a rank upper bound per tier: tier 1 = ranks 1..N₁, tier 2 = N₁+1..N₂ … tier 7 = everything above the last bound.',
  'tables.methodNote.rank.when':
    'Most intuitive when the scale is fixed and you only ever look at one table; the bounds are calibrated for a particular table size, so a very different table distorts the colours.',
  'tables.methodNote.coverage.what':
    'Given the “cumulative percent of text each tier covers”, the program solves the coverage curve for the matching rank bounds (interpolating in log10(rank)).',
  'tables.methodNote.coverage.when':
    'For questions like “how many words cover the top 50% of text”. Cost: tier sizes vary a lot, and the coverage curve must be read first.',
  'tables.methodNote.even.what':
    'Crudely split entries into seven equal parts: each tier has about the same number of entries.',
  'tables.methodNote.even.when':
    'Use when you want similar sample sizes per tier (sampling, statistical tests); completely unrelated to the “common / rare” intuition.',
  'tables.pctBoundsTitle': 'Top-% bounds (%)',
  'tables.pctBoundsHint': 'Tier 7 is always “everything above”, so fill in only 6, strictly increasing',
  'tables.pctBoundAria': 'Tier {index} top-percent bound',
  'tables.pctBoundsSaved': 'Top-percent bounds saved.',
  'tables.pctBoundsReset': 'Restored this table’s default top-percent tiers.',
  'tables.pctNote':
    'The conversion uses the **primary** word table’s entry count ({entries} entries): top-% bound × entries = rank bound. A table of a different size yields different rank thresholds for the same top % — which is exactly why it compares better than absolute ranks.',
  'tables.solveFromPct': 'Convert to rank thresholds and switch to “By rank”',
  'tables.coverageTargetLabel': 'Cumulative coverage target (%)',
  'tables.coverageTargetHint':
    'Tier 7 is always “everything above”, so fill in only 6, strictly increasing',
  'tables.solveButton': 'Solve current coverage into rank thresholds',
  'tables.resetDefault': 'Restore defaults',
  'tables.groupN': 'Tier {index}',
  'tables.groupCoverageAria': 'Tier {index} cumulative coverage',
  'tables.curveLoadFailed': 'Failed to load curve: {error}',
  'tables.curveLoading': 'Loading coverage curve…',
  'tables.curvePoints': '{count} points',
  'tables.curveSummary':
    'Curves: {word} (word) · {char} (char) · cached per output on the Rust side; revisiting this page doesn’t recompute.',
  'tables.solvedWordTitle': 'Word-table solution',
  'tables.solvedCharTitle': 'Char-table solution',
  'tables.groupUpperLe': 'Tier {index} ≤',
  'tables.wordBoundsTitle': 'Word-table thresholds',
  'tables.charBoundsTitle': 'Char-table thresholds',
  'tables.sixBoundsHint': '6 rank bounds',
  'tables.boundAria': '{kind}tier {index} rank bound',
  'tables.group7Note': 'Tier 7 = “everything above” (all entries ranked after {count})',
  'tables.previewTitle': 'Live preview',
  'tables.previewHint':
    'Editing the numbers above recomputes immediately; “entries per tier” is an estimate without a curve, while cumulative coverage from a curve is exact',
  'tables.wordPreview': 'Word-table preview',
  'tables.charPreview': 'Char-table preview',
  'tables.preview.col.name': 'Tier',
  'tables.preview.col.range': 'Bound',
  'tables.preview.col.entries': 'Entries',
  'tables.preview.col.coverage': 'Coverage',
  'tables.preview.col.cumulative': 'Cumulative',
  'tables.preview.col.source': 'Source',
  'tables.sourceCurve': 'Curve',
  'tables.sourceEstimated': 'Estimated by tier share',
  'tables.legendTitle': 'Legend (word-table effective thresholds)',
  'tables.applyNote':
    'Settings apply immediately and are saved: token coloring, hover popovers, and the Leaderboard tier column all recompute with these thresholds. When all three tier fields are null / \'rank\', the UI exactly matches the meta defaults.',

  // 操作提示（toast）
  'tables.saveFailed': 'Save failed: {error}',
  'tables.methodSwitched': 'Tiering method switched to “{method}”.',
  'tables.wordBoundsSaved': 'Word-table thresholds saved.',
  'tables.charBoundsSaved': 'Char-table thresholds saved.',
  'tables.coverageSaved': 'Coverage targets saved.',
  'tables.coverageReset': 'Restored default coverage targets (from the meta cumulative coverage).',
  'tables.wordBoundsReset': 'Word-table thresholds restored to the meta defaults.',
  'tables.charBoundsReset': 'Char-table thresholds restored to the meta defaults.',
  'tables.solveFailed': 'No coverage curve available; cannot solve.',
  'tables.solveFailedWithError': 'No coverage curve available; cannot solve: {error}.',
  'tables.solved':
    'Solved the coverage curve into rank thresholds (word: {ranks}) and switched back to “By rank”.',

  // ---------------------------------------------------------------- 排行榜页
  'leaderboard.browserPreview':
    'Browser preview mode: using built-in demo data; paging, search, and tier filtering all work.',
  'leaderboard.checking': 'Checking corpus outputs…',
  'leaderboard.statusFailed': 'Failed to read dataset status: {error}',
  'leaderboard.noTable.title': 'No frequency tables yet',
  'leaderboard.noTable.description': 'The leaderboard needs frequency tables first. Directory checked: ',
  'leaderboard.noDir': '(no table opened yet)',
  'leaderboard.goWordFreq': 'Go to Generate Frequency Tables',
  'leaderboard.goDicts': 'Go to Dictionaries',
  'leaderboard.recheck': 'Re-check',
  // 产物未装载到后端（与 tables.notLoaded.* 同构）
  'leaderboard.notLoaded.title': 'Output directory not loaded into the backend',
  'leaderboard.notLoaded.p1WithError': 'meta.json found ({dir}), but `open_dataset` failed: {error}.',
  'leaderboard.notLoaded.p1NoError': 'meta.json found ({dir}), but `open_dataset` failed.',
  'leaderboard.notLoaded.p2':
    'Until that command is implemented on the Rust side, the list endpoint may return empty results.',

  // 控制区
  'leaderboard.title': 'Leaderboard',
  'leaderboard.full': 'Full corpus',
  'leaderboard.scope': 'Scope',
  'leaderboard.primaryScope': 'Primary table ({scope})',
  'leaderboard.primaryBadge': 'Primary',
  'leaderboard.summary':
    'Generated {generated} · current table {entries} entries · {tokens} tokens',
  'leaderboard.domain': 'Domain',
  'leaderboard.searchPlaceholder': 'Search by prefix, e.g. “语”',
  'leaderboard.searchAria': 'Prefix search',
  'leaderboard.searching': 'Searching…',
  'leaderboard.tierFilter': 'Tier filter',
  'leaderboard.tierFilterHint':
    'Applies only to the {count} rows currently loaded (the backend pages by rank, so cross-page filtering isn’t possible)',
  'leaderboard.clearFilter': 'Clear filter',

  // 排名列表
  'leaderboard.rankListTitle': 'Rankings',
  'leaderboard.filteredCount': '{count} rows after filter',
  'leaderboard.loading': 'Loading…',
  'leaderboard.rowHint':
    'Click any row to “Add to analysis” and send the token to the Sentence Analysis page.',
  'leaderboard.col.rank': 'Rank',
  'leaderboard.col.word': 'Word',
  'leaderboard.col.count': 'Frequency',
  'leaderboard.col.share': 'Share',
  'leaderboard.col.top': 'Top %',
  'leaderboard.col.topTitle': 'Rank ÷ the current table’s total entries',
  'leaderboard.col.shareTitle':
    'Share = the token’s frequency ÷ full-corpus table total tokens (not a proportion of entries)',
  'leaderboard.col.tier': 'Tier',
  'leaderboard.col.flags': 'Flags',
  'leaderboard.col.actions': 'Actions',
  'leaderboard.rowTitle': 'Click to add to analysis',
  'leaderboard.charNoFlag': 'No dictionary flag in char table',
  'leaderboard.inDict': 'In dictionary',
  'leaderboard.flagOutOfDict': 'Out of dictionary',
  'leaderboard.flagUserDict': 'User dictionary',
  'leaderboard.detail': 'Detail',
  'leaderboard.copy': 'Copy',
  'leaderboard.emptySearch': 'No tokens starting with “{query}”.',
  'leaderboard.emptyFiltered':
    'No tokens on this page match the tier filter. Try another page or clear the filter.',
  'leaderboard.emptyPage': 'No data on this page.',
  'leaderboard.searchResultCount': '{count} search results',
  'leaderboard.rankRange': 'Ranks {from}–{to} / {total} total',

  // 分页
  'leaderboard.firstPage': 'First',
  'leaderboard.prevPage': 'Prev',
  'leaderboard.pageLabel': 'Page',
  'leaderboard.pageAria': 'Page number',
  'leaderboard.totalPages': '/ {count}',
  'leaderboard.nextPage': 'Next',
  'leaderboard.lastPage': 'Last',
  'leaderboard.perPage': '{count} per page',

  // 查词详情
  'leaderboard.detailTitle': 'Token detail',
  'leaderboard.close': 'Close',
  'leaderboard.groupN': 'Tier {index}',
  'leaderboard.detail.top': 'Top',
  'leaderboard.detail.topTitle': 'Rank ÷ the table’s total entries',
  'leaderboard.detail.shareTitle': 'The token’s frequency ÷ full-corpus table total tokens',
  'leaderboard.detail.dict': 'Dictionary',
  'leaderboard.detail.inDict': 'In jieba dictionary',
  'leaderboard.detail.outDict': 'Out of dictionary',
  'leaderboard.addToAnalysis': 'Add to analysis',
  'leaderboard.copyWord': 'Copy token',
  'leaderboard.detailLoading': 'Looking up…',
  'leaderboard.addedToAnalysis': 'Added to analysis: {word}',
  'leaderboard.copied': 'Copied: {word}',

  // ---------------------------------------------------------------- 悬浮小窗
  'popup.title': 'VocTier Capture',
  'popup.noTable': 'No frequency tables',
  'popup.themeTitle': 'Theme: {hint}',
  'popup.themeAria': 'Toggle theme',
  'popup.copyAllTitle': 'Copy all',
  'popup.copy': 'Copy',
  'popup.sendTitle': 'Send to main window (Sentence Analysis)',
  'popup.sendBack': 'Send to main window',
  'popup.hideTitle': 'Hide window',
  'popup.closeAria': 'Close window',
  'popup.inputPlaceholder': 'Paste or type Chinese text to analyze…',
  'popup.clearTitle': 'Clear',
  'popup.clear': 'Clear',
  'popup.loading': 'Loading frequency tables…',
  'popup.noTableDesc': 'No frequency tables available, so captured text can’t be colored.',
  'popup.noTableHint':
    'Run a statistics pass on the “Generate Frequency Tables” page in the main window first.',
  'popup.emptyHint': 'After you type or paste text, each token’s tier coloring appears here in real time.',
  'popup.detailAria': 'Token detail',
  'popup.detailTitle': 'Token detail',
  'popup.unpin': 'Unpin',
  'popup.summary': '{accepted} tokens · {unknown} not in corpus',
  /** meta 存在时才追加的表名段（与 popup.summary 拼成一行） */
  'popup.summaryTable': ' · {entries} in word table',
  'popup.detailEmpty': 'Nothing to show yet.',
  'popup.copyEmpty': 'Nothing to copy',
  'popup.copied': 'Copied',
  'popup.sendEmpty': 'Nothing to send',
  'popup.sentToMain': 'Sent to main window',

  // ---------------------------------------------------------------- 词条详情（TokenDetail）
  'tokenDetail.emptyHint': 'Hover any token above; its frequency, rank, and tier appear here.',
  'tokenDetail.pctNoRecord':
    'This token has no record in the full-corpus table, so it has no share.',
  'tokenDetail.fullWord': 'Full-corpus word table',
  'tokenDetail.fullChar': 'Full-corpus char table',
  'tokenDetail.pctNoTotal': 'Share = token occurrences ÷ {which} total tokens',
  'tokenDetail.pct': 'Share = {count} occurrences ÷ {which} total {total} tokens',
  'tokenDetail.freq': 'Frequency',
  'tokenDetail.rank': 'Rank',
  'tokenDetail.top': 'Top',
  'tokenDetail.topTitle': 'Rank ÷ the primary table’s total entries',
  'tokenDetail.topValue': 'Top {pct}',
  'tokenDetail.share': 'Share',
  'tokenDetail.dictUnknown': 'Dictionary status unknown',
  'tokenDetail.inDict': 'In jieba dictionary',
  'tokenDetail.outDictHmm': 'Out of dictionary (HMM new word)',
  'tokenDetail.fromUserDict': 'From user dictionary',
  'tokenDetail.pinned': 'Pinned',
  'tokenDetail.skipped': 'Punctuation / whitespace — not counted',
  'tokenDetail.notCollected':
    'Not in corpus: this token appears in neither the word nor the char table, so it has no frequency, rank, or tier (unlike “very rare”, which is a real tier with ranks).',
  'tokenDetail.rangeLine': '{group} · effective rank bound {range}',
  'tokenDetail.tableRanksTitle': 'All tables (top %)',

  // ---------------------------------------------------------------- 分组统计表（TierStatsTable）
  'tierStats.col.group': 'Tier',
  'tierStats.col.upper': 'Rank upper bound',
  'tierStats.col.entries': 'Entries',
  'tierStats.col.tokens': 'Tokens',
  'tierStats.col.coverage': 'Tier coverage',
  'tierStats.col.cumulative': 'Cumulative',

  // ---------------------------------------------------------------- 桥接层用户可见错误串（bridge.ts）
  'bridge.noTauri':
    'Not running in the desktop app — this feature needs the VocTier desktop application.',
  'bridge.unknownVersion': 'Unknown',
  'bridge.invalidDatasetDir': 'This directory is not a valid VocTier output directory.',
  'bridge.noMeta': 'The backend returned no output metadata.',
  'bridge.wordNotInCorpus': '“{word}” is not in the corpus',
  'bridge.popupUnavailable': 'The popup can’t be opened in browser preview mode.',
  'bridge.noMainWindow': 'There’s no main window to send back to in browser preview mode.',
  'bridge.clipboardUnsupported': 'Clipboard writes are not supported in this environment.',
  'bridge.pickDirectory': 'Choose directory',
  'bridge.pickFile': 'Choose file',

  // ---------------------------------------------------------------- 通用格式
  'format.allAbove': 'everything above',
  'format.durationMs': '{value} ms',
  'format.durationSec': '{value} s',
  'format.durationMinSec': '{minutes}m {seconds}s',
  'format.durationHourMin': '{hours}h {minutes}m',

  // ---------------------------------------------------------------- 表与 token
  'table.word': 'Word table',
  'table.char': 'Char table',
  'table.none': 'No table',

  // ---------------------------------------------------------------- 布局骨架
  'layout.sidebarTagline': 'Word frequency analysis',
  'layout.sidebarReady': 'Ready',
  'layout.mainNav': 'Main navigation',
  'layout.badge': 'Skeleton',
  'layout.searchPlaceholder': 'Search tokens (placeholder)',
  'layout.searchLabel': 'Search tokens',
  'layout.stub.pending': 'Planned',
  'layout.stub.hooksTitle': 'Integration guide',
  'layout.stub.hooksDesc': 'Suggested places to mount the future business logic',
};
