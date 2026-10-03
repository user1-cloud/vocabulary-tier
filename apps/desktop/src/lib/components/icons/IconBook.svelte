<script lang="ts">
  /** 词典管理：一本摊开的词典（「词 词频 词性」那种每行一条的文本） */
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
  <path d="M12 6.5v13" />
  <path d="M12 6.5C10.6 5.2 8.6 4.5 6 4.5H3.5v12H6c2.6 0 4.6.7 6 2" />
  <path d="M12 6.5c1.4-1.3 3.4-2 6-2h2.5v12H18c-2.6 0-4.6.7-6 2" />
  <path d="M6.5 8.5h2.5M6.5 11.5h3" />
  <path d="M15 8.5h2.5M14.5 11.5h3" />
</svg>
