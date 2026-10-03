<script lang="ts">
  /**
   * SidebarMenuItem —— 单个菜单项（渲染为 <li>）。shadcn 标准语义。
   */
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLLIElement> & {
    children?: import('svelte').Snippet;
  };

  let { class: className = '', children, ...rest }: Props = $props();
</script>

<li data-slot="sidebar-menu-item" class={cn('relative min-w-0', className)} {...rest}>
  {@render children?.()}
</li>
