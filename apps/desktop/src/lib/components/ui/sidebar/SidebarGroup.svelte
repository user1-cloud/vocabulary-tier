<script lang="ts">
  /**
   * SidebarGroup —— 侧栏分组容器（渲染为 <div>）。shadcn 标准语义。
   */
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLDivElement> & {
    children?: import('svelte').Snippet;
  };

  let { class: className = '', children, ...rest }: Props = $props();
</script>

<div data-slot="sidebar-group" class={cn('relative flex w-full min-w-0 flex-col p-2', className)} {...rest}>
  {@render children?.()}
</div>
