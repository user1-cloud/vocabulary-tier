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
   *   窗口变矮 → 先压缩「着色 token 卡片」，直到它出现滚动条（内容刚好放得下）；
   *   再继续变矮 → 才开始压缩下面的「词条详情」。
   *   实现靠 flex：卡片 `flex-1` + `min-h-24`，详情 `h-28 min-h-24 max-h-[55%]`。
   *   收缩空间先按比例从卡片里扣，卡片到下限之后才轮到详情。
   *   （实测：480×320 时卡片出现滚动条，480×300 起详情才开始变矮。）
   *
   * 为什么详情是固定区域而不是跟随鼠标的浮层：小窗只有 ~460×340，浮层一靠近
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
    analyzeText,
    copyText,
    datasetStatus,
    emitPopupReply,
    fetchCaptureNote,
    getSettings,
    hidePopup,
    isTauri,
    onPopupNote,
    onPopupText,
    openDataset,
    takePendingSelection,
  } from '$lib/api/bridge';
  import { findTable } from '$lib/format';
  import { cn } from '$lib/utils';
  import { THEME_LABELS, setTheme, theme } from '$lib/theme.svelte';
  import { nextThemeMode, themeToggleHint } from '$lib/theme-sync';
  import IconMoon from '$lib/components/icons/IconMoon.svelte';
  import IconSun from '$lib/components/icons/IconSun.svelte';
  import {
    activeBounds,
    activeTierIndex,
    appSettings,
    tierCurves,
    tierNameAt,
    tierNamesOf,
  } from '$lib/tiers.svelte';
  import { summarizeTokens } from '$lib/segments';
  import type { Meta, Settings, TokenInfo } from '$lib/types';

  let text = $state('');
  let tokens = $state<TokenInfo[]>([]);
  let meta = $state<Meta | null>(null);
  let datasetDir = $state<string | null>(null);
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

  const summary = $derived(summarizeTokens(tokens, tierNameOfToken));
  const wordBounds = $derived(meta ? activeBounds('word', meta) : []);
  const tierNames = $derived(tierNamesOf(meta));
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

  /** 一个 token 实际落在哪一组（自定义阈值下与后端给的 tier 不同） */
  function tierNameOfToken(token: TokenInfo): string | null {
    if (!meta) return token.tier_name;
    const isChar = token.single_cjk || token.table === 'char';
    const index = activeTierIndex(
      isChar ? 'char' : 'word',
      token.rank,
      meta,
      isChar ? 'full/char' : 'full/word'
    );
    return tierNameAt(index, tierNames) ?? token.tier_name;
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
      // 透明度由设置决定，直接作用到根元素上，避免窗口整体透明度过低看不清字
      const opacity = Math.min(1, Math.max(0.3, settingsRes.data.popupOpacity || 1));
      document.documentElement.style.setProperty('--popup-opacity', String(opacity));
      if (settingsRes.data.popupAutoCloseMs > 0) {
        closeTimer = setTimeout(() => void closeWindow(), settingsRes.data.popupAutoCloseMs);
      }
    }

    const dir = settingsRes.ok
      ? (settingsRes.data.dataDir ?? settingsRes.data.corpusDir)
      : null;
    const status = await datasetStatus(dir);
    if (status.ok && status.data.exists && status.data.meta) {
      // 让后端装载产物（按 meta.tokenizer 重建分词器）。失败则退回
      // dataset_status 里带的 meta，不阻塞小窗。
      const opened = await openDataset(status.data.dir);
      meta = opened.ok ? opened.data : status.data.meta;
      datasetDir = status.data.dir;
    } else {
      meta = null;
    }

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
    const res = await analyzeText(value, [], datasetDir);
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
      flash('没有可复制的内容');
      return;
    }
    const res = await copyText(text);
    flash(res.ok ? '已复制' : res.error);
  }

  async function sendBack() {
    if (!text.trim()) {
      flash('没有可回传的内容');
      return;
    }
    const res = await emitPopupReply(text);
    flash(res.ok ? '已发送到主窗口' : res.error);
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
      VocTier 取词
    </span>
    {#if !hasTable && !loading}
      <span data-tauri-drag-region class="text-[10px] text-amber-600 dark:text-amber-400">
        无词频表
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
        title={`主题：${themeHint}`}
        aria-label="切换主题"
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
        <span>{THEME_LABELS[theme.mode]}</span>
      </button>
      <button
        type="button"
        draggable="false"
        class="rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title="复制全文"
        onclick={() => void copyAll()}
      >
        复制
      </button>
      <button
        type="button"
        draggable="false"
        class="rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title="发送到主窗口（划句分析页）"
        onclick={() => void sendBack()}
      >
        发回主窗口
      </button>
      <button
        type="button"
        draggable="false"
        class="rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
        title="隐藏小窗"
        aria-label="关闭小窗"
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
      placeholder="粘贴或输入要查词的中文…"
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
        title="清空"
        onclick={clearText}
      >
        清空
      </button>
    {/if}
  </div>

  <!-- 着色 token 卡片（上半，自己滚动）。
       缩放时第一个让位：flex 先把它的空间扣掉，扣到内容放不下（出现滚动条）为止；
       再继续变矮才轮到下面的词条详情。
       min-h-24(96px) 是它的下限：窗口被拖到极限时也别让这一栏彻底消失。 -->
  <div class="scrollbar-thin flex min-h-24 flex-1 flex-col justify-center overflow-y-auto px-2.5 py-2">
    {#if loading}
      <p class="py-6 text-center text-[11px] text-muted-foreground">正在载入词频表…</p>
    {:else if !hasTable}
      <div class="rounded-lg border border-dashed border-border px-3 py-4 text-center text-[11px] text-muted-foreground">
        <p>还没有可用的词频表，取词结果无法着色。</p>
        <p class="mt-1">请先在主窗口的「生成词频表」页完成一次统计。</p>
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
          输入或粘贴文字后，这里会实时显示每个词的分组着色。
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

      <div class="mt-2 border-t border-border pt-1.5">
        <TierLegend
          names={tierNames}
          bounds={wordBounds}
          class="gap-x-2 gap-y-1"
        />
      </div>
    {/if}
  </div>

  <!-- 词条详情：高度与**内容**无关（选中标点还是长词都一样高），恒定 112px。
       112px 是实测出来的「四格数据 + 徽标行（词典标记）」够用的高度；
       注意**不能**给它 flex-grow：试过之后窗口一高它就按 55% 抢走半屏，
       上面那份 token 列表只剩两行（token 列表才是主体内容）。
       窗口变矮时它是第二个让位的（第一是上面那栏），min-h-24 是它自己的下限。 -->
  <section
    class="flex h-28 max-h-28 min-h-24 shrink basis-auto flex-col border-t border-border bg-surface-muted/40"
    data-testid="popup-token-detail"
    aria-label="词条详情"
  >
    <div class="flex shrink-0 items-center gap-2 px-2.5 pt-1.5">
      <span class="text-[10px] font-medium text-muted-foreground">词条详情</span>
      {#if pinnedIndex !== null}
        <button
          type="button"
          class="rounded px-1 text-[10px] text-primary hover:bg-accent"
          onclick={() => (pinnedIndex = null)}
        >
          取消钉住
        </button>
      {/if}
      <span class="ml-auto text-[10px] text-muted-foreground">
        {summary.accepted} 词 · 未收录 {summary.unknownUnique} 种{#if meta}
          · 词表 {findTable(meta.tables, 'word')?.entries ?? 0} 条{/if}
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
        emptyHint="还没有可显示的词条。"
      />
    </div>
  </section>
</div>
