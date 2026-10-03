<script lang="ts">
  /**
   * SidebarMenuSubButton —— 子菜单按钮（区块小点，渲染为 <button>）。
   * isActive 表示当前定位到的区块，用左侧竖条 + 高亮标识（nav-active-bar）。
   */
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLButtonAttributes & {
    isActive?: boolean;
    children?: import('svelte').Snippet;
  };

  let { isActive = false, class: className = '', children, ...rest }: Props = $props();
</script>

<button
  type="button"
  data-slot="sidebar-menu-sub-button"
  data-active={isActive ? 'true' : undefined}
  class={cn(
    'group/menu-sub-button flex h-7 w-full min-w-0 items-center gap-2 overflow-hidden rounded-md px-2 py-0.5 text-left text-xs transition-colors',
    isActive
      ? 'nav-active-bar bg-sidebar-accent font-medium text-sidebar-accent-foreground'
      : 'text-sidebar-foreground/75 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground',
    className
  )}
  {...rest}
>
  {@render children?.()}
</button>
