<script lang="ts">
  /**
   * ThemeToggle —— 手动切换浅色 / 深色 / 跟随系统。
   *
   * 用原生 <details> 做下拉，避免为了一个菜单再引入 bits-ui 的 DropdownMenu
   * 并把样式对齐成本拉高。后续要换成 bits-ui DropdownMenu 也很简单。
   */
  import { cn } from '$lib/utils';
  import { THEME_LABELS, setTheme, theme, type ThemeMode } from '$lib/theme.svelte';
  import IconSun from '$lib/components/icons/IconSun.svelte';
  import IconMoon from '$lib/components/icons/IconMoon.svelte';

  const MODES: ThemeMode[] = ['light', 'dark', 'system'];

  const label = $derived(THEME_LABELS[theme.mode]);
</script>

<details class="group relative">
  <summary
    class={cn(
      'flex h-9 cursor-pointer list-none items-center gap-2 rounded-md border border-input bg-surface px-2.5 text-xs font-medium text-foreground',
      'transition-colors hover:bg-accent hover:text-accent-foreground',
      'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background',
      '[&::-webkit-details-marker]:hidden'
    )}
  >
    {#if theme.mode === 'dark'}
      <IconMoon size={15} />
    {:else if theme.mode === 'light'}
      <IconSun size={15} />
    {:else}
      <span class="text-[11px] leading-none">◐</span>
    {/if}
    <span>{label}</span>
  </summary>

  <div
    class="absolute right-0 top-[calc(100%+0.375rem)] z-50 min-w-36 overflow-hidden rounded-lg border border-border bg-card p-1 shadow-lg"
  >
    {#each MODES as mode (mode)}
      <button
        type="button"
        class={cn(
          'flex w-full items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-xs transition-colors',
          'hover:bg-accent hover:text-accent-foreground',
          mode === theme.mode && 'bg-accent/60 text-accent-foreground font-medium'
        )}
        onclick={() => setTheme(mode)}
      >
        <span>{THEME_LABELS[mode]}</span>
        {#if mode === theme.mode}
          <span class="text-primary">✓</span>
        {/if}
      </button>
    {/each}
  </div>
</details>
