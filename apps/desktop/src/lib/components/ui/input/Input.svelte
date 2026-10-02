<script lang="ts">
  /**
   * Input —— shadcn-svelte 风格输入框。
   *
   * 支持两种双向绑定（Svelte 5 通过 $bindable 暴露，类型用重载区分）：
   *
   *   <Input bind:value={text} />                     // value: string
   *   <Input type="number" bind:value={count} />      // value: number
   *
   * 为什么给 number 留一个重载：`bind:value` 的类型必须与 DOM 属性一致，
   * 否则 svelte-check 会报 "Type 'number' is not assignable to type 'string'"。
   */
  import type { HTMLInputAttributes } from 'svelte/elements';
  import { cn } from '$lib/utils';

  type BaseProps = Omit<HTMLInputAttributes, 'value' | 'type'> & { class?: string };

  type Props = BaseProps & {
    type?: 'text' | 'search' | 'password' | 'url' | 'tel' | 'email';
    /** $bindable：支持 <Input bind:value={text} /> */
    value?: string;
  };

  type NumberProps = BaseProps & {
    type: 'number';
    /** $bindable：支持 <Input type="number" bind:value={count} /> */
    value?: number | null;
  };

  let {
    value = $bindable(''),
    class: className = '',
    type = 'text',
    ...rest
  }: Props | NumberProps = $props();

  const classes = $derived(
    cn(
      'flex h-9 w-full rounded-md border border-input bg-surface px-3 py-1 text-sm shadow-sm transition-colors',
      'placeholder:text-muted-foreground',
      'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background',
      'disabled:cursor-not-allowed disabled:opacity-50',
      className
    )
  );
</script>

{#if type === 'number'}
  <input type="number" bind:value={value as number | null} class={classes} {...rest} />
{:else}
  <input {type} bind:value={value as string} class={classes} {...rest} />
{/if}
