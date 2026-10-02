<script lang="ts">
  /** 划句分析：文本行 + 分段标记 */
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
  <path d="M4 7h16" />
  <path d="M4 12h9" />
  <path d="M4 17h13" />
  <circle cx="18" cy="12" r="2.25" />
</svg>
