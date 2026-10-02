<script lang="ts">
  /**
   * App —— 主窗口根组件：左侧固定侧边栏 + 右侧内容区。
   *
   * 路由：骨架阶段用一个 $state 保存当前选中项，不引入路由库。
   *      页面之间要跳转时派发 DOM 事件即可（见下面的 NAVIGATE_EVENT / ANALYZE_WORD_EVENT），
   *      这样页面组件不需要知道路由状态存在哪里。
   */
  import Sidebar from '$lib/components/layout/Sidebar.svelte';
  import Topbar from '$lib/components/layout/Topbar.svelte';
  import {
    ANALYZE_WORD_EVENT,
    DEFAULT_ROUTE,
    findNavItem,
    isRouteId,
    NAVIGATE_EVENT,
    type RouteId,
  } from '$lib/navigation';
  import { initTheme } from '$lib/theme.svelte';
  import { adoptLocaleFromSettings, t } from '$lib/i18n.svelte';
  import { onPopupReply } from '$lib/api/bridge';
  import { loadTierSettings } from '$lib/tiers.svelte';

  import WordFreqPage from './routes/WordFreqPage.svelte';
  import SentencesPage from './routes/SentencesPage.svelte';
  import LeaderboardPage from './routes/LeaderboardPage.svelte';
  import DictsPage from './routes/DictsPage.svelte';
  import TablesPage from './routes/TablesPage.svelte';
  import SettingsPage from './routes/SettingsPage.svelte';

  /** 当前路由（响应式） */
  let route = $state<RouteId>(DEFAULT_ROUTE);

  /**
   * 页面版本号：跳转到同一个路由时用它强制重新挂载目标组件，
   * 保证 sentencesSeed 这类「一次性输入」一定被重新消费。
   */
  let routeEpoch = $state(0);

  /** 顶栏标题 / 描述由路由表派生，避免两处手写不一致 */
  const current = $derived(findNavItem(route));

  function goTo(target: RouteId, epochBump = false) {
    route = target;
    if (epochBump) routeEpoch += 1;
  }

  /**
   * 待注入划句分析页的文本（来自排行榜「加入分析」或悬浮小窗回传）。
   *
   * 为什么不在模板里直接调函数消费一个 pending 变量：Svelte 5 禁止在模板
   * 表达式（以及 $derived）里修改 state，否则会抛 state_unsafe_mutation。
   * 所以这里改成「事件里落快照 + 模板只读快照」。
   */
  let sentencesSeed = $state('');

  function goToSentences(seed: string) {
    sentencesSeed = seed;
    goTo('sentences', true);
  }

  // 初始化主题：监听系统主题变化，并把结果打到 <html class="dark"> 上。
  // $effect 的返回值会被当作清理函数，组件卸载时自动移除 matchMedia 监听。
  $effect(() => initTheme());

  // 全局设置只读一次：分组自定义（tierMethod / tierPct / tier*Bounds / tierCoverage）与
  // 主词频表（primaryScope）是所有页面共享的状态，放在 $lib/tiers.svelte.ts 里。
  // 失败时用 defaultSettings()，界面照 meta 默认阈值渲染，不会崩。
  $effect(() => {
    void loadTierSettings().then((res) => {
      // 界面语言的**权威值**在设置里（localStorage 只是首屏的快速通道，
      // 主窗口与小窗的存储可能被宿主隔开），读到就采用。
      if (res.ok) adoptLocaleFromSettings(res.data.locale);
    });
  });

  // 跨页面跳转 / 「加入分析」/ 悬浮小窗回传文本
  $effect(() => {
    const onNavigate = (event: Event) => {
      const detail = (event as CustomEvent<unknown>).detail;
      if (isRouteId(detail)) goTo(detail, true);
    };
    const onAnalyzeWord = (event: Event) => {
      const detail = (event as CustomEvent<unknown>).detail;
      if (typeof detail !== 'string' || detail.length === 0) return;
      goToSentences(detail);
    };
    const onPopup = (text: string) => {
      if (!text) return;
      goToSentences(text);
    };

    window.addEventListener(NAVIGATE_EVENT, onNavigate);
    window.addEventListener(ANALYZE_WORD_EVENT, onAnalyzeWord);
    // 悬浮小窗通过 Tauri 事件总线回传文本（浏览器里是 no-op）
    const offPopup = onPopupReply(onPopup);

    return () => {
      window.removeEventListener(NAVIGATE_EVENT, onNavigate);
      window.removeEventListener(ANALYZE_WORD_EVENT, onAnalyzeWord);
      offPopup();
    };
  });
</script>

<div class="flex h-full w-full overflow-hidden bg-background text-foreground">
  <Sidebar bind:active={route} />

  <div class="flex min-w-0 flex-1 flex-col">
    <Topbar title={t(current.titleKey)} description={t(current.descriptionKey)} />

    <main class="scrollbar-thin flex-1 overflow-y-auto bg-surface-muted/40 p-6">
      <div class="mx-auto w-full max-w-5xl">
        {#key routeEpoch}
          {#if route === 'wordfreq'}
            <WordFreqPage />
          {:else if route === 'sentences'}
            <SentencesPage initialText={sentencesSeed} />
          {:else if route === 'leaderboard'}
            <LeaderboardPage />
          {:else if route === 'dicts'}
            <DictsPage />
          {:else if route === 'tables'}
            <TablesPage />
          {:else if route === 'settings'}
            <SettingsPage />
          {/if}
        {/key}
      </div>
    </main>
  </div>
</div>
