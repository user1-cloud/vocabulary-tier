<script module lang="ts">
  import type { NavSection } from '$lib/navigation';

  /** 页面内可导航区块：navigation.ts 直接组合它，新增区块只需在这里加一项 + 一个 <SectionCard> */
  export const PAGE_SECTIONS = {
    datadir: { id: 'datadir', labelKey: 'dicts.dataDirTitle' },
    list: { id: 'list', labelKey: 'dicts.listTitle' },
  } as const satisfies Record<string, NavSection>;
</script>

<script lang="ts">
  /**
   * 词典管理 —— 数据文件夹 `dicts\` 里的 `.dict` 清单。
   *
   * 词典外置之后这一页是**入口页**：安装包只是"帮你放了两个进去"，预置项与用户
   * 自己导入的完全是同一种东西（**没有权限等级**，预置项同样可删），所以这里
   * 只有一个纯展示用的来源徽标。
   *
   * 两件必须说清楚的事：
   *   1. **隐患**。词典是普通文本文件，用户会拿记事本改。`report` 里的 `freq_zero`
   *     （显式写了 0 → 这些词永远切不出来）与 `freq_omitted`（没写词频 → 按
   *     建议值折算）在界面上必须显眼，否则用户只会看到"某些词怎么查不到"。
   *   2. **删除的连带影响**。删掉被某张表引用的词典，那张表就变成「词典缺失」，
   *      频次不再可信。所以确认时要把"哪些表用到它"列出来 —— 用
   *      `table_list()` 里每张表 `meta.tokenizer.dicts` 的 `name` 反查。
   *
   * 所有 Tauri 调用都走 $lib/api/bridge（页面不直接 import @tauri-apps/api）。
   */
  import { Badge } from '$lib/components/ui/badge';
  import { Button } from '$lib/components/ui/button';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { SectionCard } from '$lib/components/ui/section-card';
  import { Separator } from '$lib/components/ui/separator';
  import {
    dictDelete,
    dictImport,
    dictList,
    isTauri,
    libraryInfo,
    libraryReady,
    openExternal,
    pickDirectory,
    pickFile,
    setDataDir,
    tableList,
  } from '$lib/api/bridge';
  import { formatInt } from '$lib/format';
  import { t } from '$lib/i18n.svelte';
  import type { MessageKey } from '$lib/messages';
  import { resolvedDicts, type DictItem, type LibraryInfo, type Origin, type TableItem } from '$lib/types';
  import { cn } from '$lib/utils';

  /** 来源徽标的文案 key（`Origin` 的取值与 Rust 侧一一对应） */
  const ORIGIN_LABELS: Record<Origin, MessageKey> = {
    seeded: 'dicts.origin.seeded',
    imported: 'dicts.origin.imported',
    scanned: 'dicts.origin.scanned',
    unknown: 'dicts.origin.unknown',
  };

  let info = $state<LibraryInfo | null>(null);
  let dicts = $state<DictItem[]>([]);
  let tables = $state<TableItem[]>([]);
  let ready = $state(false);

  let loading = $state(true);
  let busy = $state(false);
  let loadError = $state('');
  let notice = $state('');
  let noticeTone = $state<'info' | 'error' | 'success'>('info');

  /** 等待确认删除的词典文件名（null = 没有待确认的删除） */
  let pendingDelete = $state<string | null>(null);

  const hasUsable = $derived(dicts.some((item) => item.error === null));

  /**
   * `dictRef.sha256（小写）→ 用到它的表名`。
   *
   * 按**指纹**反查而不是按名字：后端就是按「指纹 → 名字」匹配的，重名的两份词典
   * 只有指纹能区分。名字只作为兜底（v1 老产物没有指纹）。
   */
  const dictUsage = $derived.by(() => {
    const byHash = new Map<string, string[]>();
    const byName = new Map<string, string[]>();
    for (const table of tables) {
      for (const ref of resolvedDicts(table.meta?.tokenizer)) {
        if (ref.sha256) {
          byHash.set(ref.sha256.toLowerCase(), [...(byHash.get(ref.sha256.toLowerCase()) ?? []), table.name]);
        }
        if (ref.name) {
          byName.set(ref.name, [...(byName.get(ref.name) ?? []), table.name]);
        }
      }
    }
    return { byHash, byName };
  });

  /** 哪些表用到了这份词典（去重，保序） */
  function usedBy(item: DictItem): string[] {
    const names = new Set<string>();
    if (item.file_name.endsWith('.dict')) names.add(item.file_name.slice(0, -'.dict'.length));
    names.add(item.dict.id);
    names.add(item.dict.name);

    const out = new Set<string>();
    if (item.dict.sha256) {
      for (const name of dictUsage.byHash.get(item.dict.sha256.toLowerCase()) ?? []) out.add(name);
    }
    for (const name of names) {
      if (!name) continue;
      for (const table of dictUsage.byName.get(name) ?? []) out.add(table);
    }
    return [...out];
  }

  /** 「有效词条数」：`report.entries` 比 `dict.entries` 更权威（坏文件时后者是 0） */
  function entriesOf(item: DictItem): number {
    return item.report.entries || item.dict.entries;
  }

  function showNotice(message: string, tone: 'info' | 'error' | 'success' = 'info') {
    notice = message;
    noticeTone = tone;
    window.setTimeout(() => {
      if (notice === message) notice = '';
    }, 5000);
  }

  /** 拉一次数据文件夹状况 + 词典清单 + 词频表清单（词频表清单用来反查引用关系） */
  async function refresh() {
    const [infoRes, dictRes, tableRes, readyRes] = await Promise.all([
      libraryInfo(),
      dictList(),
      tableList(),
      libraryReady(),
    ]);
    if (infoRes.ok) info = infoRes.data;
    if (dictRes.ok) {
      dicts = dictRes.data;
      // 列表变了（导入 / 删除），待确认的那一项可能已经不存在了
      if (pendingDelete && !dicts.some((item) => item.file_name === pendingDelete)) {
        pendingDelete = null;
      }
    } else {
      loadError = dictRes.error;
    }
    if (tableRes.ok) tables = tableRes.data;
    if (readyRes.ok) ready = readyRes.data;
  }

  $effect(() => {
    void (async () => {
      loading = true;
      loadError = '';
      await refresh();
      loading = false;
    })();
  });

  async function changeDataDir() {
    const picked = await pickDirectory(t('dicts.pickDataDir'));
    if (!picked.ok) {
      showNotice(picked.error, 'error');
      return;
    }
    if (!picked.data) return; // 用户取消
    busy = true;
    const res = await setDataDir(picked.data);
    if (res.ok) {
      info = res.data;
      pendingDelete = null;
      await refresh();
      showNotice(t('dicts.dataDirChanged', { dir: res.data.root }), 'success');
    } else {
      showNotice(res.error, 'error');
    }
    busy = false;
  }

  async function importDict() {
    const picked = await pickFile(t('dicts.pickDictFile'), [
      { name: t('dicts.dictFilter'), extensions: ['dict'] },
    ]);
    if (!picked.ok) {
      showNotice(picked.error, 'error');
      return;
    }
    if (!picked.data) return; // 用户取消
    busy = true;
    const res = await dictImport(picked.data);
    busy = false;
    if (!res.ok) {
      showNotice(res.error, 'error');
      return;
    }
    await refresh();
    showNotice(t('dicts.imported', { name: res.data }), 'success');
  }

  async function confirmDelete(item: DictItem) {
    const users = usedBy(item);
    busy = true;
    const res = await dictDelete(item.file_name);
    busy = false;
    if (!res.ok) {
      showNotice(res.error, 'error');
      return;
    }
    pendingDelete = null;
    await refresh();
    showNotice(
      users.length > 0
        ? t('dicts.deletedWithUsers', {
            name: item.file_name,
            tables: users.join(t('common.listSeparator')),
          })
        : t('dicts.deleted', { name: item.file_name }),
      'success'
    );
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      {t('dicts.browserPreview')}
    </div>
  {/if}

  {#if notice}
    <div
      class={cn(
        'rounded-lg border px-3 py-2 text-xs',
        noticeTone === 'error'
          ? 'border-destructive/30 bg-destructive/5 text-destructive'
          : noticeTone === 'success'
            ? 'border-emerald-500/30 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400'
            : 'border-primary/30 bg-primary/5 text-primary'
      )}
      data-testid="dicts-notice"
    >
      {notice}
    </div>
  {/if}

  {#if loading}
    <Card>
      <CardContent class="py-8 text-center text-xs text-muted-foreground">
        {t('dicts.loading')}
      </CardContent>
    </Card>
  {:else}
    <!-- ============================ 数据文件夹 ============================ -->
    <SectionCard
      section={PAGE_SECTIONS.datadir}
      descriptionKey="dicts.dataDirDescription"
      data-testid="dicts-library-card"
    >
      {#snippet titleExtra()}
        {#if info?.is_default}
          <Badge variant="outline">{t('dicts.defaultLocation')}</Badge>
        {/if}
        {#if hasUsable}
          <Badge variant="success">{t('dicts.readyBadge')}</Badge>
        {:else}
          <Badge variant="outline" class="border-destructive/40 text-destructive">
            {t('dicts.notReadyBadge')}
          </Badge>
        {/if}
      {/snippet}
        <div class="flex flex-wrap items-center gap-2">
          <code
            class="selectable min-w-64 flex-1 truncate rounded-md border border-border bg-surface-muted/50 px-3 py-1.5 font-mono text-xs"
            data-testid="dicts-root"
          >
            {info?.root ?? '—'}
          </code>
          <Button variant="outline" size="sm" disabled={!isTauri() || !info} onclick={() => info && void openExternal(info.root)}>
            {t('dicts.openDataDir')}
          </Button>
          <Button variant="outline" size="sm" disabled={!isTauri() || !info} onclick={() => info && void openExternal(info.dicts_dir)}>
            {t('dicts.openDictsDir')}
          </Button>
          <Button size="sm" disabled={busy} onclick={() => void changeDataDir()}>
            {t('dicts.changeDataDir')}
          </Button>
        </div>

        <p class="text-[11px] text-muted-foreground">
          {t('dicts.dataDirLayout', {
            dicts: info?.dicts_dir ?? 'dicts',
            tables: info?.tables_dir ?? 'tables',
          })}
        </p>

        {#if !hasUsable}
          <p
            class="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive"
            data-testid="dicts-no-usable"
          >
            {t('dicts.noUsableDict')}
          </p>
        {/if}

        {#if ready !== hasUsable}
          <p class="text-[11px] text-muted-foreground">{t('dicts.readyMismatch')}</p>
        {/if}
  </SectionCard>

    {#if loadError}
      <Card class="border-destructive/30">
        <CardContent class="py-6 text-xs text-destructive">
          {t('dicts.loadFailed', { error: loadError })}
        </CardContent>
      </Card>
    {/if}

    <!-- ============================ 词典清单 ============================ -->
    <SectionCard section={PAGE_SECTIONS.list} descriptionKey="dicts.listDescription" data-testid="dicts-list-card">
      {#snippet titleExtra()}
        <Badge variant="secondary">{t('dicts.countBadge', { count: formatInt(dicts.length) })}</Badge>
        <Button variant="outline" size="sm" class="ml-auto" disabled={busy} onclick={() => void importDict()}>
          {t('dicts.import')}
        </Button>
      {/snippet}
        {#if dicts.length === 0}
          <p class="rounded-lg border border-dashed border-border px-3 py-6 text-center text-xs text-muted-foreground">
            {t('dicts.empty')}
          </p>
        {/if}

        {#each dicts as item (item.file_name)}
          {@const users = usedBy(item)}
          {@const broken = item.error !== null}
          <div
            class={cn(
              'flex flex-col gap-2 rounded-lg border p-3',
              broken ? 'border-destructive/40 bg-destructive/5' : 'border-border'
            )}
            data-dict-row={item.file_name}
          >
            <div class="flex flex-wrap items-center gap-2">
              <span class="font-medium" data-testid="dict-name">{item.dict.name || item.file_name}</span>
              <Badge variant="outline">{t(ORIGIN_LABELS[item.origin])}</Badge>
              {#if broken}
                <Badge variant="outline" class="border-destructive/40 text-destructive">
                  {t('dicts.brokenBadge')}
                </Badge>
              {/if}
              <span class="text-[11px] text-muted-foreground">
                {t('dicts.entries', { count: formatInt(entriesOf(item)) })}
              </span>
              <span class="ml-auto flex flex-wrap items-center gap-1.5">
                <Button variant="outline" size="sm" onclick={() => void openExternal(item.dict.path)}>
                  {t('dicts.revealFile')}
                </Button>
                <Button
                  variant={pendingDelete === item.file_name ? 'destructive' : 'ghost'}
                  size="sm"
                  disabled={busy}
                  onclick={() => (pendingDelete = pendingDelete === item.file_name ? null : item.file_name)}
                >
                  {t('dicts.delete')}
                </Button>
              </span>
            </div>

            <p class="selectable truncate font-mono text-[11px] text-muted-foreground">
              {item.file_name} · {item.dict.path}
            </p>

            {#if broken}
              <p class="rounded-md border border-destructive/40 bg-destructive/5 px-2 py-1.5 text-[11px] text-destructive">
                {t('dicts.error', { error: item.error ?? '' })}
              </p>
            {/if}

            <!-- 隐患提示：只显示确实存在的那几条，全为 0 时什么都不显示 -->
            {#if item.report.freq_zero > 0}
              <p
                class="rounded-md border border-amber-500/40 bg-amber-500/5 px-2 py-1.5 text-[11px] text-amber-700 dark:text-amber-300"
                data-testid="dict-freq-zero"
              >
                {t('dicts.warnFreqZero', { count: formatInt(item.report.freq_zero) })}
              </p>
            {/if}
            {#if item.report.freq_omitted > 0}
              <p class="rounded-md border border-amber-500/30 bg-amber-500/5 px-2 py-1.5 text-[11px] text-amber-700 dark:text-amber-300">
                {t('dicts.warnFreqOmitted', { count: formatInt(item.report.freq_omitted) })}
              </p>
            {/if}
            {#if item.report.comments > 0 || item.report.blanks > 0}
              <p class="text-[11px] text-muted-foreground">
                {t('dicts.skippedLines', {
                  comments: formatInt(item.report.comments),
                  blanks: formatInt(item.report.blanks),
                })}
              </p>
            {/if}

            {#if users.length > 0}
              <p class="text-[11px] text-muted-foreground" data-testid="dict-used-by">
                {t('dicts.usedByTables', { tables: users.join(t('common.listSeparator')) })}
              </p>
            {/if}

            <!-- 删除确认：就地展开，列出用到它的表（删了那些表就「词典缺失」） -->
            {#if pendingDelete === item.file_name}
              <div
                class="flex flex-col gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-[11px] text-destructive"
                data-testid="dict-delete-confirm"
              >
                <p class="font-medium">{t('dicts.confirmDeleteTitle', { name: item.file_name })}</p>
                {#if users.length > 0}
                  <p>
                    {t('dicts.confirmDeleteUsed', {
                      count: users.length,
                      tables: users.join(t('common.listSeparator')),
                    })}
                  </p>
                {/if}
                <p>{t('dicts.confirmDeleteNote')}</p>
                <span class="flex gap-1.5">
                  <Button variant="destructive" size="sm" disabled={busy} onclick={() => void confirmDelete(item)}>
                    {t('dicts.confirmDeleteYes')}
                  </Button>
                  <Button variant="outline" size="sm" onclick={() => (pendingDelete = null)}>
                    {t('common.cancel')}
                  </Button>
                </span>
              </div>
            {/if}
          </div>
        {/each}

        <Separator />

        <p class="text-[11px] leading-relaxed text-muted-foreground">{t('dicts.noPermissionNote')}</p>
  </SectionCard>
  {/if}
</div>
