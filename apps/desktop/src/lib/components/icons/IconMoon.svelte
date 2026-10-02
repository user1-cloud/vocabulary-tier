<script lang="ts">
  /** 主题切换：月亮（深色态显示）/ 太阳由 IconSun 负责 */
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
  <path d="M20.5 14.3A8.5 8.5 0 0 1 9.7 3.5a8.5 8.5 0 1 0 10.8 10.8z" />
</svg>
