<script lang="ts">
  /**
   * SidebarGroupLabel —— 分组标题（渲染为 <div>）。shadcn 标准语义。
   * 本项目分组头是可折叠的（CollapsibleTrigger），所以通常直接用 CollapsibleTrigger
   * 套本组件的样式 class（见 Sidebar.svelte），此组件用于非折叠的分组标题场景。
   */
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLDivElement> & {
    children?: import('svelte').Snippet;
  };

  let { class: className = '', children, ...rest }: Props = $props();
</script>

<div
  data-slot="sidebar-group-label"
  class={cn(
    'flex h-7 shrink-0 items-center rounded-md px-2 text-xs font-medium tracking-wider text-sidebar-foreground/60',
    className
  )}
  {...rest}
>
  {@render children?.()}
</div>
