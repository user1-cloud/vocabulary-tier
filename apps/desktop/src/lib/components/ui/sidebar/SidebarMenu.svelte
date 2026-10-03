<script lang="ts">
  /**
   * SidebarMenu —— 侧栏菜单容器（渲染为 <ul>）。shadcn 标准语义：一组菜单项。
   */
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLUListElement> & {
    children?: import('svelte').Snippet;
  };

  let { class: className = '', children, ...rest }: Props = $props();
</script>

<ul data-slot="sidebar-menu" class={cn('flex w-full min-w-0 flex-col gap-1', className)} {...rest}>
  {@render children?.()}
</ul>
