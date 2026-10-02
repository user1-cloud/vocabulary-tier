<script lang="ts">
  /**
   * PageStub —— 页面占位外壳。
   *
   * 每个页面在骨架阶段都只渲染这个组件；接入真实业务时把对应
   * src/routes/*.svelte 里的 <PageStub> 换成真实内容即可，
   * 不需要动布局层。
   */
  import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
  import { Badge } from '$lib/components/ui/badge';

  type Props = {
    /** 页面主标题（与顶栏一致） */
    title: string;
    /** 一句话说明这个页面未来做什么 */
    summary: string;
    /** 列出计划中的功能点 */
    planned?: string[];
    /** 建议接入的文件 / 命令，给后续开发者看 */
    hooks?: string[];
  };

  let { title, summary, planned = [], hooks = [] }: Props = $props();

  // 占位格子数量固定，纯装饰
  const skeletonRows = [0, 1, 2, 3, 4];
</script>

<div class="flex flex-col gap-4">
  <Card>
    <CardHeader>
      <div class="flex items-center gap-2">
        <CardTitle>{title}</CardTitle>
        <Badge variant="outline">待实现</Badge>
      </div>
      <CardDescription>{summary}</CardDescription>
    </CardHeader>
    <CardContent class="flex flex-col gap-4">
      {#if planned.length > 0}
        <ul class="flex flex-col gap-1.5">
          {#each planned as point (point)}
            <li class="flex items-start gap-2 text-xs text-muted-foreground">
              <span class="mt-1.5 size-1.5 shrink-0 rounded-full bg-primary/60"></span>
              <span>{point}</span>
            </li>
          {/each}
        </ul>
      {/if}

      <!-- 占位内容区：真实功能会替换掉这几行 -->
      <div class="flex flex-col gap-2 rounded-lg border border-dashed border-border p-4">
        <div class="h-3 w-1/3 rounded bg-surface-muted"></div>
        {#each skeletonRows as row (row)}
          <div class="h-3 rounded bg-surface-muted" style="width: {95 - row * 11}%"></div>
        {/each}
      </div>
    </CardContent>
  </Card>

  {#if hooks.length > 0}
    <Card>
      <CardHeader>
        <CardTitle>接入指引</CardTitle>
        <CardDescription>后续业务逻辑建议挂载在这些位置</CardDescription>
      </CardHeader>
      <CardContent>
        <ul class="flex flex-col gap-1.5">
          {#each hooks as hook (hook)}
            <li class="selectable font-mono text-[11px] leading-relaxed text-muted-foreground">
              {hook}
            </li>
          {/each}
        </ul>
      </CardContent>
    </Card>
  {/if}
</div>
