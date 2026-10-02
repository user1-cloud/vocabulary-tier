<script lang="ts">
  /**
   * Card —— shadcn-svelte 风格容器。
   *
   *   <Card>
   *     <CardHeader>
   *       <CardTitle>标题</CardTitle>
   *       <CardDescription>说明文字</CardDescription>
   *     </CardHeader>
   *     <CardContent>正文</CardContent>
   *     <CardFooter>操作区</CardFooter>
   *   </Card>
   */
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLDivElement> & {
    class?: string;
    children?: Snippet;
  };

  let { class: className = '', children, ...rest }: Props = $props();
</script>

<div
  class={cn(
    'rounded-xl border border-border bg-card text-card-foreground shadow-sm',
    className
  )}
  {...rest}
>
  {@render children?.()}
</div>
