<script lang="ts">
  /**
   * Topbar —— 内容区顶部标题栏，右侧为主题切换。
   */
  import ThemeToggle from './ThemeToggle.svelte';

  type Props = {
    title: string;
    description?: string;
  };

  let { title, description }: Props = $props();
</script>

<header
  class="flex h-16 shrink-0 items-center gap-4 border-b border-border bg-surface/80 px-6 backdrop-blur"
>
  <div class="flex min-w-0 flex-1 flex-col justify-center">
    <div class="flex items-center gap-2">
      <h1 class="truncate text-base font-semibold tracking-tight">{title}</h1>
    </div>
    {#if description}
      <p class="truncate text-xs text-muted-foreground">{description}</p>
    {/if}
  </div>

  <div class="flex shrink-0 items-center gap-2">
    <ThemeToggle />
  </div>
</header>
