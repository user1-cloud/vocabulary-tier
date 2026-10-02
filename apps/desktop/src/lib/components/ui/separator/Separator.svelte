<script lang="ts">
  /**
   * Separator —— 分割线（shadcn-svelte 的 Separator 也是无依赖的简单实现）
   */
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLDivElement> & {
    orientation?: 'horizontal' | 'vertical';
    class?: string;
  };

  let { orientation = 'horizontal', class: className = '', ...rest }: Props = $props();
</script>

<div
  role="separator"
  aria-orientation={orientation}
  class={cn(
    'shrink-0 bg-border',
    orientation === 'horizontal' ? 'h-px w-full' : 'h-full w-px',
    className
  )}
  {...rest}
></div>
