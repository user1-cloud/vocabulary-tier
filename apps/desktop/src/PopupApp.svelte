<script lang="ts">
  /**
   * PopupApp —— 悬浮小窗（窗口 label === 'popup'）。
   *
   * 无边框窗口，结构：
   *   顶部：可拖拽条（data-tauri-drag-region）+ 操作按钮
   *   中部：输入框（启动时用 take_pending_selection 填入取到的文本）
   *   下部：着色卡片（与划句分析页同一套渲染，紧凑模式）
   *
   * 「发回主窗口」：冻结接口里没有对应命令，`open_popup` 的语义是
   * 「打开小窗并填入文本」，用它等于自己给自己发。因此这里走 Tauri 自带的
   * 事件总线广播 `voctier:popup-text`（见 bridge.ts 的 emitPopupText），
   * 主窗口在 App.svelte 里 listen 同一事件即可，Rust 侧无需新增命令。
   */
  import TokenChips from '$lib/components/analysis/TokenChips.svelte';
  import TokenTip from '$lib/components/analysis/TokenTip.svelte';
  import TierLegend from '$lib/components/analysis/TierLegend.svelte';
  import {
    analyzeText,
    copyText,
    datasetStatus,
    emitPopupText,
    getSettings,
    isTauri,
    openDataset,
    takePendingSelection,
  } from '$lib/api/bridge';
  import { findTable } from '$lib/format';
  import { cn } from '$lib/utils';
  import { activeBounds, activeTierIndex, appSettings, tierNameAt, tierNamesOf } from '$lib/tiers.svelte';
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

  let hovered = $state<TokenInfo | null>(null);
  let tipPos = $state({ x: 0, y: 0 });
  let wrapper = $state<HTMLDivElement | null>(null);

  let requestSeq = 0;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let closeTimer: ReturnType<typeof setTimeout> | null = null;

  const summary = $derived(summarizeTokens(tokens, tierNameOfToken));
  const wordBounds = $derived(meta ? activeBounds('word', meta) : []);
  const charBounds = $derived(meta ? activeBounds('char', meta) : []);
  const tierNames = $derived(tierNamesOf(meta));
  const tipIsChar = $derived(hovered?.single_cjk === true || hovered?.table === 'char');
  const tipBounds = $derived(tipIsChar ? charBounds : wordBounds);
  const hasTable = $derived(meta !== null);

  // 分组阈值只影响「着色与图例」，不影响查频次是否成功；有设置就用设置
  const settings = $derived(settingsState ?? appSettings.value);

  /** 一个 token 实际落在哪一组（自定义阈值下与后端给的 tier 不同） */
  function tierNameOfToken(token: TokenInfo): string | null {
    if (!meta) return token.tier_name;
    const isChar = token.single_cjk || token.table === 'char';
    const index = activeTierIndex(isChar ? 'char' : 'word', token.rank, meta, isChar ? 'full/char' : 'full/word');
    return tierNameAt(index, tierNames) ?? token.tier_name;
  }

  // ---------------------------------------------------------------- 生命周期

  $effect(() => {
    void bootstrap();
  });

  // 文本变化（含手输）→ 重新分析
  $effect(() => {
    const value = text;
    const ready = hasTable;
    return schedule(value, ready);
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

    const pending = await takePendingSelection();
    if (pending.ok && pending.data.trim()) text = pending.data;

    loading = false;
  }

  function schedule(value: string, ready: boolean) {
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

  function onHover(token: TokenInfo | null, event: MouseEvent | null) {
    if (!token || !event || !wrapper) {
      hovered = null;
      return;
    }
    const rect = wrapper.getBoundingClientRect();
    const x = Math.min(Math.max(4, event.clientX - rect.left), Math.max(4, rect.width - 240));
    const y = event.clientY - rect.top;
    tipPos = { x, y: Math.max(4, y) };
    hovered = token;
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
    const res = await emitPopupText(text);
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

  async function clearText() {
    text = '';
    tokens = [];
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

    <span class="ml-auto flex items-center gap-1">
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

  <!-- 着色卡片 -->
  <div class="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-2.5 py-2">
    {#if loading}
      <p class="py-6 text-center text-[11px] text-muted-foreground">正在载入词频表…</p>
    {:else if !hasTable}
      <div class="rounded-lg border border-dashed border-border px-3 py-4 text-center text-[11px] text-muted-foreground">
        <p>还没有可用的词频表，取词结果无法着色。</p>
        <p class="mt-1">请先在主窗口的「生成词频表」页完成一次统计。</p>
      </div>
    {:else if !text.trim()}
      <p class="py-6 text-center text-[11px] text-muted-foreground">
        输入或粘贴文字后，这里会实时显示每个词的分组着色。
      </p>
    {:else}
      {#if error}
        <p class="mb-2 rounded-md border border-destructive/30 bg-destructive/5 px-2 py-1 text-[11px] text-destructive">
          {error}
        </p>
      {/if}

      <div
        bind:this={wrapper}
        class="relative"
        style="opacity: {settings ? Math.min(1, Math.max(0.3, settings.popupOpacity || 1)) : 1}"
      >
        <TokenChips {tokens} compact onHover={onHover} {meta} {settings} />

        {#if hovered}
          <div
            class="pointer-events-none absolute z-40 w-60"
            style="left:{tipPos.x}px;top:{tipPos.y}px;transform:translateY(12px)"
          >
            <TokenTip
              token={hovered}
              name={tierNameOfToken(hovered)}
              bounds={tipBounds}
              names={tierNames}
              class="text-[11px]"
            />
          </div>
        {/if}
      </div>

      <div class="mt-3 border-t border-border pt-2">
        <TierLegend
          names={tierNames}
          bounds={wordBounds}
          class="gap-x-2 gap-y-1"
        />
      </div>

      <p class="mt-2 text-[10px] text-muted-foreground">
        {summary.accepted} 个计入统计的 token · 未收录 {summary.unknownUnique} 种
        {#if meta}
          · 词表 {findTable(meta.tables, 'word')?.entries ?? 0} 条
        {/if}
      </p>
    {/if}
  </div>
</div>
