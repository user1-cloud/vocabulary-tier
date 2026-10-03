<script lang="ts">
  /**
   * 手写内联 SVG 图标（stroke 风格，对齐 lucide 视觉规范）。
   * chevron-right：用于分组展开指示器，展开时由调用方旋转 90°。
   */
  import type { IconProps } from './icon-types';

  let {
    size = 16,
    class: className = '',
    strokeWidth = 1.75,
  }: IconProps = $props();
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  class={className}
  aria-hidden="true"
>
  <path d="m9 18 6-6-6-6" />
</svg>
