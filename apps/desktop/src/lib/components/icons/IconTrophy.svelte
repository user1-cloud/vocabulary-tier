<script lang="ts">
  /** 排行榜：奖杯 */
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
  <path d="M7 4h10v5a5 5 0 0 1-10 0z" />
  <path d="M7 5H5a2 2 0 0 0 2 4" />
  <path d="M17 5h2a2 2 0 0 1-2 4" />
  <path d="M12 14v3" />
  <path d="M8.5 20h7" />
  <path d="M10 17h4l.5 3h-5z" />
</svg>
