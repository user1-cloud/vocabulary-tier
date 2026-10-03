<script lang="ts">
  /**
   * SidebarGroupContent —— 分组内容区（渲染为 <div>）。shadcn 标准语义。
   */
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLDivElement> & {
    children?: import('svelte').Snippet;
  };

  let { class: className = '', children, ...rest }: Props = $props();
</script>

<div data-slot="sidebar-group-content" class={cn('w-full text-sm', className)} {...rest}>
  {@render children?.()}
</div>
