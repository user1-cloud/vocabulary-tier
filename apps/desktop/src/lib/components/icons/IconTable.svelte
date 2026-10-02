<script lang="ts">
  /** 表管理：一叠数据表 */
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
  <rect x="3" y="3" width="18" height="7" rx="1.5" />
  <rect x="3" y="14" width="18" height="7" rx="1.5" />
  <path d="M8 6.5h.01M8 17.5h.01M12 6.5h5M12 17.5h5" />
</svg>
