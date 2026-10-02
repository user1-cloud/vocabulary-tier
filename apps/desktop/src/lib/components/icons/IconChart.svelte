<script lang="ts">
  /**
   * 手写内联 SVG 图标（stroke 风格，对齐 lucide 视觉规范）。
   *
   * 不引入图标包的原因：骨架阶段只需要个位数图标，内联可以彻底避开
   * 图标包与 Svelte 5 的版本兼容风险。后续图标变多时直接换成
   * `@lucide/svelte` 即可，用法一致。
   */
  import type { IconProps } from './icon-types';

  let {
    size = 18,
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
  <path d="M3 3v16a2 2 0 0 0 2 2h16" />
  <rect x="7" y="12" width="3" height="6" rx="1" />
  <rect x="12" y="8" width="3" height="10" rx="1" />
  <rect x="17" y="4" width="3" height="14" rx="1" />
</svg>
