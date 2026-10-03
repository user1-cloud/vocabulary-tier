<script lang="ts">
  /**
   * SidebarMenuSubItem —— 子菜单单项（渲染为 <li>）。
   */
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLLIElement> & {
    children?: import('svelte').Snippet;
  };

  let { class: className = '', children, ...rest }: Props = $props();
</script>

<li data-slot="sidebar-menu-sub-item" class={cn('relative min-w-0', className)} {...rest}>
  {@render children?.()}
</li>
