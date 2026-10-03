<script lang="ts">
  /**
   * SidebarMenuSub —— 子菜单容器（渲染为 <ul>）。shadcn 标准语义：
   * 左侧一条竖线 + 缩进，表示这是上一级菜单下的嵌套层。
   * 折叠行为由外层 CollapsibleContent 承担（标准 Sidebar 就是这么做的）。
   */
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLUListElement> & {
    children?: import('svelte').Snippet;
  };

  let { class: className = '', children, ...rest }: Props = $props();
</script>

<ul
  data-slot="sidebar-menu-sub"
  class={cn(
    'mx-3.5 flex min-w-0 translate-x-px flex-col gap-0.5 border-l border-sidebar-border px-2.5 py-0.5',
    className
  )}
  {...rest}
>
  {@render children?.()}
</ul>
