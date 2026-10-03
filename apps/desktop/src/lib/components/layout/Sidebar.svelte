<script lang="ts">
  /**
   * Sidebar —— 左侧固定导航栏（标准 shadcn Sidebar 组件族 + 三层树形导航）。
   *
   * 层级：分组（Collapsible）> 页面（Collapsible）> 区块（SidebarMenuSub）。
   *   - 分组头 / 页面头是可折叠触发头（CollapsibleTrigger + 标准菜单按钮样式）；
   *   - 区块小点用 SidebarMenuSubButton，点击 = 跳转到页内锚点（App.svelte 负责滚动）。
   *
   * 展开状态：
   *   - 分组与页面的手动开合分别记忆在 localStorage，刷新后保持；
   *   - 当前激活页面所在的【分组 + 页面】自动展开，其余保持收起。
   *
   * 选中态两级：active（当前页面，bind 到 App 的 route）+ activeSection（当前区块）。
   */
  import { cn } from '$lib/utils';
  import { NAV_GROUPS, type NavPage, type NavSection, type RouteId } from '$lib/navigation';
  import { t } from '$lib/i18n.svelte';
  import LogoMark from '$lib/components/icons/LogoMark.svelte';
  import IconChevron from '$lib/components/icons/IconChevron.svelte';
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from '$lib/components/ui/collapsible';
  import {
    SidebarGroup,
    SidebarGroupContent,
    SidebarMenu,
    SidebarMenuItem,
    SidebarMenuSub,
    SidebarMenuSubButton,
    SidebarMenuSubItem,
  } from '$lib/components/ui/sidebar';

  type Props = {
    /** $bindable：当前页面（对应 App 的 route） */
    active?: RouteId;
    /** $bindable：当前区块（锚点 key） */
    activeSection?: string;
    appName?: string;
    appVersion?: string;
    /** 点击区块跳转：携带 (页面, 区块锚点)；不传区块 = 仅页面级跳转 */
    onNavigate?: (page: RouteId, section?: string) => void;
  };

  let {
    active = $bindable<RouteId>('wordfreq'),
    activeSection = $bindable<string | undefined>(undefined),
    appName = 'VocTier',
    appVersion = 'v0.1.0',
    onNavigate,
  }: Props = $props();

  function selectSection(page: NavPage, section: NavSection) {
    active = page.id;
    activeSection = section.id;
    onNavigate?.(page.id, section.id);
  }

  // ---------------------------------------------------------------- 展开状态
  const GROUP_KEY = 'voctier:sidebar:groups';
  const PAGE_KEY = 'voctier:sidebar:pages';

  function loadOpen<T extends Record<string, boolean>>(key: string): T {
    try {
      const raw = localStorage.getItem(key);
      if (raw) return JSON.parse(raw) as T;
    } catch {
      /* 损坏或不可用时回退到默认（全收起） */
    }
    return {} as T;
  }

  let openGroups = $state<Record<string, boolean>>(loadOpen(GROUP_KEY));
  let openPages = $state<Record<string, boolean>>(loadOpen(PAGE_KEY));

  $effect(() => {
    try {
      localStorage.setItem(GROUP_KEY, JSON.stringify(openGroups));
    } catch {
      /* 存储不可用时静默 */
    }
  });
  $effect(() => {
    try {
      localStorage.setItem(PAGE_KEY, JSON.stringify(openPages));
    } catch {
      /* 存储不可用时静默 */
    }
  });

  // 激活页面所在的【分组 + 页面】自动展开（导航到哪就把那两级撑开）
  $effect(() => {
    const group = NAV_GROUPS.find((g) => g.pages.some((p) => p.id === active));
    if (group) openGroups[group.id] = true;
    if (active) openPages[active] = true;
  });

  const chevronClass = (isOpen: boolean) =>
    cn('shrink-0 transition-transform duration-200', isOpen && 'rotate-90');
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
      <span class="truncate text-[11px] text-muted-foreground">{t('layout.sidebarTagline')}</span>
    </span>
  </div>

  <!-- 导航：分组 > 页面 > 区块 三层 -->
  <nav class="flex flex-1 flex-col gap-1 overflow-y-auto px-2 py-1" aria-label={t('layout.mainNav')}>
    {#each NAV_GROUPS as group (group.id)}
      {@const groupOpen = openGroups[group.id] ?? false}
      <SidebarGroup>
        <Collapsible
          open={groupOpen}
          onOpenChange={(value) => {
            openGroups[group.id] = value;
          }}
        >
          <!-- 分组头 -->
          <CollapsibleTrigger
            class={cn(
              'group flex h-7 w-full items-center justify-between rounded-md px-2 text-left text-xs font-medium tracking-wider text-sidebar-foreground/60 transition-colors',
              'hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground'
            )}
          >
            <span class="truncate">{t(group.labelKey)}</span>
            <IconChevron size={14} class={chevronClass(groupOpen)} />
          </CollapsibleTrigger>

          <CollapsibleContent>
            <SidebarGroupContent>
              <SidebarMenu>
                {#each group.pages as page (page.id)}
                  {@const pageActive = page.id === active}
                  {@const pageOpen = openPages[page.id] ?? false}
                  <SidebarMenuItem>
                    <Collapsible
                      open={pageOpen}
                      onOpenChange={(value) => {
                        openPages[page.id] = value;
                      }}
                    >
                      <!-- 页面头：点击展开/收起该页的区块 -->
                      <CollapsibleTrigger
                        class={cn(
                          'group flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors',
                          pageActive
                            ? 'bg-sidebar-accent font-medium text-sidebar-accent-foreground'
                            : 'text-sidebar-foreground/85 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground'
                        )}
                      >
                        <page.icon
                          size={16}
                          class={cn(
                            'shrink-0',
                            pageActive ? 'text-primary' : 'text-muted-foreground'
                          )}
                        />
                        <span class="min-w-0 flex-1 truncate">{t(page.labelKey)}</span>
                        <IconChevron size={14} class={chevronClass(pageOpen)} />
                      </CollapsibleTrigger>

                      <CollapsibleContent>
                        <SidebarMenuSub>
                          {#each page.sections as section (section.id)}
                            {@const sectionActive =
                              page.id === active && activeSection === section.id}
                            <SidebarMenuSubItem>
                              <SidebarMenuSubButton
                                isActive={sectionActive}
                                onclick={() => selectSection(page, section)}
                              >
                                {t(section.labelKey)}
                              </SidebarMenuSubButton>
                            </SidebarMenuSubItem>
                          {/each}
                        </SidebarMenuSub>
                      </CollapsibleContent>
                    </Collapsible>
                  </SidebarMenuItem>
                {/each}
              </SidebarMenu>
            </SidebarGroupContent>
          </CollapsibleContent>
        </Collapsible>
      </SidebarGroup>
    {/each}
  </nav>

  <!-- 底部信息区：后续可放后台任务状态 -->
  <div class="border-t border-sidebar-border px-4 py-3">
    <div class="flex items-center justify-between text-[11px] text-muted-foreground">
      <span>{appVersion}</span>
      <span>{t('layout.sidebarReady')}</span>
    </div>
  </div>
</aside>
