/**
 * 简体中文文案 —— **源语言（source of truth）**。
 *
 * 约定：
 *   1. 本文件的 key 集合就是 `MessageKey`（见 `./types`）。别处翻译漏了 key 会在
 *      `pnpm check` 直接报错，不靠人眼比对两份文件。
 *   2. 只放**界面文案**。Rust 侧错误串、启动/崩溃日志、CLI 输出都不在这里 ——
 *      启动日志刻意保持中文，那是给自己排障看的（见 README §十）。
 *   3. 参数用 `{name}` 占位，由 `i18n.svelte.ts::t(key, params)` 替换。
 * 4. 新增文案一律先加到本文件，再用 `t('...')` 引用，不要在组件里写字面量。
 *
 * ## 命名与复用
 *
 *   - `common.*` 放**跨页面共用的短 UI 词**（浏览 / 清除 / 开关值…），只此一份。
 *   - 其余按页面/模块分命名空间（`nav.*`、`settings.*`、`wordfreq.*`…）。
 *   - 同一个词在不同页面**允许各留一条**（例如两个页面的「最小词频」）：语境不同，
 *     翻译可能不同，硬合并反而让译者没法按语境措辞。
 *   - 句子里要加粗某个值时，**不要把句子拆成几条消息**，而是留 `{占位符}` 并用
 *     `i18n.svelte.ts::splitMessage()` 在渲染处切开（保住词序自由度）。
 *
 * `tier.*` 是**唯一**按稳定标识命名的文案组：key 与
 * `crates/vocfreq-core/src/rank.rs::TIER_KEYS` 一一对应。组名是文案，会随语言变化，
 * 所以它必须能按 key 查到翻译，而不是拿中文组名当身份（见 `tier-colors.ts` 的说明）。
 */
export const zhCN = {
  // ---------------------------------------------------------------- 应用
  'app.title': 'VocTier 字词频率分析',
  // 品牌名不翻译（故不设 'app.name'）：侧边栏与标题里都固定写 VocTier。

  // ---------------------------------------------------------------- 跨页面共用
  'common.browse': '浏览…',
  'common.clear': '清除',
  'common.choose': '选择…',
  'common.cancel': '取消',
  /** 并列项的连接符：中文用顿号，英文应给 `, ` */
  'common.listSeparator': '、',
  'common.on': '开',
  'common.off': '关',
  'common.keep': '保留',
  'common.discard': '丢弃',
  'common.yes': '是',
  'common.no': '否',

  // ---------------------------------------------------------------- 导航
  'nav.wordfreq.label': '生成词频表',
  'nav.wordfreq.title': '生成词频表',
  'nav.wordfreq.description': '导入语料目录，统计字词出现频率并导出结果表。',
  'nav.sentences.label': '划句分析',
  'nav.sentences.title': '划句分析',
  'nav.sentences.description': '按句切分文本，逐句查看用词分布与难度层级。',
  'nav.leaderboard.label': '排行榜',
  'nav.leaderboard.title': '排行榜',
  'nav.leaderboard.description': '按频次、覆盖率、词长等维度查看字词排名。',
  'nav.dicts.label': '词库管理',
  'nav.dicts.title': '词库管理',
  'nav.dicts.description':
    '管理数据文件夹里的词库（jieba 格式的 .dict）：导入、删除，并看清每一份的隐患。',
  'nav.tables.label': '表管理',
  'nav.tables.title': '表管理',
  'nav.tables.description': '开关参与分域对比与排行榜的表，自定义七组分组阈值。',
  'nav.settings.label': '设置',
  'nav.settings.title': '设置',
  'nav.settings.description': '分词参数、输出格式、主题与界面偏好。',

  // ---------------------------------------------------------------- 主题与外观
  'theme.light': '浅色',
  'theme.dark': '深色',
  'theme.system': '跟随系统',
  'theme.toggleHint': '{current}（点击切到{next}）',
  // ---------------------------------------------------------------- 设置页
  'settings.browserPreview':
    '浏览器预览模式：设置可以编辑与「保存」，但不会写入磁盘（仅保存在内存里）。',
  'settings.loading': '正在读取设置…',
  'settings.readFailed': '读取设置失败：{error}',
  'settings.saveFailed': '保存失败：{error}',
  'settings.saved': '设置已保存。',
  'settings.resetHint': '已恢复默认值，记得点「保存设置」。',

  // 全局取词
  'settings.hotkey.title': '全局取词',
  'settings.hotkey.valid': '格式正确',
  'settings.hotkey.invalid': '格式待修正',
  'settings.hotkey.description':
    '按下热键会模拟一次 Ctrl+C 取走选区文本，并在悬浮小窗里做频率分析。',
  'settings.hotkey.label': '全局热键',
  // 这一行里有两个被 `font-mono` 包裹的示例，整句塞不进一条消息（消息表不放 HTML），
  // 所以拆成「前 / 分隔 / 后」三段。示例本身（Alt+Q、Ctrl+Shift+Q）是键名，不翻译。
  'settings.hotkey.hintPrefix': '格式：修饰键 + 主键，例如 ',
  'settings.hotkey.hintSeparator': '、',
  'settings.hotkey.hintSuffix': '。',
  'settings.testCapture': '测试取词',
  'settings.testCaptureHint': '测试会在当前前台程序里模拟一次复制，并把结果写入剪贴板。',
  'settings.captureNoText': '没有检测到选中的文本。请先在别的程序里选一段文字再试。',
  'settings.captureOkCopied': '取词成功（{count} 字），已复制到剪贴板。',
  'settings.captureOkText': '取词成功：{text}',

  // 管理员权限说明。原文中间有一段 `<b>` 加粗，因此按它拆三段。
  // 注意 `p1Before` 结尾那个空格：原模板里换行会被 HTML 折叠成一个空格，这里保留原样。
  'settings.adminNote.title': '关于「以管理员身份运行」',
  'settings.adminNote.p1Before':
    '全局取词的原理是向当前焦点窗口发送 Ctrl+C。Windows 的 UIPI（用户界面特权隔离）规定： ',
  'settings.adminNote.p1Emphasis': '低完整性级别的进程不能向高完整性级别的进程发送输入',
  'settings.adminNote.p1After':
    '。因此当目标程序（例如以管理员身份运行的编辑器、IDE 或终端）权限高于 VocTier 时，热键取词会被系统静默拦截，表现为「按了没反应」。',
  'settings.adminNote.p2':
    '解决办法：右键 VocTier 快捷方式 →「以管理员身份运行」，让两者权限一致（都以管理员运行，或都不以管理员运行）。这一限制来自 Windows 本身，应用无法绕过。',

  // 悬浮小窗
  'settings.popup.title': '悬浮小窗',
  'settings.popup.description': '取词后弹出的小窗尺寸、透明度与自动关闭行为。',
  'settings.popup.width': '宽度（px）',
  'settings.popup.height': '高度（px）',
  'settings.popup.autoClose': '自动关闭（毫秒，0 = 不自动关）',
  'settings.popup.minSize': '小窗至少 200 × 150 像素。',
  'settings.popup.opacity': '不透明度',
  'settings.popup.opacityLabel': '小窗不透明度',
  'settings.popup.alwaysOnTop': '窗口置顶',
  'settings.popup.alwaysOnTopHint': '小窗始终显示在其它窗口之上',

  // 外观与界面语言
  'settings.appearance.title': '外观',
  'settings.appearance.description': '与顶栏的主题按钮共用同一份状态。',
  'settings.appearance.currentTheme': '当前生效：{theme}',
  'settings.language.title': '语言',
  'settings.language.description': '切换界面语言。统计数据与分词口径不受影响。',
  'settings.language.onlyOne': '目前只有简体中文；其余语言的翻译补齐后会自动出现在这里。',

  // 默认统计参数
  'settings.tokenize.title': '默认统计参数',
  'settings.tokenize.description': '「生成词频表」页打开时会用这里的值作为初始值。',
  'settings.tokenize.threads': '默认线程数（0 = 自动）',
  'settings.tokenize.minCount': '最小词频',
  'settings.tokenize.hmm': 'HMM 新词发现',
  'settings.tokenize.hmmHint': '识别词典外的连续汉字组合',
  'settings.tokenize.keepDigit': '保留数字词',
  'settings.tokenize.keepDigitHint': '如 2024、3.14',
  'settings.tokenize.keepLatin': '保留拉丁词',
  'settings.tokenize.keepLatinHint': '如 API、token',
  'settings.tokenize.skipSingleChar': '跳过单字词',
  'settings.tokenize.skipSingleCharHint': '只统计多字词（字表仍会产出）',
  'settings.tokenize.domainTables': '默认产出分域表',
  'settings.tokenize.domainTablesHint': '关闭后只产出全库词表 / 字表，产物更小',

  // 默认目录与词典
  'settings.paths.title': '目录',
  'settings.paths.description':
    '划句分析读取的是「数据文件夹」里 tables\\ 下的词表；词库则放在同一个文件夹的 dicts\\ 下。',
  /** 数据文件夹的徽标（它同时装词库与词表，和旧的「输出目录」不是一回事了） */
  'settings.paths.dataDirBadge': '数据文件夹',
  'settings.paths.corpusDir': '默认语料库目录',
  'settings.paths.dataDir': '数据文件夹',
  'settings.paths.dataDirHint':
    '里面是 dicts\\（词库库）与 tables\\（词表库）两个子目录。留空表示用系统默认位置。',
  'settings.paths.openDataDir': '打开数据文件夹',
  // `userDict` 已废弃：词库现在是数据文件夹里的条目，在扫描时按顺序勾选
  'settings.paths.userDictDeprecatedTitle': '「自定义词典」这个设置已废弃',
  'settings.paths.userDictDeprecatedBody':
    '词库不再是一个额外的文件路径，而是数据文件夹 dicts\\ 里的普通条目 —— 在「生成词频表」页按顺序勾选（第一个是主词库），也可以在「词库管理」页导入或删除。',
  'settings.paths.userDictWillMigrate':
    '旧值 {path} 会在下次启动时被后端迁移进扫描用的词库链，之后这个字段不再写出。',
  'settings.paths.unset': '未设置',
  'settings.pickCorpusDir': '选择默认语料库目录',
  'settings.pickDataDir': '选择数据文件夹',

  // 保存与关于
  'settings.save.saving': '保存中…',
  'settings.save.save': '保存设置',
  'settings.save.reset': '恢复默认值',
  'settings.save.dirty': '有未保存的修改',
  'settings.save.hotkeyInvalid': '热键格式不正确，请改成 Alt+Q 这类写法。',
  'settings.about.title': '关于',
  'settings.about.description': '版本信息来自 app_info 命令。',
  'settings.about.app': '应用',
  'settings.about.appVersion': '应用版本',
  'settings.about.infoUnavailable': '暂时读不到版本信息。',
  'settings.about.infoUnavailableWithError': '暂时读不到版本信息：{error}。',
  'settings.about.footnote':
    '快捷键、目录与小窗外观会在保存后生效；热键的重新注册由 Rust 侧负责。',

  // ---------------------------------------------------------------- 生成词频表页
  'wordfreq.browserPreview':
    '浏览器预览模式：正在使用内置演示数据，完整流程可以点通（目录选择器不可用，可直接编辑路径输入框）。',
  'wordfreq.loading': '正在读取已有设置…',
  // 页首那句「当前打开的是哪张表」——统计完成后会自动刷新
  'wordfreq.currentTable.label': '当前词表：',
  'wordfreq.currentTable.meta': '生成于 {generated} · {tables} 张表',
  'wordfreq.currentTable.none':
    '还没有打开任何词表。先在下面完成一次统计，或到「表管理」页激活一张已有的表。',
  'wordfreq.currentTable.manage': '表管理',

  // 步骤条
  'wordfreq.step.corpus': '选语料库',
  'wordfreq.step.params': '配置参数',
  'wordfreq.step.run': '输出与执行',

  // ① 语料库
  'wordfreq.step1.title': '① 选择语料库目录',
  'wordfreq.step1.description': '填入或选择语料根目录，先「探测」识别分域、文件数与解析规则。',
  'wordfreq.step1.corpusPlaceholder': '例如 D:\\corpus 或 /data/corpus',
  'wordfreq.step1.corpusLabel': '语料库目录',
  'wordfreq.step1.probe': '探测',
  'wordfreq.step1.probing': '探测中…',
  // 这三条句子里都有加粗的值，渲染处用 splitMessage 按占位符切分（见 i18n.svelte.ts）
  'wordfreq.plan.domains': '识别到分域 {count} 个',
  'wordfreq.plan.files': '文件 {count}',
  'wordfreq.plan.bytes': '体积 {value}',
  'wordfreq.plan.onlyThisDomain': '只统计 {domain}',
  /** `{field}` 是等宽显示的 ScanParams 字段名（不翻译） */
  'wordfreq.plan.ruleHint':
    '勾选分域 = 只统计这几项（对应 {field}）；不勾选任何一项表示统计全部分域。',
  'wordfreq.plan.noRule': '（未匹配到显式规则，按默认扩展名扫描）',
  'wordfreq.plan.col.include': '参与统计',
  'wordfreq.plan.col.domain': '分域',
  // 「文件」「体积」两列复用上面的 stat 文案（同页同义，不重复登记）
  'wordfreq.plan.col.rules': '解析规则',

  // ② 统计参数
  'wordfreq.step2.title': '② 配置统计参数',
  'wordfreq.step2.description': '这些参数会写进 meta.json，并作为设置页的默认值保存。',
  'wordfreq.step2.threads': '线程数（0 = 自动）',
  'wordfreq.step2.minCount': '最小词频',
  // 词库链：多选，**顺序有意义**（第一个是主词库）
  'wordfreq.step2.dictChain': '词库链',
  'wordfreq.step2.dictChainBadge': '顺序有意义',
  'wordfreq.step2.dictChainHint':
    '勾选要用的词库，**先勾的是主词库**；后面的依次叠加，同名条目以后面的为准。一个都不勾表示用 dicts\\ 里的全部 .dict（按文件名排序）。',
  'wordfreq.step2.dictPrimary': '主词库',
  'wordfreq.step2.dictOrder': '第 {index} 份',
  'wordfreq.step2.dictCheckAria': '使用词库 {name}',
  'wordfreq.step2.dictEffective': '按顺序装载：{names}',
  'wordfreq.step2.dictEffectiveAll': '未指定 → 使用全部可用词库（按文件名排序）：{names}',
  'wordfreq.step2.dictUseAll': '改用全部词库',
  'wordfreq.step2.dictsLoading': '正在读取词库清单…',
  'wordfreq.step2.noUsableDict':
    '数据文件夹里没有可用的词库。词库外置之后没有内置兜底，请先到「词库管理」导入一份 .dict，否则统计出来的是一张只有单字的废表。',
  'wordfreq.step2.goDicts': '词库管理',
  'wordfreq.step2.hmm': '开启 HMM 新词发现',
  'wordfreq.step2.hmmHint': '识别词典外的连续汉字组合',
  'wordfreq.step2.hmmAria': '开启 HMM',
  'wordfreq.step2.keepDigit': '保留数字词',
  'wordfreq.step2.keepDigitHint': '如 2024、3.14 作为独立词条',
  'wordfreq.step2.keepLatin': '保留拉丁词',
  'wordfreq.step2.keepLatinHint': '如 API、token 作为独立词条',
  'wordfreq.step2.skipSingleChar': '跳过单字词',
  'wordfreq.step2.skipSingleCharHint': '只统计词表中的多字词（字表仍单独产出）',
  'wordfreq.step2.domainTables': '产出分域表',
  'wordfreq.step2.domainTablesHint': '关闭后只产出全库表，体积更小',
  'wordfreq.step2.writeTsv': '写出可读 TSV',
  'wordfreq.step2.writeTsvHint': '方便直接用 Excel / 文本编辑器查看',
  'wordfreq.step2.writeTsvAria': '写出 TSV',

  // ③ 输出与执行
  'wordfreq.step3.title': '③ 表名与输出目录',
  'wordfreq.step3.description':
    '产物（meta.json / *.vfr / *.tsv）会写到输出目录。默认落在数据文件夹的 tables\\<表名> 下，也可以改成任意位置。',
  'wordfreq.step3.tableName': '表名',
  'wordfreq.step3.tableNamePlaceholder': '例如 2024 新闻语料',
  'wordfreq.step3.tableNameHint':
    '表名决定默认输出目录（<数据文件夹>\\tables\\<表名>），也是「词表管理」里那张表的名字。',
  'wordfreq.step3.newTable': '新的词表',
  'wordfreq.step3.defaultTableName': '{corpus} 语料表',
  'wordfreq.step3.resuggestOut': '按表名重新预填',
  'wordfreq.step3.outPlaceholder': '例如 <数据文件夹>\\tables\\我的表',
  'wordfreq.step3.outLabel': '输出目录',
  'wordfreq.step3.checkDataset': '检查产物',
  'wordfreq.step3.start': '开始统计',
  'wordfreq.step3.scanning': '统计中…',
  'wordfreq.step3.cancel': '取消',
  'wordfreq.step3.openOutDir': '打开输出目录',
  // 数据文件夹里没有可用词库时挡住「开始统计」
  'wordfreq.libNotReady.title': '没有可用的词库，无法开始统计。',
  'wordfreq.libNotReady.body':
    '词库不再编在程序里：数据文件夹的 dicts\\ 里至少要有一份能读的 .dict，否则跑出来的是一张只有单字的废表。',
  'wordfreq.libNotReady.go': '去词库管理',
  'wordfreq.scanFailed': '统计失败：{error}',
  'wordfreq.needCorpus': '请先填写或选择语料库目录。',
  'wordfreq.pickCorpusDir': '选择语料库目录',
  'wordfreq.pickOutDir': '选择输出目录',
  'wordfreq.settingsSaved': '参数已保存到设置',
  'wordfreq.settingsSaveFailed': '参数保存失败：{error}',
  'wordfreq.logSubmitted': '已提交统计任务：{corpus} → {out}',
  'wordfreq.logCancelled': '已请求取消统计任务。',
  'wordfreq.logDone': '统计完成，结果摘要已生成。',
  'wordfreq.logOpenFailed': '产物装载失败（其它页面可能还不识别）：{error}',
  'wordfreq.datasetUnavailable': '产物目录暂不可用：{dir}',
  'wordfreq.datasetLoaded': '已装载产物目录：{dir}（{tables} 张表）',
  'wordfreq.datasetLoadFailed': '已识别产物目录：{dir}，但装载失败：{error}',

  // 进度、计划与日志
  'wordfreq.stat.files': '文件',
  'wordfreq.stat.bytes': '体积',
  'wordfreq.stat.lines': '行数',
  'wordfreq.stat.paras': '段落',
  'wordfreq.stat.badLines': '跳过行',
  'wordfreq.progress.preparing': '准备中',
  'wordfreq.progress.domain': '· 当前分域 {domain}',
  'wordfreq.progress.bytes': '已处理 {done} / {total}',
  'wordfreq.progress.units': '单元 {done} / {total}',
  'wordfreq.planEvent.title': '扫描计划',
  'wordfreq.planEvent.summary': '共 {files} 个文件 / {bytes}，分域：',
  'wordfreq.tables.title': '已写出的表',
  'wordfreq.tables.entries': '{entries} 词条 · {tokens} token',
  'wordfreq.tables.summary': '{entries} 词条 · {tokens} token · .vfr {bytes}',
  'wordfreq.logs.title': '日志',

  // 结果卡片
  'wordfreq.result.title': '统计完成',
  'wordfreq.result.meta':
    '生成于 {generated} · 耗时 {elapsed} · 工具版本 {tool} · schema v{schema}',
  'wordfreq.result.engine': '分词引擎 {engine} {version}',
  'wordfreq.result.hmm': 'HMM {value}',
  'wordfreq.result.digits': '数字词 {value}',
  'wordfreq.result.latin': '拉丁词 {value}',
  'wordfreq.result.skipSingle': '跳过单字 {value}',
  /** `dicts[0]` 是主词库；v1 的 `dict` / `user_dict` 已废弃，不再单独显示 */
  'wordfreq.result.dicts': '词库链 {names}',
  'wordfreq.result.done':
    '统计完成 ✅ 现在可以去「{sentences}」页粘贴文本，查看每个词的频率分组；也可以在「{leaderboard}」页浏览完整排行。',

  // ---------------------------------------------------------------- 划句分析页
  'sentences.browserPreview':
    '浏览器预览模式：正在使用内置演示数据，「发到悬浮小窗 / 手动取词」需要在桌面端运行。',
  'sentences.checking': '正在检查语料库产物…',
  'sentences.statusFailed': '读取数据集状态失败：{error}',

  // 没有可用产物时的引导
  'sentences.noTable.title': '还没有可用的词频表',
  'sentences.noTable.description':
    '划句分析依赖「生成词频表」产出的 meta.json 与 .vfr 索引表。当前检查的目录：',
  'sentences.noTable.noDir': '（未设置输出目录）',
  'sentences.noTable.step1': '去「生成词频表」页选择语料库并开始统计',
  'sentences.noTable.step2': '统计完成后回到本页，这里会自动加载新产物',
  'sentences.noTable.go': '去生成词频表',
  'sentences.noTable.goDicts': '去词库管理',
  'sentences.noTable.recheck': '重新检查',

  // 产物未装载到后端
  'sentences.notLoaded.title': '产物目录尚未装载到后端',
  'sentences.notLoaded.p1WithError':
    '已找到 meta.json（{dir}），但 `open_dataset` 未成功：{error}。',
  'sentences.notLoaded.p1NoError': '已找到 meta.json（{dir}），但 `open_dataset` 未成功。',
  'sentences.notLoaded.p2': '在 Rust 侧实现该命令前，划句分析会返回「未收录」。',

  // 数据集概览
  'sentences.dataset.title': '数据集',
  'sentences.dataset.ready': '已就绪',
  'sentences.dataset.hmm': 'HMM {value}',
  'sentences.dataset.dictChain': '词库链 {count} 份：{names}',
  'sentences.dataset.summary':
    '{generated} 生成 · 全库 {tokens} token · 词表 {wordTable} · 字表 {charTable}',
  'sentences.tableEntries': '{entries} 条',

  // 文本输入
  'sentences.input.title': '文本输入',
  'sentences.input.modeAll': '分析全文',
  'sentences.input.modeSelection': '只分析选中',
  'sentences.input.description': '粘贴文字，或在文本框里划选一段文字；下方的着色结果会实时更新。',
  'sentences.input.placeholder': '在这里粘贴要分析的中文文本……',
  'sentences.input.counter': '{chars} 字 · {lines} 行',
  'sentences.input.selected': '已选 {count} 字',
  'sentences.input.selectHint': '（请在文本框中划选一段文字）',
  'sentences.input.scopeAll': '分析范围：全文',
  'sentences.input.analyzing': '分析中…',

  // 分域过滤
  'sentences.domains.title': '对比范围',
  'sentences.domains.hint': '对比列里显示哪些作用域；不选 = 全显示。着色只看主表。',
  'sentences.domains.selectAll': '全选',
  'sentences.domains.entries': '{entries} 条',
  /** 「清空」按钮在分域过滤与操作行各出现一次，同页同义，共用一条 */
  'sentences.clear': '清空',

  // 操作
  'sentences.actions.sendToPopup': '发到悬浮小窗',
  'sentences.actions.capture': '手动取词',
  'sentences.actions.copy': '复制全文',

  // 分析结果
  'sentences.result.title': '分析结果',
  'sentences.result.tokenCount': '{count} token',
  'sentences.result.accepted': '计入统计 {count}',
  'sentences.result.skipped': '标点/空白 {count}',
  'sentences.result.unknown': '未收录 {unique} 种 / {total} 次',
  'sentences.result.description':
    '悬停任意词，右侧「词条详情」面板显示它的频次、排名、前 %、占比与分域排名；点击词条可钉住详情。',
  'sentences.result.thresholdHint':
    '词表与字表的七组阈值都可以在「表管理」页自定义，这里按生效阈值着色。',
  'sentences.result.averageBaseline': '平均每 token 占比基准：词表 {word} · 字表 {char}',
  'sentences.empty.selection': '请在文本框中划选一段文字，或切换到「分析全文」。',
  'sentences.empty.none': '暂无可分析的内容，先粘贴一段文字试试。',

  // 词条详情面板
  'sentences.detail.aria': '词条详情',
  'sentences.detail.title': '词条详情',
  'sentences.detail.pinned': '已钉住',
  'sentences.detail.hint': '悬停查看 · 点击钉住',
  'sentences.detail.emptyHint':
    '悬停左侧任意词条，这里会固定显示它的频次、排名、前 %、占比、分组与各分域排名。',
  'sentences.detail.pinnedHint': '钉住后悬停别的词不会改变这里',
  'sentences.detail.unpin': '取消钉住',

  // 提示（toast）
  'sentences.notice.emptyNoSend': '文本框是空的，没有可发送的内容。',
  'sentences.notice.sentToPopup': '已把全文发送到悬浮小窗。',
  'sentences.notice.captureEmpty': '当前没有检测到全局选中的文本。',
  'sentences.notice.captureOk': '已取到全局选中的文本并重新分析。',
  'sentences.notice.emptyText': '文本框是空的。',
  'sentences.notice.copied': '全文已复制到剪贴板。',

  // ---------------------------------------------------------------- 七组分组标签
  // key 必须与 crates/vocfreq-core/src/rank.rs::TIER_KEYS 一致（顺序也一致）
  'tier.very_common': '极多',
  'tier.common': '很多',
  'tier.fairly_common': '较多',
  'tier.medium': '中等',
  'tier.fairly_rare': '较少',
  'tier.rare': '很少',
  'tier.very_rare': '极少',
  /** 第 8 种状态：语料库未收录（与「极少」不同，那一组是有排名的真实分组） */
  'tier.unknown': '未收录',

  // ---------------------------------------------------------------- 生效阈值回退警告
  // 这些是**错误码对应的文案**：错误码本身在 `types.ts::BoundsWarning`，
  // `format.ts::boundsInfo` 只回 `{ key, params }`，渲染交给展示层。
  // 词表/字表分成两条（而不是 `{kind}阈值…`）：省掉展示层的嵌套查表，
  // 也让英文翻译能按语境各自措辞。
  'bounds.invalidWord': '词表阈值必须是非负整数、严格递增、共 {count} 个，已临时回退到 meta 默认值。',
  'bounds.invalidChar': '字表阈值必须是非负整数、严格递增、共 {count} 个，已临时回退到 meta 默认值。',
  'bounds.invalidPct': '前%上界必须是 0~100 之间、严格递增、共 {count} 个，已临时回退到这张表的默认口径。',
  'bounds.evenNoEntries': '这张表的词条数是 0，无法按词条数等分，已回退到 meta 默认值。',
  'bounds.evenInvalid': '按词条数等分的结果不合法（词条数太少），已回退到 meta 默认值。',
  'bounds.coverageInvalid': '覆盖率目标必须严格递增、且都在 0~100% 之间，已回退到 meta 默认值。',
  'bounds.coverageNoCurve': '还没有拿到覆盖率曲线，正在用 meta 默认阈值显示。',
  'bounds.coverageUnsolvable':
    '这组覆盖率目标反解不出严格递增的排名阈值（目标太接近或超出曲线范围），已回退到 meta 默认值。',
  /** 带前缀的形式（「词条详情」面板用）：`{message}` 是上面某一条 */
  'bounds.fallbackNotice': '生效阈值有回退：{message}',

  // ---------------------------------------------------------------- 词库管理页
  // 词库外置之后 `dicts\` 里那些 .dict 文件的管理界面。
  // 「预置」只是个展示徽标，**没有任何权限等级**：预置项同样可以删。
  'dicts.browserPreview':
    '浏览器预览模式：下面是一份演示数据文件夹（含一份坏文件与一份有隐患的词库），导入 / 删除只作用于内存。',
  'dicts.loading': '正在读取数据文件夹…',
  'dicts.loadFailed': '读取词库清单失败：{error}',
  'dicts.origin.seeded': '预置',
  'dicts.origin.imported': '导入',
  'dicts.origin.scanned': '自己扫',
  'dicts.origin.unknown': '未知',

  // 数据文件夹
  'dicts.dataDirTitle': '数据文件夹',
  'dicts.dataDirDescription':
    '词库（dicts\\）与词表（tables\\）都住在这里。换文件夹只建目录、不搬运已有内容，并且会清掉当前激活的表。',
  'dicts.defaultLocation': '默认位置',
  'dicts.readyBadge': '有可用词库',
  'dicts.notReadyBadge': '没有可用词库',
  'dicts.openDataDir': '打开所在文件夹',
  'dicts.openDictsDir': '打开 dicts\\',
  'dicts.changeDataDir': '更换数据文件夹',
  'dicts.dataDirLayout': '词库目录：{dicts} · 词表目录：{tables}',
  'dicts.dataDirChanged': '数据文件夹已改为 {dir}。当前激活的表已被清空，请到「表管理」页重新激活一张。',
  'dicts.pickDataDir': '选择数据文件夹',
  'dicts.noUsableDict':
    '这里一份能读的词库都没有，无法开始统计 —— 词库不再编在程序里，没有内置兜底。请先导入一份 .dict。',
  'dicts.readyMismatch': '提示：界面上的可用词库判断与后端 library_ready 的结果不一致，请重新检查一次。',

  // 词库清单
  'dicts.listTitle': '词库清单',
  'dicts.countBadge': '{count} 份',
  'dicts.listDescription':
    '一份词库就是一个 .dict 文件（jieba 的「词 词频 词性」，每行一条）。文件是普通文本，可以直接用记事本改，改完的内容指纹会变，依赖它的表会显示「词库已变」。',
  'dicts.empty': '数据文件夹里还没有任何 .dict 文件。点右上角「导入词库」放一份进来。',
  'dicts.import': '导入词库',
  'dicts.pickDictFile': '选择词库文件（.dict）',
  'dicts.dictFilter': 'jieba 词库',
  'dicts.imported': '已导入：{name}',
  'dicts.entries': '{count} 条有效词条',
  'dicts.brokenBadge': '读不了',
  'dicts.error': '文件有问题：{error}',
  'dicts.revealFile': '打开所在文件夹',
  'dicts.delete': '删除',
  'dicts.deleted': '已删除词库 {name}。',
  'dicts.deletedWithUsers':
    '已删除词库 {name}。这些表引用了它，现在会显示「词库缺失」：{tables}。',
  'dicts.usedByTables': '被这些表引用：{tables}',

  // 隐患提示
  'dicts.warnFreqZero':
    '{count} 条把词频写成了 0。jieba 那一列是**概率权重**不是词频，写成 0 的词**永远切不出来**，只会在词表里占个位置。想留它们就把数字改成正数，不想留就删掉那几行。',
  'dicts.warnFreqOmitted':
    '{count} 条没有写词频列。装载时按 suggest_freq 折算，权重通常比显式写出来的更低，实际排序可能和你的预期不同。',
  'dicts.skippedLines': '跳过 {comments} 行注释、{blanks} 行空行。',

  // 删除确认（就地展开）
  'dicts.confirmDeleteTitle': '确认删除词库「{name}」？',
  'dicts.confirmDeleteUsed':
    '有 {count} 张表用到了它：{tables}。删掉之后这些表会变成「词库缺失」，分词口径与建表时不一致，频次不可信 —— 需要在「表管理」里对它们重新统计。',
  'dicts.confirmDeleteNote':
    '文件会从数据文件夹里直接删除，不进回收站，无法撤销。预置词库也可以删（这里没有权限等级的区别）。',
  'dicts.confirmDeleteYes': '确认删除',
  'dicts.noPermissionNote':
    '说明：预置词库和用户自己导入的完全是同一种东西，没有权限等级，都可以删除或改名。上方的来源徽标只是用来告诉你它是怎么来的。',

  // ---------------------------------------------------------------- 表管理页
  'tables.previewCapNote': '有 {skipped} 个阈值大于这张表的词条数（{entries} 条），预览里已夹到词条总数；真实分组判定不受影响。',

  // 浏览器预览 / 加载 / 空态 / 未装载
  'tables.browserPreview':
    '浏览器预览模式：表清单与开关读写的是内置演示数据（仅内存），分组设置同样只保存在内存里。',
  'tables.loading': '正在读取产物目录与设置…',
  'tables.loadFailed': '读取数据集状态失败：{error}',
  'tables.emptyTitle': '还没有可管理的表',
  'tables.emptyDescription':
    '表清单来自 meta.json（`meta.tables`）。请先在「生成词频表」页产出一份结果。',
  'tables.goWordFreq': '去生成词频表',
  'tables.recheck': '重新检查',
  // 产物未装载到后端（与 sentences.notLoaded.* 同构，语境不同各留一份）
  'tables.notLoaded.title': '产物目录尚未装载到后端',
  'tables.notLoaded.p1WithError': '已找到 meta.json（{dir}），但 `open_dataset` 未成功：{error}。',
  'tables.notLoaded.p1NoError': '已找到 meta.json（{dir}），但 `open_dataset` 未成功。',
  'tables.notLoaded.p2': '表清单仍可管理，但 `tier_curve` 可能拿不到曲线。',

  // A. 频率表清单（全部作用域，完全平级）
  'tables.listTitle': '频率表',
  'tables.tableCountBadge': '{count} 张表',
  'tables.scopeCountBadge': '{count} 个作用域',
  'tables.primaryBadge': '主表：{scope}',
  'tables.primaryDescription':
    '每个作用域各有一张词表 + 一张字表，它们**完全平级**（`full` 只是"所有域加在一起"的那一个）。{emphasis}，划句分析的着色与分组、排行榜、分组阈值预览都以它为准，其余表只做对比。',
  'tables.primaryEmphasis': '勾选「主表」决定"这个词有多常见"',
  'tables.equalNote':
    '表格里每张表都是同一份产物里的一个作用域，没有全库表与分域表的区别。想换一个"最常见"的基准就改主表；想把几个域合成一张新表就用下面的「相加」。',
  'tables.primarySet': '主词频表已切换为「{scope}」。',
  'tables.primaryFailed': '切换主表到「{scope}」失败：{error}',
  'tables.primaryAria': '把 {scope} 设为主词频表',
  'tables.primaryBadgeShort': '主表',
  'tables.enabledCountBadge': '已启用 {count} 张',
  'tables.saving': '保存中…',
  'tables.quickSelect': '快捷选择',
  'tables.selectAll': '全选',
  'tables.selectNone': '全不选',
  'tables.onlyFull': '只留全库',
  'tables.onlyWord': '只留词表',
  'tables.onlyChar': '只留字表',
  'tables.resetAllEnabled': '重置为全部启用',
  'tables.col.scope': '作用域',
  'tables.col.kind': '类型',
  'tables.col.entries': '词条数',
  'tables.col.tokens': '总 token',
  'tables.col.vfr': '.vfr 体积',
  'tables.col.lastCoverage': '最后一组覆盖率',
  'tables.col.source': '来源',
  'tables.col.primary': '主表',
  'tables.sourceScanned': '扫描产出',
  'tables.sourceComposed': '相加（{count} 张）',
  'tables.countHint':
    '提示：扫描时用了 `--skip-domain-tables` 或只统计了部分分域时，作用域会少一些。完整产物是「全量 + 每个一级子目录」各一张词表 + 一张字表。',

  // B. 相加
  'tables.compose.title': '相加',
  'tables.compose.description':
    '把若干张表加起来成一张新表（结果写在同一个产物目录里，作为另一个平级作用域）。相加是**精确**的：扫描本身就是逐作用域扫完再累加，所以各域表相加逐条等于全量扫描出来的那张表。',
  'tables.compose.pickLabel': '要相加的表',
  'tables.compose.pickedBadge': '已选 {count} 张',
  'tables.compose.nameLabel': '新作用域名',
  'tables.compose.namePlaceholder': '例如：相加：新闻与维基',
  'tables.compose.defaultName': '相加',
  'tables.compose.run': '相加',
  'tables.compose.running': '相加中…',
  'tables.compose.needPick': '先勾选至少一张要相加的表。',
  'tables.compose.mixedKinds': '一次只能相加同一类（词表或字表）。分两次相加即可。',
  'tables.compose.done': '已相加出作用域「{scope}」：{kinds}，{entries} 条。',
  'tables.compose.doneShort': '已相加出「{scope}」',
  'tables.compose.note':
    '代价：磁盘上会多出一份表（大约等于一张扫描出来的表）。源表不需要了可以在上面的词表管理里删掉回收空间；新表本身也可以再被相加。',

  // A2. 词表管理（数据文件夹 tables\）+ 词库绑定状态
  //   这是词库外置之后最关键的一块：词库能被用户随手改，改了以后旧表的频次就
  //   跟分词口径对不上了，所以绑定状态必须显眼，并给一键重新统计。
  'tables.library.title': '词表管理',
  'tables.library.countBadge': '{count} 张表',
  'tables.library.description':
    '数据文件夹 tables\\ 下的词表，外加"指向数据文件夹之外的产物目录"那一张。每张表都记录着它建表时用的词库链（带 sha256 指纹），这里是校验结果。',
  'tables.library.activeBadge': '当前激活：{name}',
  'tables.library.dataDir': '数据文件夹：{dir}',
  'tables.library.loadFailed': '读取词表清单失败：{error}',
  'tables.library.empty': '数据文件夹里还没有词表。去「生成词频表」页统计一次，产物就会落到这里。',
  'tables.library.active': '激活中',
  'tables.library.external': '数据文件夹之外',
  'tables.library.corpus': '语料库：{corpus}',
  'tables.library.generated': '生成于 {generated}',
  'tables.library.tableError': '这张表读不全：{error}',
  'tables.library.entriesUnavailable': '词条数未知',
  'tables.library.activate': '激活',
  'tables.library.rescan': '重新统计',
  'tables.library.rescanSuggestion': '一键重新统计',
  'tables.library.openDir': '打开所在文件夹',
  'tables.library.delete': '删除',
  'tables.library.activated': '已激活「{name}」，分词器已按它记录的词库链重建。',
  'tables.library.activateFailed': '激活「{name}」失败：{error}',
  'tables.library.deleted': '已删除词表「{name}」。',
  'tables.library.deleteFailed': '删除「{name}」失败：{error}',
  'tables.library.confirmDeleteTitle': '确认删除词表「{name}」？',
  'tables.library.confirmDeleteNote':
    '整个产物目录会被删掉（meta.json 与全部 .vfr / .tsv），无法撤销。数据文件夹之外的表不给删除入口 —— 那个目录不归本程序管。',
  'tables.library.confirmDeleteYes': '确认删除',
  'tables.library.activeBinding': '当前激活表的词库绑定：{label}',
  'tables.library.dictChain': '词库链：{chain}',
  'tables.library.absentDicts': '（数据文件夹里找不到：{names}）',

  // 绑定状态（`Binding` 的四种取值）
  'tables.binding.ok': '词库一致',
  'tables.binding.legacy': '老产物，无词库记录，无从校验',
  'tables.binding.legacyDetail':
    '这是 schema v1 产物：当年词库编在程序里，产物只留了一句自由文本描述，没有指纹，所以既不能说它一致、也不能说它不一致。',
  'tables.binding.drifted': '词库已变，频次可能不准',
  'tables.binding.driftedDetail': '内容变了的词库：{changed}。分词口径已与建表时不同，建议重新统计。',
  'tables.binding.missing': '找不到词库',
  'tables.binding.missingDetail': '数据文件夹里已经没有这些词库：{missing}。重新统计前请先补回或改掉词库链。',

  // B. 分组自定义
  'tables.tierConfigTitle': '分组自定义',
  'tables.tierCountBadge': '七组',
  'tables.rankUpperBound': '排名上界',
  /** 说明句：{names} 是七组名；句子里有一处 <b> 加粗，用 splitMessage 切 {emphasis} */
  'tables.tierConfigDescription':
    '七组的名字固定（{names}），这里定的是每组的{emphasis}：第 1..6 组各有一个上界，第 7 组自动是「以上全部」。',
  'tables.methodLabel': '分组方法',
  'tables.method.topPct': '按前%（默认）',
  'tables.method.rank': '按排名',
  'tables.method.coverage': '按覆盖率',
  'tables.method.even': '按词条数等分',
  'tables.whatLabel': '在做什么：',
  'tables.whenLabel': '什么时候用：',
  'tables.methodNote.topPct.what':
    '给七组定「前百分之几」的上界：前% = 排名 ÷ 该表条目总数 × 100。第 1 组 = 前 0.0026% 的词条，依此类推，第 7 组是剩下的全部。',
  'tables.methodNote.topPct.when':
    '默认口径。它只跟"位次"有关、与表的规模无关，所以换一张表（分域表几万条、全量表几百万条）甚至换主表，色阶的含义都不变。排名绝对值做不到这一点：同一个阈值套在小表上会让整片词条挤进「极多」。',
  'tables.methodNote.rank.what':
    '直接给七组定排名上界：第 1 组 = 排名 1..N₁，第 2 组 = N₁+1..N₂ …… 第 7 组 = 最后一个上界以上全部。',
  'tables.methodNote.rank.when':
    '最直观，适合口径固定、只在一张表内看色阶的场景；但阈值是为某一张表的规模校准的，换个规模差很多的表就会失真。',
  'tables.methodNote.coverage.what':
    '给定「每组累计覆盖正文的百分比」，程序用覆盖率曲线反解出对应的排名上界（在 log10(rank) 上插值）。',
  'tables.methodNote.coverage.when':
    '关心「前 50% 正文由多少词覆盖」这类问题时用；代价是各组的大小差异会很大，而且必须先把覆盖率曲线读一遍。',
  'tables.methodNote.even.what': '简单粗暴地按词条数七等分：每组词条数几乎相同。',
  'tables.methodNote.even.when':
    '想让每组样本量接近（抽样、统计检验）时用；与「常用 / 生僻」的直觉完全无关。',
  'tables.pctBoundsTitle': '前%上界（%）',
  'tables.pctBoundsHint': '第 7 组固定是「以上全部」，所以只填 6 个，且必须严格递增',
  'tables.pctBoundAria': '第 {index} 组前%上界',
  'tables.pctBoundsSaved': '前%上界已保存。',
  'tables.pctBoundsReset': '已恢复为这张表自带的默认前%口径。',
  'tables.pctNote':
    '换算用的是**主表**的词表条目数（当前 {entries} 条）：前%上界 × 条目数 = 排名上界。换成条目数不同的表，同一组前%会得到不同的排名阈值 —— 这正是它比绝对排名更可比的原因。',
  'tables.solveFromPct': '换算成排名阈值并切到「按排名」',
  'tables.coverageTargetLabel': '累计覆盖率目标（%）',
  'tables.coverageTargetHint': '第 7 组固定是「以上全部」，所以只填 6 个，且必须严格递增',
  'tables.solveButton': '一键用当前覆盖率反解成排名阈值',
  'tables.resetDefault': '恢复默认',
  'tables.groupN': '第 {index} 组',
  'tables.groupCoverageAria': '第 {index} 组累计覆盖率',
  'tables.curveLoadFailed': '曲线加载失败：{error}',
  'tables.curveLoading': '正在读取覆盖率曲线…',
  'tables.curvePoints': '{count} 点',
  'tables.curveSummary':
    '曲线：{word}（词表）· {char}（字表）· Rust 侧按产物缓存，重复进入本页不会重算。',
  'tables.solvedWordTitle': '词表反解结果',
  'tables.solvedCharTitle': '字表反解结果',
  'tables.groupUpperLe': '第 {index} 组 ≤',
  'tables.wordBoundsTitle': '词表阈值',
  'tables.charBoundsTitle': '字表阈值',
  'tables.sixBoundsHint': '6 个排名上界',
  'tables.boundAria': '{kind}第 {index} 组排名上界',
  'tables.group7Note': '第 7 组 = 「以上全部」（排在 {count} 名之后的所有词条）',
  'tables.previewTitle': '实时预览',
  'tables.previewHint':
    '改上面的数字会立刻重算；「组内词条数」在无曲线时是估算值，累计覆盖率取自曲线时是精确的',
  'tables.wordPreview': '词表预览',
  'tables.charPreview': '字表预览',
  'tables.preview.col.name': '组名',
  'tables.preview.col.range': '阈值',
  'tables.preview.col.entries': '组内词条数',
  'tables.preview.col.coverage': '本组覆盖率',
  'tables.preview.col.cumulative': '累计覆盖率',
  'tables.preview.col.source': '来源',
  'tables.sourceCurve': '曲线',
  'tables.sourceEstimated': '按分档比例估算',
  'tables.legendTitle': '图例效果（词表生效阈值）',
  'tables.applyNote':
    '设置即时生效并已保存：划句分析页的词条着色、悬停浮层与排行榜的分组列都按这份阈值重算。三个分组字段全是 null / \'rank\' 时，界面与 meta 默认分组完全一致。',

  // 操作提示（toast）
  'tables.saveFailed': '保存失败：{error}',
  'tables.methodSwitched': '分组方法已切换为「{method}」。',
  'tables.wordBoundsSaved': '词表阈值已保存。',
  'tables.charBoundsSaved': '字表阈值已保存。',
  'tables.coverageSaved': '覆盖率目标已保存。',
  'tables.coverageReset': '已恢复默认覆盖率目标（取自 meta 的累计覆盖率）。',
  'tables.wordBoundsReset': '词表阈值已恢复为 meta 默认值。',
  'tables.charBoundsReset': '字表阈值已恢复为 meta 默认值。',
  'tables.solveFailed': '拿不到覆盖率曲线，无法反解。',
  'tables.solveFailedWithError': '拿不到覆盖率曲线，无法反解：{error}。',
  'tables.solved': '已用覆盖率曲线反解成排名阈值（词表：{ranks}），并切回「按排名」。',

  // ---------------------------------------------------------------- 排行榜页
  'leaderboard.browserPreview':
    '浏览器预览模式：正在使用内置演示数据，翻页 / 搜索 / 分组筛选都可以直接体验。',
  'leaderboard.checking': '正在检查语料库产物…',
  'leaderboard.statusFailed': '读取数据集状态失败：{error}',
  'leaderboard.noTable.title': '还没有可用的词频表',
  'leaderboard.noTable.description': '排行榜需要先生成频率表。当前检查的目录：',
  'leaderboard.noDir': '（还没有打开任何词表）',
  'leaderboard.goWordFreq': '去生成词频表',
  'leaderboard.goDicts': '去词库管理',
  'leaderboard.recheck': '重新检查',
  // 产物未装载到后端（与 tables.notLoaded.* 同构；原模板的「未成功 。。」里那个换行折出的
  // 空格和重复句号是源稿瑕疵，这里按干净形式写，见交付说明）
  'leaderboard.notLoaded.title': '产物目录尚未装载到后端',
  'leaderboard.notLoaded.p1WithError': '已找到 meta.json（{dir}），但 `open_dataset` 未成功：{error}。',
  'leaderboard.notLoaded.p1NoError': '已找到 meta.json（{dir}），但 `open_dataset` 未成功。',
  'leaderboard.notLoaded.p2': '在 Rust 侧实现该命令前，列表接口可能返回空结果。',

  // 控制区
  'leaderboard.title': '排行榜',
  'leaderboard.full': '全库',
  'leaderboard.scope': '作用域',
  'leaderboard.primaryScope': '主词频表（{scope}）',
  'leaderboard.primaryBadge': '主表',
  'leaderboard.summary': '{generated} 生成 · 当前表 {entries} 条目 · {tokens} token',
  'leaderboard.domain': '域',
  'leaderboard.searchPlaceholder': '按词首前缀搜索，例如「语」',
  'leaderboard.searchAria': '前缀搜索',
  'leaderboard.searching': '搜索中…',
  'leaderboard.tierFilter': '分组筛选',
  'leaderboard.tierFilterHint':
    '仅作用于当前已加载的 {count} 条（后端按排名分页，无法跨页筛选）',
  'leaderboard.clearFilter': '清除筛选',

  // 排名列表
  'leaderboard.rankListTitle': '排名列表',
  'leaderboard.filteredCount': '筛选后 {count} 条',
  'leaderboard.loading': '加载中…',
  'leaderboard.rowHint': '点击任意一行可「加入分析」，把该词送到划句分析页。',
  'leaderboard.col.rank': '排名',
  'leaderboard.col.word': '词',
  'leaderboard.col.count': '频次',
  'leaderboard.col.share': '占比',
  'leaderboard.col.top': '前%',
  'leaderboard.col.topTitle': '排名 ÷ 当前表的条目总数',
  'leaderboard.col.shareTitle': '占比 = 该词频次 ÷ 全库表总 token 数（不是词条数的比例）',
  'leaderboard.col.tier': '分组',
  'leaderboard.col.flags': '标记',
  'leaderboard.col.actions': '操作',
  'leaderboard.rowTitle': '点击加入划句分析',
  'leaderboard.charNoFlag': '字表无词典标记',
  'leaderboard.inDict': '词典内',
  'leaderboard.flagOutOfDict': '词典外',
  'leaderboard.flagUserDict': '用户词典',
  'leaderboard.detail': '详情',
  'leaderboard.copy': '复制',
  'leaderboard.emptySearch': '没有以「{query}」开头的词条。',
  'leaderboard.emptyFiltered': '当前页没有符合分组筛选的词条，试试翻页或清除筛选。',
  'leaderboard.emptyPage': '这一页没有数据。',
  'leaderboard.searchResultCount': '搜索结果 {count} 条',
  'leaderboard.rankRange': '排名 {from}–{to} / 共 {total}',

  // 分页
  'leaderboard.firstPage': '首页',
  'leaderboard.prevPage': '上一页',
  'leaderboard.pageLabel': '第',
  'leaderboard.pageAria': '页码',
  'leaderboard.totalPages': '/ {count} 页',
  'leaderboard.nextPage': '下一页',
  'leaderboard.lastPage': '末页',
  'leaderboard.perPage': '每页 {count} 条',

  // 查词详情
  'leaderboard.detailTitle': '词条详情',
  'leaderboard.close': '关闭',
  'leaderboard.groupN': '第 {index} 组',
  'leaderboard.detail.top': '前',
  'leaderboard.detail.topTitle': '排名 ÷ 该表词条总数',
  'leaderboard.detail.shareTitle': '该词频次 ÷ 全库表总 token 数',
  'leaderboard.detail.dict': '词典',
  'leaderboard.detail.inDict': 'jieba 词典内',
  'leaderboard.detail.outDict': '词典外',
  'leaderboard.addToAnalysis': '加入分析',
  'leaderboard.copyWord': '复制词',
  'leaderboard.detailLoading': '查询中…',
  'leaderboard.addedToAnalysis': '已加入划句分析：{word}',
  'leaderboard.copied': '已复制：{word}',

  // ---------------------------------------------------------------- 悬浮小窗
  'popup.title': 'VocTier 取词',
  'popup.noTable': '无词频表',
  'popup.themeTitle': '主题：{hint}',
  'popup.themeAria': '切换主题',
  'popup.copyAllTitle': '复制全文',
  'popup.copy': '复制',
  'popup.sendTitle': '发送到主窗口（划句分析页）',
  'popup.sendBack': '发回主窗口',
  'popup.hideTitle': '隐藏小窗',
  'popup.closeAria': '关闭小窗',
  'popup.inputPlaceholder': '粘贴或输入要查词的中文…',
  'popup.clearTitle': '清空',
  'popup.clear': '清空',
  'popup.loading': '正在载入词频表…',
  'popup.noTableDesc': '还没有可用的词频表，取词结果无法着色。',
  'popup.noTableHint': '请先在主窗口的「生成词频表」页完成一次统计。',
  'popup.emptyHint': '输入或粘贴文字后，这里会实时显示每个词的分组着色。',
  'popup.detailAria': '词条详情',
  'popup.detailTitle': '词条详情',
  'popup.unpin': '取消钉住',
  'popup.summary': '{accepted} 词 · 未收录 {unknown} 种',
  /** meta 存在时才追加的表名段（与 popup.summary 拼成一行） */
  'popup.summaryTable': ' · 词表 {entries} 条',
  'popup.detailEmpty': '还没有可显示的词条。',
  'popup.copyEmpty': '没有可复制的内容',
  'popup.copied': '已复制',
  'popup.sendEmpty': '没有可回传的内容',
  'popup.sentToMain': '已发送到主窗口',

  // ---------------------------------------------------------------- 词条详情（TokenDetail）
  'tokenDetail.emptyHint': '悬停上方任意词条，这里会显示它的频次、排名与分组详情。',
  'tokenDetail.pctNoRecord': '该词条在全库表里没有记录，因此没有占比。',
  'tokenDetail.fullWord': '全库词表',
  'tokenDetail.fullChar': '全库字表',
  'tokenDetail.pctNoTotal': '占比 = 该词条出现次数 ÷ {which}的总 token 数',
  'tokenDetail.pct': '占比 = 该词条出现次数 {count} ÷ {which}总 token 数 {total}',
  'tokenDetail.freq': '频次',
  'tokenDetail.rank': '排名',
  'tokenDetail.top': '前',
  'tokenDetail.topTitle': '排名 ÷ 主词频表的条目总数',
  'tokenDetail.topValue': '前 {pct}',
  'tokenDetail.share': '占比',
  'tokenDetail.dictUnknown': '词典信息未知',
  'tokenDetail.inDict': 'jieba 词典内',
  'tokenDetail.outDictHmm': '词典外（HMM 新词）',
  'tokenDetail.fromUserDict': '来自用户词典',
  'tokenDetail.pinned': '已钉住',
  'tokenDetail.skipped': '标点 / 空白，不参与统计',
  'tokenDetail.notCollected':
    '语料库未收录：词表 / 字表里都没有这个词条，因此没有频次、排名与分组（与「极少」不同，那一组是有排名的真实分组）。',
  'tokenDetail.rangeLine': '{group} · 生效排名上界 {range}',
  // 铺平之后"各分域"其实就是"各张表"：同一个词在不同作用域里的排名对比。
  // 一律显示前%，因为排名绝对值跨表不可比（分域表只有几万条、全量表有几百万条）。
  'tokenDetail.tableRanksTitle': '各表对比（前%）',

  // ---------------------------------------------------------------- 分组统计表（TierStatsTable）
  'tierStats.col.group': '分组',
  'tierStats.col.upper': '排名上界',
  'tierStats.col.entries': '词条数',
  'tierStats.col.tokens': 'token 数',
  'tierStats.col.coverage': '组内覆盖率',
  'tierStats.col.cumulative': '累计覆盖率',

  // ---------------------------------------------------------------- 桥接层用户可见错误串（bridge.ts）
  'bridge.noTauri': '当前不在桌面端运行，这个功能需要 VocTier 桌面应用。',
  'bridge.unknownVersion': '未知',
  'bridge.invalidDatasetDir': '该目录不是有效的 VocTier 产物目录。',
  'bridge.noMeta': '后端没有返回产物元数据。',
  'bridge.wordNotInCorpus': '语料库中未收录「{word}」',
  'bridge.popupUnavailable': '浏览器预览模式下无法打开悬浮小窗。',
  'bridge.noMainWindow': '浏览器预览模式下没有主窗口可回传。',
  'bridge.clipboardUnsupported': '当前环境不支持剪贴板写入。',
  'bridge.pickDirectory': '选择目录',
  'bridge.pickFile': '选择文件',

  // ---------------------------------------------------------------- 通用格式
  // 「—」（无数据占位）刻意不进消息表：它是排版符号，不是语言相关的文案。
  'format.allAbove': '以上全部',
  'format.durationMs': '{value} 毫秒',
  'format.durationSec': '{value} 秒',
  'format.durationMinSec': '{minutes} 分 {seconds} 秒',
  'format.durationHourMin': '{hours} 小时 {minutes} 分',

  // ---------------------------------------------------------------- 表与 token
  'table.word': '词表',
  'table.char': '字表',
  'table.none': '未查表',

  // ---------------------------------------------------------------- 布局骨架
  'layout.sidebarTagline': '字词频率分析',
  'layout.sidebarReady': '就绪',
  'layout.mainNav': '主导航',
  'layout.badge': '骨架',
  'layout.searchPlaceholder': '搜索词条（占位）',
  'layout.searchLabel': '搜索词条',
  'layout.stub.pending': '待实现',
  'layout.stub.hooksTitle': '接入指引',
  'layout.stub.hooksDesc': '后续业务逻辑建议挂载在这些位置',
} as const;
