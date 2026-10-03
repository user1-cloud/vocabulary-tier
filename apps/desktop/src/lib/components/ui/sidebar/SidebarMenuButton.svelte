<script lang="ts">
  /**
   * SidebarMenuButton —— 菜单按钮（渲染为 <button>）。shadcn 标准语义。
   *
   * isActive  当前选中（分组/页面级高亮）
   * isChild   属于子菜单层级的项（缩进样式由外层 SidebarMenuSub 承担）
   * variant   outline 用于整页入口等次一级形态
   *
   * 页面头作为 CollapsibleTrigger 时，也可以直接给 CollapsibleTrigger 套本组件的
   * 样式 class（见 Sidebar.svelte 的用法），避免 button 嵌套。
   */
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLButtonAttributes & {
    isActive?: boolean;
    isChild?: boolean;
    variant?: 'default' | 'outline';
    children?: import('svelte').Snippet;
  };

  let {
    isActive = false,
    isChild = false,
    variant = 'default',
    class: className = '',
    children,
    ...rest
  }: Props = $props();
</script>

<button
  type="button"
  data-slot="sidebar-menu-button"
  data-active={isActive ? 'true' : undefined}
  class={cn(
    'group/menu-button flex h-8 w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors',
    'disabled:pointer-events-none disabled:opacity-50',
    isActive
      ? 'bg-sidebar-accent font-medium text-sidebar-accent-foreground'
      : 'text-sidebar-foreground/85 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground',
    variant === 'outline' &&
      'border bg-background hover:bg-sidebar-accent/80 hover:text-sidebar-accent-foreground',
    className
  )}
  {...rest}
>
  {@render children?.()}
</button>
