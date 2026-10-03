<script lang="ts">
  /**
   * SectionCard —— 「可导航区块」容器。
   *
   * 每个区块 = 一张带锚点的 Card：`data-section` 由 `section.id` 自动挂上（侧栏第三级
   * 区块点击据此滚动/高亮），标题由 `section.labelKey` 渲染。区块清单见各页面顶部导出的
   * `PAGE_SECTIONS`，navigation.ts 直接组合它，因此**新增区块只需在页面里加一项 + 一个
   * <SectionCard>，导航配置与锚点属性都不用再手动改**。
   *
   * 用法：
   *   <SectionCard section={PAGE_SECTIONS.step1} descriptionKey="wordfreq.step1.description"
   *                contentClass="flex flex-col gap-3">
   *     <svelte:fragment slot="titleExtra"><Badge variant="outline">plan_corpus</Badge></svelte:fragment>
   *     <!-- 区块正文 -->
   *   </SectionCard>
   *
   *   副标题有两种：纯文本用 descriptionKey；需要带格式（占位符/加粗）时用
   *   `<svelte:fragment slot="description">`（优先于 descriptionKey）。
   */
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import {
    Card,
    CardContent,
    CardDescription,
    CardHeader,
    CardTitle,
  } from '$lib/components/ui/card';
  import { t } from '$lib/i18n.svelte';
  import type { MessageKey } from '$lib/messages';
  import type { NavSection } from '$lib/navigation';
  import { cn } from '$lib/utils';

  type Props = HTMLAttributes<HTMLDivElement> & {
    /** 区块定义：id → data-section 锚点，labelKey → 区块标题（文案 key） */
    section: NavSection;
    /** 纯文本副标题（文案 key）；带格式的副标题用 `description` 命名 slot 替代 */
    descriptionKey?: MessageKey;
    /** CardContent 额外 class（默认 flex flex-col gap-3） */
    contentClass?: string;
    /** 标题右侧附加元素（Badge/按钮），用 <svelte:fragment slot="titleExtra"> */
    titleExtra?: Snippet<[]>;
    /** 带格式的副标题，用 <svelte:fragment slot="description">，优先于 descriptionKey */
    description?: Snippet<[]>;
    /** 区块正文（放进 CardContent） */
    children?: Snippet<[]>;
    class?: string;
  };

  let {
    section,
    descriptionKey,
    contentClass = 'flex flex-col gap-3',
    titleExtra,
    description,
    children,
    class: className = '',
    ...rest
  }: Props = $props();
</script>

<Card data-section={section.id} class={className} {...rest}>
  <CardHeader>
    <div class="flex flex-wrap items-center gap-2">
      <CardTitle>{t(section.labelKey)}</CardTitle>
      {@render titleExtra?.()}
    </div>
    {#if description}
      <CardDescription>{@render description()}</CardDescription>
    {:else if descriptionKey}
      <CardDescription>{t(descriptionKey)}</CardDescription>
    {/if}
  </CardHeader>
  {#if children}
    <CardContent class={cn(contentClass)}>{@render children()}</CardContent>
  {/if}
</Card>

