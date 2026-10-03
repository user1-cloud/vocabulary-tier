<script lang="ts">
  /**
   * Collapsible —— 可折叠容器（分组导航用）。
   *
   * 薄封装 bits-ui 的 Collapsible 原语：Root 管理 open 状态与展开动画。
   * props 类型直接复用 bits-ui 的 RootProps（通过 ComponentProps 取到），
   * 避免手写 HTMLAttributes 与 bits-ui 内部类型（如 id: string|null）冲突。
   * 与目录里其它 shadcn-svelte 风格组件一样是手写的，不跑 CLI（见 ui/index.ts 注释）。
   */
  import { Collapsible as CollapsiblePrimitive } from 'bits-ui';
  import type { ComponentProps } from 'svelte';

  type RootProps = ComponentProps<typeof CollapsiblePrimitive.Root>;

  let {
    open = $bindable(false),
    onOpenChange,
    onOpenChangeComplete,
    disabled,
    children,
    ...rest
  }: RootProps = $props();
</script>

<CollapsiblePrimitive.Root
  bind:open={open}
  {onOpenChange}
  {onOpenChangeComplete}
  {disabled}
  {...rest}
>
  {@render children?.()}
</CollapsiblePrimitive.Root>
