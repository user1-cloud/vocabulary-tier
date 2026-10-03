<script lang="ts">
  /**
   * CollapsibleContent —— 折叠区内容（渲染为 <div>，展开时带位移动画）。
   * forceMount：关闭时也把内容保留在 DOM（默认 false）；hiddenUntilFound：让
   * 浏览器可被搜索到折叠内容。二者都是 bits-ui 原生的可选 prop，默认不启用。
   */
  import { Collapsible as CollapsiblePrimitive } from 'bits-ui';
  import type { ComponentProps } from 'svelte';

  type ContentProps = ComponentProps<typeof CollapsiblePrimitive.Content>;

  let { forceMount, hiddenUntilFound, children, ...rest }: ContentProps = $props();
</script>

<CollapsiblePrimitive.Content {forceMount} {hiddenUntilFound} {...rest}>
  {@render children?.()}
</CollapsiblePrimitive.Content>
