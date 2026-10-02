/**
 * shadcn-svelte 风格 UI 组件统一出口。
 *
 * 目前只实现了骨架真正用得到的部分（Button / Card / Input / Badge /
 * Separator / Switch）。Tabs、Slider、Select、Table 等留到有真实交互
 * 需求时再补——它们都基于 bits-ui，加的时候复制同样目录结构即可：
 *
 *   src/lib/components/ui/tabs/{Tabs.svelte,TabsList.svelte,...,index.ts}
 *
 * 注意：没有使用 `pnpm dlx shadcn-svelte init`，因为它是交互式的。
 */
export * from './badge';
export * from './button';
export * from './card';
export * from './input';
export * from './separator';
export * from './switch';
