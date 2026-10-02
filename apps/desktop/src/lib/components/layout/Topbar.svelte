<script lang="ts">
  /**
   * Topbar —— 内容区顶部标题栏。
   * 右侧预留了全局搜索框（占位，不接逻辑）和主题切换。
   */
  import ThemeToggle from './ThemeToggle.svelte';
  import Input from '$lib/components/ui/input/Input.svelte';
  import { Badge } from '$lib/components/ui/badge';
  import { t } from '$lib/i18n.svelte';

  type Props = {
    title: string;
    description?: string;
  };

  let { title, description }: Props = $props();

  // 占位状态：等接入业务后改成「全局词条搜索」
  let query = $state('');
</script>

<header
  class="flex h-16 shrink-0 items-center gap-4 border-b border-border bg-surface/80 px-6 backdrop-blur"
>
  <div class="flex min-w-0 flex-1 flex-col justify-center">
    <div class="flex items-center gap-2">
      <h1 class="truncate text-base font-semibold tracking-tight">{title}</h1>
      <Badge variant="secondary">{t('layout.badge')}</Badge>
    </div>
    {#if description}
      <p class="truncate text-xs text-muted-foreground">{description}</p>
    {/if}
  </div>

  <div class="flex shrink-0 items-center gap-2">
    <Input
      bind:value={query}
      placeholder={t('layout.searchPlaceholder')}
      class="h-9 w-52 text-xs"
      aria-label={t('layout.searchLabel')}
    />
    <ThemeToggle />
  </div>
</header>
