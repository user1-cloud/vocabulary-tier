<script lang="ts">
  /**
   * Button —— shadcn-svelte 风格。
   *
   *   <Button>保存</Button>
   *   <Button variant="outline" size="sm">取消</Button>
   *   <Button variant="ghost" size="icon" aria-label="设置"><IconSettings /></Button>
   *
   * 变体表用纯对象 + cn() 实现；如果你想换成 tailwind-variants 的 tv()，
   * 直接把 VARIANTS / SIZES 搬过去即可。
   */
  import type { Snippet } from 'svelte';
  import { cn } from '$lib/utils';

  type Variant = 'default' | 'secondary' | 'outline' | 'ghost' | 'destructive' | 'link';
  type Size = 'sm' | 'md' | 'lg' | 'icon';

  /** 允许透传的原生属性（按需补充即可，避免用 [key: string]: unknown 丢失类型） */
  type RestProps = {
    type?: 'button' | 'submit' | 'reset';
    disabled?: boolean;
    name?: string;
    value?: string;
    title?: string;
    id?: string;
    form?: string;
    autofocus?: boolean;
    'aria-label'?: string;
    'aria-pressed'?: boolean | 'true' | 'false';
    'aria-expanded'?: boolean | 'true' | 'false';
    'aria-haspopup'?: boolean | 'true' | 'false' | 'menu' | 'dialog' | 'listbox' | 'tree' | 'grid';
    onclick?: (event: MouseEvent) => void;
    onkeydown?: (event: KeyboardEvent) => void;
    onfocus?: (event: FocusEvent) => void;
    onblur?: (event: FocusEvent) => void;
  };

  type Props = RestProps & {
    variant?: Variant;
    size?: Size;
    /** $bindable：支持 <Button bind:ref={el}> 取到真实 DOM 节点 */
    ref?: HTMLButtonElement | null;
    class?: string;
    children?: Snippet;
  };

  let {
    variant = 'default',
    size = 'md',
    ref = $bindable(null),
    class: className = '',
    children,
    type = 'button',
    ...rest
  }: Props = $props();

  const BASE =
    'inline-flex shrink-0 select-none items-center justify-center gap-1.5 whitespace-nowrap rounded-md font-medium transition-colors ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background ' +
    'disabled:pointer-events-none disabled:opacity-50 [&_svg]:shrink-0';

  const VARIANTS: Record<Variant, string> = {
    default: 'bg-primary text-primary-foreground hover:bg-primary/90',
    secondary: 'bg-secondary text-secondary-foreground hover:bg-secondary/80',
    outline: 'border border-input bg-surface hover:bg-accent hover:text-accent-foreground',
    ghost: 'hover:bg-accent hover:text-accent-foreground',
    destructive: 'bg-destructive text-destructive-foreground hover:bg-destructive/90',
    link: 'text-primary underline-offset-4 hover:underline',
  };

  const SIZES: Record<Size, string> = {
    sm: 'h-8 px-3 text-xs [&_svg]:size-4',
    md: 'h-9 px-4 text-sm [&_svg]:size-4',
    lg: 'h-10 px-6 text-sm [&_svg]:size-5',
    icon: 'size-9 [&_svg]:size-4',
  };

  const classes = $derived(cn(BASE, VARIANTS[variant], SIZES[size], className));
</script>

<button bind:this={ref} {type} class={classes} {...rest}>
  {@render children?.()}
</button>
