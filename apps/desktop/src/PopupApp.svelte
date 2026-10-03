<script lang="ts">
  /**
   * PopupApp —— 悬浮小窗（窗口 label === 'popup'）。
   *
   * 无边框窗口，结构：
   *   顶部：可拖拽条（data-tauri-drag-region）+ 主题切换 + 操作按钮
   *   中部：输入框
   *   下部：上半 = 着色 token 卡片（可滚动），下半 = **固定尺寸的词条详情区**
   *         （固定高度、独立滚动，永远占满剩余空间）
   *
   * 缩放时的让位顺序（用户要求）：
   *   窗口变矮 → 先压缩「着色 token 卡片」，压到内容放不下（出现滚动条）那一刻；
   *   卡片触到 min-h-16 之后 → 才开始压缩下面的「词条详情」。
   *   实现：卡片 `flex-1 min-h-16`，详情 `h-48 min-h-24 shrink basis-auto`。
   *   flex 按比例收缩，卡片基数大所以先让位；两块都有下限，极矮时也是卡片先到底。
   *   实测：480×420 卡片出现滚动条，480×340 起详情才开始变矮。
   *
   * 为什么详情是固定区域而不是跟随鼠标的浮层：小窗默认只有 480×420，浮层一靠近
   * 窗口边缘就被裁掉，读不全。固定区域永远在窗口内，内容多了自己滚。
   *
   * 文本从哪来（两条路都走，互不覆盖）：
   *   1. 挂载时 `take_pending_selection()` 取一次（第一次打开小窗时用）；
   *   2. 之后只认后端推送的 `popup:text` 事件（`onPopupText`）——
   *      小窗关闭时只是隐藏、不销毁，JS 不会重新挂载，靠事件才能拿到第二次的文本。
   *
   * 「发回主窗口」按钮：走 Tauri 自带事件总线广播 `voctier:popup-text`
   * （见 bridge.ts 的 emitPopupReply），主窗口在 App.svelte 里 listen 同一事件。
   */
  import TokenChips from '$lib/components/analysis/TokenChips.svelte';
  import TokenDetail from '$lib/components/analysis/TokenDetail.svelte';
  import TierLegend from '$lib/components/analysis/TierLegend.svelte';
  import {
    activeDataset,
    analyzeText,
    copyText,
    emitPopupReply,
    fetchCaptureNote,
    getSettings,
    hidePopup,
    isTauri,
    onPopupNote,
    onPopupText,
    takePendingSelection,
  } from '$lib/api/bridge';
  import { findTable } from '$lib/format';
  import { cn } from '$lib/utils';
  import { setTheme, theme, themeLabel } from '$lib/theme.svelte';
  import { adoptLocaleFromSettings, t } from '$lib/i18n.svelte';
  import { nextThemeMode, themeToggleHint } from '$lib/theme-sync';
  import IconMoon from '$lib/components/icons/IconMoon.svelte';
  import IconSun from '$lib/components/icons/IconSun.svelte';
  import {
    activeBounds,
    activeTierIndex,
    appSettings,
    primaryTableKey,
    tierCurves,
    tierNamesOf,
  } from '$lib/tiers.svelte';
  import { tierKeysFrom } from '$lib/tier-colors';
  import { summarizeTokens } from '$lib/segments';
  import type { Meta, Settings, TokenInfo } from '$lib/types';

  let text = $state('');
  let tokens = $state<TokenInfo[]>([]);
  let meta = $state<Meta | null>(null);
  let settingsState = $state<Settings | null>(null);
  let loading = $state(true);
  let error = $state('');
  let notice = $state('');
  /** 最近一次取词为什么没取到（空串 = 取到了，或还没取过） */
  let captureNote = $state('');

  /**
   * 详情区里显示的 token —— 用**下标**记住，而不是 token 对象：
   * 重新分析会整批换掉 token 对象，用下标才能在结果刷新后继续指向同一个位置。
   * 鼠标移开时**不清空**（否则鼠标移向详情区去读的时候会闪没）。
   */
  let activeIndex = $state<number | null>(null);
  /** 钉住：点击某个 token 后，悬停别的词不再改变详情区 */
  let pinnedIndex = $state<number | null>(null);

  let requestSeq = 0;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let closeTimer: ReturnType<typeof setTimeout> | null = null;

  /** 挂载时的 `take_pending_selection()` 是否已经返回（用于与事件解竞态） */
  let mountTakeDone = false;
  /** 用户是否手输过（手输过就不再让挂载取词覆盖） */
  let userEdited = false;
  /**
   * 「立刻重新分析」的标记（**普通变量**，不是 $state）：
   * 事件推来的文本会直接跑一次 analyze，这里记下来让防抖那一路跳过同一个值，
   * 避免同一次文本变化发两份请求。
   */
  let immediate: { value: string; ready: boolean } | null = null;

  const summary = $derived(summarizeTokens(tokens, tierIndexOfToken));
  const wordBounds = $derived(meta ? activeBounds('word', meta) : []);
  const tierNames = $derived(tierNamesOf(meta));
  const tierKeys = $derived(tierKeysFrom(meta));
  const hasTable = $derived(meta !== null);

  /**
   * 当前详情区展示哪一条：钉住的优先，其次最后一次悬停的。
   *
   * 还没有悬停过时（两个都是 null）显示**结果里的第一条**，而不是空态提示：
   *   - 小窗底部的区域是固定 112px 高，一段空态提示会把四格数据顶出可视区；
   *   - 一进来就能看到「排名 / 前 % / 占比」长什么样，比看提示更有用。
   * 标点等不参与统计的 token 也照样渲染（值为「—」），所以高度不会跳。
   */
  const shownIndex = $derived(pinnedIndex ?? activeIndex ?? (tokens.length > 0 ? 0 : null));
  const shownToken = $derived(shownIndex === null ? null : (tokens[shownIndex] ?? null));

  // 分组阈值只影响「着色与图例」，不影响查频次是否成功；有设置就用设置
  const settings = $derived(settingsState ?? appSettings.value);

  /** 顶部主题按钮的提示文案：当前档位 + 点一下会切到哪一档 */
  const themeHint = $derived(themeToggleHint(theme.mode));

  /**
   * 一个 token 实际落在哪一组 —— 返回**组号**（0..6），未收录返回 null。
   *
   * 自定义阈值下与后端给的 `tier` 可能不同；算不出生效组号时回落到 `token.tier`。
   */
  function tierIndexOfToken(token: TokenInfo): number | null {
    if (!meta) return token.tier;
    const kind = token.single_cjk || token.table === 'char' ? 'char' : 'word';
    // 阈值取自**主表组**那张表（小窗与主窗口必须是同一套口径，否则同一句话两处颜色不同）
    const index = activeTierIndex(kind, token.rank, meta, primaryTableKey(meta, kind));
    return index ?? token.tier;
  }

  // ---------------------------------------------------------------- 生命周期

  $effect(() => {
    void bootstrap();
  });

  // 文本变化（手输或事件推来）→ 重新分析
  $effect(() => {
    const value = text;
    const ready = hasTable;
    return schedule(value, ready);
  });

  // Esc 关闭小窗。热键流程的标配：看完一句随手关掉，焦点会由后端还给
  // 你刚才划词的那个程序，方便接着选下一句。
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        void hidePopup();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // 订阅后端的 `popup:text` 与 `popup:note` 推送；组件卸载时 unlisten
  $effect(() => {
    let disposed = false;
    const unlistens: Array<() => void> = [];
    const track = (p: Promise<() => void>) => {
      void p.then((off) => {
        if (disposed) off();
        else unlistens.push(off);
      });
    };
    track(onPopupText((next) => applyPopupText(next)));
    track(onPopupNote((next) => (captureNote = next)));
    return () => {
      disposed = true;
      for (const off of unlistens) off();
    };
  });

  async function bootstrap() {
    const settingsRes = await getSettings();
    if (settingsRes.ok) {
      settingsState = settingsRes.data;
      // 界面语言以设置为准（localStorage 只是首屏快速通道）
      adoptLocaleFromSettings(settingsRes.data.locale);
      // 透明度由设置决定，直接作用到根元素上，避免窗口整体透明度过低看不清字
      const opacity = Math.min(1, Math.max(0.3, settingsRes.data.popupOpacity || 1));
      document.documentElement.style.setProperty('--popup-opacity', String(opacity));
      if (settingsRes.data.popupAutoCloseMs > 0) {
        closeTimer = setTimeout(() => void closeWindow(), settingsRes.data.popupAutoCloseMs);
      }
    }

    // 当前打开的那张表：**取**后端的，不再用 settings.dataDir 拼路径 ——
    // dataDir 现在是「数据文件夹」（里面是 dicts\ 与 tables\），它下面没有 meta.json。
    // 也不再需要 open_dataset：后端在启动 / 激活时已经按词典链重建过分词器。
    const current = await activeDataset();
    meta = current.ok && current.data ? current.data : null;

    // 第一次打开小窗时，事件与挂载取词存在竞态：
    //   - 先收到事件（可能是空串）→ applyPopupText 已经把文本放进去了，这里不再覆盖；
    //   - 事件还没到（或先到的空串把框留空）→ 用挂载取到的文本兜底。
    // 只在「输入框还空着 + 用户没手输过」时兜底，避免把事件推来的内容擦掉。
    const pending = await takePendingSelection();
    mountTakeDone = true;
    if (pending.ok && pending.data.trim() && !userEdited && text === '') {
      text = pending.data;
    }
    // 取词失败原因也拉一次（事件可能早于监听注册）
    if (!captureNote) captureNote = await fetchCaptureNote();

    loading = false;
  }

  /**
   * 收到后端推送的待分析文本。
   *
   *   - 与当前输入框内容相同 → 直接返回（不重复触发分析）；
   *   - 空串 → 清空输入框（= 「当前没有选中内容，请手输」）；
   *   - 非空 → 填进输入框并**立刻**分析一次（不等防抖）。
   */
  function applyPopupText(next: string) {
    const value = typeof next === 'string' ? next : '';
    if (value === text) return;
    // 挂载取词还没回来时，空事件先不处理：它可能只是「取词前」的占位，
    // 若是竞态导致事件先到，紧接着挂载那一路就会把真正的文本填进来。
    if (value === '' && !mountTakeDone) return;

    text = value;
    if (value === '') {
      tokens = [];
      activeIndex = null;
      pinnedIndex = null;
      return;
    }

    immediate = { value, ready: hasTable };
    void run(value, hasTable);
  }

  function schedule(value: string, ready: boolean) {
    // 事件推来的文本已经立刻分析过，这里跳过同一个值，避免重复请求
    if (immediate !== null && immediate.value === value && immediate.ready === ready) {
      immediate = null;
      return;
    }
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      void run(value, ready);
    }, 200);
    return () => {
      if (debounceTimer !== null) {
        clearTimeout(debounceTimer);
        debounceTimer = null;
      }
    };
  }

  async function run(value: string, ready: boolean) {
    if (!ready || !value.trim()) {
      tokens = [];
      return;
    }
    const seq = ++requestSeq;
    // dir 传 null = 用后端当前打开的那张表（见 ensure_dataset 的语义）
    const res = await analyzeText(value, [], null);
    if (seq !== requestSeq) return;
    if (!res.ok) {
      error = res.error;
      tokens = [];
      return;
    }
    error = '';
    tokens = res.data;
  }

  // ---------------------------------------------------------------- 交互

  /**
   * token 在结果数组里的下标。
   *
   * 不直接用 `tokens.indexOf(token)`：`$state` 数组是深层代理，交给组件的
   * token 与数组里的元素是同一份代理，但下标比较走「位置 + 文本」更稳
   * （代理身份比较在不同渲染路径下不保证相等）。
   */
  function indexOfToken(token: TokenInfo): number {
    return tokens.findIndex(
      (item) =>
        item === token || (item.byte_start === token.byte_start && item.text === token.text)
    );
  }

  /** 悬停：更新详情区；`token === null`（鼠标移开）时**保留**最后一次的详情 */
  function onHover(token: TokenInfo | null) {
    if (!token) return;
    const index = indexOfToken(token);
    if (index < 0) return;
    activeIndex = index;
  }

  /** 点击钉住 / 取消钉住（钉住后悬停别的词不改变详情区） */
  function onPick(token: TokenInfo) {
    const index = indexOfToken(token);
    if (index < 0) return;
    pinnedIndex = pinnedIndex === index ? null : index;
    activeIndex = index;
  }

  function flash(message: string) {
    notice = message;
    window.setTimeout(() => {
      if (notice === message) notice = '';
    }, 2500);
  }

  async function copyAll() {
    if (!text.trim()) {
      flash(t('popup.copyEmpty'));
      return;
    }
    const res = await copyText(text);
    flash(res.ok ? t('popup.copied') : res.error);
  }

  async function sendBack() {
    if (!text.trim()) {
      flash(t('popup.sendEmpty'));
      return;
    }
    const res = await emitPopupReply(text);
    flash(res.ok ? t('popup.sentToMain') : res.error);
  }

  async function closeWindow() {
    if (closeTimer !== null) clearTimeout(closeTimer);
    if (!isTauri()) return;
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().hide();
    } catch {
      // 隐藏失败时退回到关闭；两者都失败就什么都不做（小窗仍可用）
      try {
        window.close();
      } catch {
        /* 忽略 */
      }
    }
  }

  function clearText() {
    text = '';
    tokens = [];
    activeIndex = null;
    pinnedIndex = null;
  }
</script>

<div
  class="flex h-full w-full flex-col overflow-hidden rounded-xl border border-border bg-card text-card-foreground shadow-2xl"
  style="opacity: var(--popup-opacity, 1)"
>
  <!-- 可拖拽标题栏 -->
  <header
    data-tauri-drag-region
    class="flex h-8 shrink-0 items-center gap-2 border-b border-border bg-surface-muted/70 px-2.5 select-none"
  >
    <span data-tauri-drag-region class="text-[11px] font-medium text-muted-foreground">
      {t('popup.title')}
    </span>
    {#if !hasTable && !loading}
      <span data-tauri-drag-region class="text-[10px] text-amber-600 dark:text-amber-400">
        {t('popup.noTable')}
      </span>
    {/if}
    {#if notice}
      <span class="truncate text-[10px] text-primary">{notice}</span>
    {/if}

    <span class="ml-auto flex items-center gap-0.5">
      <!-- 主题切换：浅色 → 深色 → 跟随系统 循环。
           写 localStorage 会触发 StorageEvent，Tauri 事件总线再兜一路，
           两个窗口的主题因此始终一致（见 $lib/theme-sync.ts）。
           按钮带图标 + 当前档位文字，占位不大，不会把「发回主窗口」挤走。 -->
      <button
        type="button"
        draggable="false"
        class="flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title={t('popup.themeTitle', { hint: themeHint })}
        aria-label={t('popup.themeAria')}
        data-testid="popup-theme-toggle"
        onclick={() => setTheme(nextThemeMode(theme.mode))}
      >
        {#if theme.mode === 'dark'}
          <IconMoon size={12} />
        {:else if theme.mode === 'light'}
          <IconSun size={12} />
        {:else}
          <span class="text-[10px] leading-none">◐</span>
        {/if}
        <span>{themeLabel(theme.mode)}</span>
      </button>
      <button
        type="button"
        draggable="false"
        class="rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title={t('popup.copyAllTitle')}
        onclick={() => void copyAll()}
      >
        {t('popup.copy')}
      </button>
      <button
        type="button"
        draggable="false"
        class="rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title={t('popup.sendTitle')}
        onclick={() => void sendBack()}
      >
        {t('popup.sendBack')}
      </button>
      <button
        type="button"
        draggable="false"
        class="rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
        title={t('popup.hideTitle')}
        aria-label={t('popup.closeAria')}
        onclick={() => void closeWindow()}
      >
        ✕
      </button>
    </span>
  </header>

  <!-- 输入区 -->
  <div class="flex shrink-0 items-center gap-1.5 border-b border-border px-2 py-1.5">
    <textarea
      bind:value={text}
      oninput={() => (userEdited = true)}
      rows="2"
      spellcheck="false"
      placeholder={t('popup.inputPlaceholder')}
      class={cn(
        'scrollbar-thin max-h-20 min-h-9 flex-1 resize-none rounded-md border border-input bg-surface px-2 py-1',
        'text-xs leading-relaxed text-foreground placeholder:text-muted-foreground',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background'
      )}
    ></textarea>
    {#if text}
      <button
        type="button"
        class="rounded px-1.5 py-1 text-[11px] text-muted-foreground hover:bg-accent"
        title={t('popup.clearTitle')}
        onclick={clearText}
      >
        {t('popup.clear')}
      </button>
    {/if}
  </div>

  <!-- 着色 token 卡片（上半，自己滚动）。
       ⚠️ 三个要点，改之前先读：
       1. 里面的两个子块必须 `shrink-0`。flex 子项默认 flex-shrink:1，不锁住的话
          它们会把自己**压扁**来适应高度 —— 那样 card 的 scrollHeight 永远等于
          clientHeight，浏览器认为「没溢出」，滚动条就出不来。
       2. 本栏用 `flex: 1 1 auto`（**basis 是 auto，不是 0%**）+ `min-h-16`。
          basis:auto = 「以内容自然高度为基准，再按 grow/shrink 分配空间」。
          这样窗口变高时，多出来的空间会**同时**分给本栏和下面的详情区，
          详情区才可能长大到「不用滚动、一眼看尽」。
          （用 `flex-1` 时 basis 是 0%，它会把自由空间全吃光，详情区加多少
          grow 都拿不到空间 —— 这个坑我踩过，实测四种写法结果完全一样。）
       3. 收窗口时本栏先让位：内容放不下 → 出现滚动条 → 压到 min-h-16 之后，
          才轮到详情区变矮。 -->
  <div
    data-testid="popup-token-cards"
    style="flex: 1 1 auto;"
    class="scrollbar-thin flex min-h-16 flex-col overflow-y-auto px-2.5 py-2"
  >
    {#if loading}
      <p class="py-6 text-center text-[11px] text-muted-foreground">{t('popup.loading')}</p>
    {:else if !hasTable}
      <div class="rounded-lg border border-dashed border-border px-3 py-4 text-center text-[11px] text-muted-foreground">
        <p>{t('popup.noTableDesc')}</p>
        <p class="mt-1">{t('popup.noTableHint')}</p>
      </div>
    {:else if !text.trim()}
      {#if captureNote}
        <!-- 取词失败的真原因：直接显示，省得用户去翻日志文件。
             break-words 是必需的：原因文案里可能出现长串不可断的字符。 -->
        <div
          class="rounded-lg border border-amber-500/40 bg-amber-500/10 px-2.5 py-2 text-[11px] leading-relaxed break-words text-amber-700 dark:text-amber-300"
          data-testid="capture-note"
        >
          {captureNote}
        </div>
      {:else}
        <p class="py-6 text-center text-[11px] text-muted-foreground">
          {t('popup.emptyHint')}
        </p>
      {/if}
    {:else}
      {#if error}
        <p class="mb-2 rounded-md border border-destructive/30 bg-destructive/5 px-2 py-1 text-[11px] text-destructive">
          {error}
        </p>
      {/if}

      <!-- 有内容也要显示来源说明：可能分析的是「剪贴板里已有的内容」而不是刚选中的文字，
           不说清用户会以为取词成功了。 -->
      {#if captureNote}
        <p
          class="mb-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1 text-[10px] leading-relaxed break-words text-amber-700 dark:text-amber-300"
          data-testid="capture-note"
        >
          {captureNote}
        </p>
      {/if}

      <div class="mb-2 shrink-0">
        <TokenChips
          {tokens}
          compact
          {meta}
          {settings}
          curves={tierCurves}
          activeIndex={shownIndex}
          onHover={(token) => onHover(token)}
          onPick={(token) => onPick(token)}
        />
      </div>

      <div class="mt-auto shrink-0 border-t border-border pt-1.5">
        <TierLegend
          names={tierNames}
          keys={tierKeys}
          bounds={wordBounds}
          class="gap-x-2 gap-y-1"
        />
      </div>
    {/if}
  </div>

  <!-- 词条详情：**能长大到不用滚动**，也能在窗口变矮时让位。
       实测「骨架（词头+四格+徽标行）+ 各表组排名 + 未收录说明」最多需要约 270px。
       `flex: 3 1 auto` 里那个 **3** 是关键：它和中栏（grow 1）按 1:3 分多余空间，
       实测窗高 ≥520 时详情区就能长到「内容全放下、不用滚动、一眼看尽」，
       同时中栏还能保留约 147px（≈5 行 token）。给 1:1 会差 20 多像素、
       给 grow:0 则中栏永远不变高（窗口拉高它也不长），都不合适。
       高度 12rem(192px) 是基准，max-height 19rem(304px) 是上限。
       窗口变矮时它排在中栏之后让位；min-h-32(128px) 是下限，此时骨架仍完整可见。 -->
  <section
    style="flex: 3 1 auto; height: 12rem; max-height: 19rem;"
    class="flex min-h-32 flex-col border-t border-border bg-surface-muted/40"
    data-testid="popup-token-detail"
    aria-label={t('popup.detailAria')}
  >
    <div class="flex shrink-0 items-center gap-2 px-2.5 pt-1.5">
      <span class="text-[10px] font-medium text-muted-foreground">{t('popup.detailTitle')}</span>
      {#if pinnedIndex !== null}
        <button
          type="button"
          class="rounded px-1 text-[10px] text-primary hover:bg-accent"
          onclick={() => (pinnedIndex = null)}
        >
          {t('popup.unpin')}
        </button>
      {/if}
      <span class="ml-auto text-[10px] text-muted-foreground">
        {t('popup.summary', { accepted: summary.accepted, unknown: summary.unknownUnique })}{#if meta}{t('popup.summaryTable', { entries: findTable(meta.tables, 'word')?.entries ?? 0 })}{/if}
      </span>
    </div>
    <div class="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-2.5 py-1.5">
      <TokenDetail
        token={shownToken}
        {meta}
        {settings}
        curves={tierCurves}
        compact
        pinned={pinnedIndex !== null}
        emptyHint={t('popup.detailEmpty')}
      />
    </div>
  </section>
</div>
