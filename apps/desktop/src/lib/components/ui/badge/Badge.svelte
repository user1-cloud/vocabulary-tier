<script lang="ts">
  /** Badge —— 标签 / 状态徽章 */
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Variant = 'default' | 'secondary' | 'outline' | 'success';

  type Props = HTMLAttributes<HTMLSpanElement> & {
    variant?: Variant;
    class?: string;
    children?: Snippet;
  };

  let { variant = 'default', class: className = '', children, ...rest }: Props = $props();

  const VARIANTS: Record<Variant, string> = {
    default: 'border-transparent bg-primary/12 text-primary',
    secondary: 'border-transparent bg-secondary text-secondary-foreground',
    outline: 'border-border text-muted-foreground',
    success: 'border-transparent bg-emerald-500/12 text-emerald-600 dark:text-emerald-400',
  };
</script>

<span
  class={cn(
    'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium leading-tight',
    VARIANTS[variant],
    className
  )}
  {...rest}
>
  {@render children?.()}
</span>
