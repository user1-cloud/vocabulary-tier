<script lang="ts">
  /**
   * Sidebar —— 左侧固定导航栏。
   *
   * 当前选中项通过 $props 传入（由 App.svelte 持有路由状态），
   * 组件本身不拥有状态，方便以后换成真实路由。
   */
  import { cn } from '$lib/utils';
  import { NAV_ITEMS, type RouteId } from '$lib/navigation';
  import LogoMark from '$lib/components/icons/LogoMark.svelte';

  type Props = {
    /** $bindable：支持 <Sidebar bind:active /> */
    active?: RouteId;
    appName?: string;
    appVersion?: string;
    onNavigate?: (id: RouteId) => void;
  };

  let {
    active = $bindable<RouteId>('wordfreq'),
    appName = 'VocTier',
    appVersion = 'v0.1.0',
    onNavigate,
  }: Props = $props();

  function select(id: RouteId) {
    active = id;
    onNavigate?.(id);
  }
</script>

<aside
  class="flex h-full w-sidebar shrink-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground"
>
  <!-- 应用标识 -->
  <div class="flex items-center gap-2.5 px-4 py-4">
    <span class="text-primary">
      <span class="block size-8">
        <LogoMark />
      </span>
    </span>
    <span class="flex min-w-0 flex-col">
      <span class="truncate text-sm font-semibold tracking-tight">{appName}</span>
      <span class="truncate text-[11px] text-muted-foreground">字词频率分析</span>
    </span>
  </div>

  <!-- 导航 -->
  <nav class="flex flex-1 flex-col gap-0.5 px-2 py-1" aria-label="主导航">
    {#each NAV_ITEMS as item (item.id)}
      {@const isActive = item.id === active}
      <button
        type="button"
        aria-current={isActive ? 'page' : undefined}
        onclick={() => select(item.id)}
        class={cn(
          'group flex items-center gap-2.5 rounded-md py-2 pl-3 pr-2.5 text-left text-[13px] transition-colors',
          isActive
            ? 'nav-active-bar bg-sidebar-accent font-medium text-sidebar-accent-foreground'
            : 'text-sidebar-foreground/85 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground'
        )}
      >
        <item.icon
          size={16}
          class={isActive ? 'text-primary' : 'text-muted-foreground group-hover:text-primary'}
        />
        <span class="truncate">{item.label}</span>
      </button>
    {/each}
  </nav>

  <!-- 底部信息区：后续可放后台任务状态 -->
  <div class="border-t border-sidebar-border px-4 py-3">
    <div class="flex items-center justify-between text-[11px] text-muted-foreground">
      <span>{appVersion}</span>
      <span>就绪</span>
    </div>
  </div>
</aside>
