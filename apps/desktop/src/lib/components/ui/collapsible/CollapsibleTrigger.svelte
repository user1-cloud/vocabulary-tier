<script lang="ts">
  /**
   * CollapsibleTrigger —— 展开/收起的触发按钮（渲染为 <button>）。
   */
  import { Collapsible as CollapsiblePrimitive } from 'bits-ui';
  import type { ComponentProps } from 'svelte';

  type TriggerProps = ComponentProps<typeof CollapsiblePrimitive.Trigger>;

  let { disabled, children, ...rest }: TriggerProps = $props();
</script>

<CollapsiblePrimitive.Trigger {disabled} {...rest}>
  {@render children?.()}
</CollapsiblePrimitive.Trigger>
