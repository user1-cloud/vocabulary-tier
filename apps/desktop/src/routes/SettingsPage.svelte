<script lang="ts">
  /**
   * 设置 —— 全局热键、悬浮小窗外观、主题、分词默认参数与默认目录。
   *
   * 保存走 `set_settings`（bridge.setSettings）。字段名与冻结接口的 Settings
   * 结构一一对应，不做 camelCase 转换，方便和 Rust 侧对照。
   */
  import { Badge } from '$lib/components/ui/badge';
  import { Button } from '$lib/components/ui/button';
  import {
    Card,
    CardContent,
    CardDescription,
    CardHeader,
    CardTitle,
  } from '$lib/components/ui/card';
  import Input from '$lib/components/ui/input/Input.svelte';
  import { Separator } from '$lib/components/ui/separator';
  import { Switch } from '$lib/components/ui/switch';
  import {
    appInfo,
    captureSelection,
    copyText,
    defaultSettings,
    ensureDataDirs,
    getSettings,
    isTauri,
    openExternal,
    pickDirectory,
  } from '$lib/api/bridge';
  import { adoptSettings, saveSettingsRespectingTierState } from '$lib/tiers.svelte';
  import { setTheme, theme, themeLabel, type ThemeMode } from '$lib/theme.svelte';
  import { availableLocales, locale, t } from '$lib/i18n.svelte';
  import { changeLocale } from '$lib/locale-sync';
  import { cn } from '$lib/utils';
  import type { AppInfo, Settings } from '$lib/types';

  /** 形如 Alt+Q / Ctrl+Shift+Q：至少一个修饰键 + 单个字母或数字 */
  const HOTKEY_PATTERN =
    /^(Ctrl|Control|Alt|Shift|Super|Meta|Cmd)(\+(Ctrl|Control|Alt|Shift|Super|Meta|Cmd))*\+[A-Za-z0-9]$/;

  const THEME_MODES: ThemeMode[] = ['light', 'dark', 'system'];

  let form = $state<Settings>(defaultSettings());
  let loading = $state(true);
  let saving = $state(false);
  let dirty = $state(false);

  let statusMessage = $state('');
  let statusTone = $state<'info' | 'error' | 'success'>('info');
  let info = $state<AppInfo | null>(null);
  let infoError = $state('');
  let pickerHint = $state('');

  const hotkeyValid = $derived(HOTKEY_PATTERN.test(form.hotkey.trim()));
  const popupSizeValid = $derived(form.popupWidth >= 200 && form.popupHeight >= 150);
  const canSave = $derived(!saving && hotkeyValid && popupSizeValid);

  // ---------------------------------------------------------------- 生命周期

  $effect(() => {
    void bootstrap();
  });

  function markDirty() {
    dirty = true;
    statusMessage = '';
  }

  /** 所有修改都经过这里，保证 dirty 标记不遗漏 */
  function patch(part: Partial<Settings>) {
    form = { ...form, ...part };
    markDirty();
  }

  async function bootstrap() {
    loading = true;
    const [settingsRes, infoRes] = await Promise.all([getSettings(), appInfo()]);
    if (settingsRes.ok) {
      // ⚠️ 主题与语言都以**共享状态**里的实时值为准（见下面 `applyTheme` / `applyLocale`）：
      // 设置文件里的这两个字段只在点「保存设置」时写入，读回来会把用户刚切、还没保存的
      // 选择顶回去（顶栏 / 小窗的主题、本页刚切的语言都会中招）。
      form = { ...settingsRes.data, theme: theme.mode, locale: locale.value };
    } else {
      statusTone = 'error';
      statusMessage = t('settings.readFailed', { error: settingsRes.error });
    }
    if (infoRes.ok) info = infoRes.data;
    else infoError = infoRes.error;
    loading = false;
  }

  async function save() {
    saving = true;
    statusMessage = '';
    // 分组自定义 / 表开关这几个字段的权威来源是 $lib/tiers.svelte.ts 里的共享状态：
    // 本页可能是在「表管理」页改完之后才打开的，直接拿本页 form 里的旧值会把它冲掉。
    // theme / locale 同理，而且它们比这几个字段更容易过期：顶栏、小窗随时能切主题，
    // 语言在本页也是「切了立即生效」，所以**总是**以共享状态里的实时值为准
    // （form 只用来标记「改过」，不作为权威值）。
    const res = await saveSettingsRespectingTierState(
      { ...form, theme: theme.mode, locale: locale.value, hotkey: form.hotkey.trim() },
      ['tierMethod', 'tierWordBounds', 'tierCharBounds', 'tierCoverage']
    );
    saving = false;
    if (!res.ok) {
      statusTone = 'error';
      statusMessage = t('settings.saveFailed', { error: res.error });
      return;
    }
    form = { ...res.data, theme: theme.mode, locale: locale.value };
    adoptSettings(res.data);
    dirty = false;
    statusTone = 'success';
    statusMessage = t('settings.saved');
    // 这里**不要**再回写 theme / locale：落盘的就是刚发过去的实时值，而保存期间用户
    // 还可能在顶栏再切一次主题、或再点一次语言，回写会把他那次切换顶回去。
  }

  /**
   * 切换界面语言。
   *
   * 与主题按钮同一套行为：**立即生效**（能立刻看到界面变成目标语言），
   * 同时标脏；真正落盘要等用户点「保存设置」。`changeLocale` 负责本窗口状态 +
   * localStorage + 广播给小窗。
   */
  function applyLocale(next: string) {
    changeLocale(next);
    patch({ locale: next });
  }

  function resetToDefaults() {
    // 主题与语言是「本机偏好」，恢复默认值时不跟着重置，否则用户会突然看不懂界面
    form = { ...defaultSettings(), theme: form.theme, locale: form.locale };
    markDirty();
    statusTone = 'info';
    statusMessage = t('settings.resetHint');
  }

  async function chooseDir(field: 'corpusDir' | 'dataDir') {
    pickerHint = '';
    const res = await pickDirectory(
      field === 'corpusDir' ? t('settings.pickCorpusDir') : t('settings.pickDataDir')
    );
    if (!res.ok) {
      pickerHint = res.error;
      return;
    }
    if (res.data) patch({ [field]: res.data } as Partial<Settings>);
  }

  /** 打开数据文件夹（里面是 dicts\ 与 tables\）。路径为空时让后端用默认位置。 */
  async function openDataDir() {
    pickerHint = '';
    if (!form.dataDir) {
      const res = await ensureDataDirs();
      if (!res.ok) {
        pickerHint = res.error;
        return;
      }
      patch({ dataDir: res.data });
      await openExternal(res.data);
      return;
    }
    const opened = await openExternal(form.dataDir);
    if (!opened.ok) pickerHint = opened.error;
  }

  async function testCapture() {
    const res = await captureSelection();
    if (!res.ok) {
      statusTone = 'error';
      statusMessage = res.error;
      return;
    }
    if (!res.data) {
      statusTone = 'info';
      statusMessage = t('settings.captureNoText');
      return;
    }
    const copied = await copyText(res.data);
    statusTone = copied.ok ? 'success' : 'info';
    statusMessage = copied.ok
      ? t('settings.captureOkCopied', { count: res.data.length })
      : t('settings.captureOkText', { text: res.data });
  }

  /**
   * 切换主题：**立即生效**（`setTheme` 写共享状态 + localStorage + 广播给顶栏与小窗），
   * 同时标脏；真正落盘要等用户点「保存设置」。
   *
   * 选中态读的是共享状态 `theme.mode`（见模板），不是本页 `form.theme`：顶栏 / 小窗
   * 也改同一个值，读表单里的副本就会和它们脱钩。
   */
  function applyTheme(value: ThemeMode) {
    patch({ theme: value });
    setTheme(value);
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      {t('settings.browserPreview')}
    </div>
  {/if}

  {#if statusMessage}
    <div
      class={cn(
        'rounded-lg border px-3 py-2 text-xs',
        statusTone === 'error'
          ? 'border-destructive/30 bg-destructive/5 text-destructive'
          : statusTone === 'success'
            ? 'border-emerald-500/30 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400'
            : 'border-primary/30 bg-primary/5 text-primary'
      )}
    >
      {statusMessage}
    </div>
  {/if}

  {#if loading}
    <p class="text-xs text-muted-foreground">{t('settings.loading')}</p>
  {:else}
    <!-- 全局取词 -->
    <Card data-section="hotkey">
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>{t('settings.hotkey.title')}</CardTitle>
          <Badge variant="outline">hotkey</Badge>
          {#if hotkeyValid}
            <Badge variant="success">{t('settings.hotkey.valid')}</Badge>
          {:else}
            <Badge variant="outline" class="border-destructive/40 text-destructive">
              {t('settings.hotkey.invalid')}
            </Badge>
          {/if}
        </div>
        <CardDescription>
          {t('settings.hotkey.description')}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">{t('settings.hotkey.label')}</span>
          <Input
            bind:value={form.hotkey}
            oninput={markDirty}
            placeholder="Alt+Q"
            class="max-w-56 font-mono text-xs"
          />
          <span class="text-[11px] text-muted-foreground">
            {t('settings.hotkey.hintPrefix')}<span class="font-mono">Alt+Q</span>{t(
              'settings.hotkey.hintSeparator'
            )}<span class="font-mono">Ctrl+Shift+Q</span>{t('settings.hotkey.hintSuffix')}
          </span>
        </label>

        <div class="flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" onclick={() => void testCapture()}>
            {t('settings.testCapture')}
          </Button>
          <span class="text-[11px] text-muted-foreground">
            {t('settings.testCaptureHint')}
          </span>
        </div>

        <!-- 管理员权限说明 -->
        <div
          class="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] leading-relaxed text-amber-700 dark:text-amber-300"
        >
          <p class="font-medium">{t('settings.adminNote.title')}</p>
          <p class="mt-1">
            {t('settings.adminNote.p1Before')}<b>{t('settings.adminNote.p1Emphasis')}</b>{t(
              'settings.adminNote.p1After'
            )}
          </p>
          <p class="mt-1">
            {t('settings.adminNote.p2')}
          </p>
        </div>
      </CardContent>
    </Card>

    <!-- 悬浮小窗 -->
    <Card data-section="popup">
      <CardHeader>
        <CardTitle>{t('settings.popup.title')}</CardTitle>
        <CardDescription>{t('settings.popup.description')}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-4">
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-3">
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">{t('settings.popup.width')}</span>
            <Input
              type="number"
              min="200"
              bind:value={form.popupWidth}
              oninput={markDirty}
              class="text-xs"
            />
          </label>
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">{t('settings.popup.height')}</span>
            <Input
              type="number"
              min="150"
              bind:value={form.popupHeight}
              oninput={markDirty}
              class="text-xs"
            />
          </label>
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">{t('settings.popup.autoClose')}</span>
            <Input
              type="number"
              min="0"
              bind:value={form.popupAutoCloseMs}
              oninput={markDirty}
              class="text-xs"
            />
          </label>
        </div>

        {#if !popupSizeValid}
          <p class="text-[11px] text-destructive">{t('settings.popup.minSize')}</p>
        {/if}

        <label class="flex flex-col gap-2">
          <span class="flex items-center justify-between text-xs font-medium">
            <span>{t('settings.popup.opacity')}</span>
            <span class="tabular-nums text-muted-foreground">{form.popupOpacity.toFixed(2)}</span>
          </span>
          <input
            type="range"
            min="0.3"
            max="1"
            step="0.01"
            value={form.popupOpacity}
            class="h-1.5 w-full cursor-pointer accent-[var(--primary)]"
            oninput={(event) => patch({ popupOpacity: Number(event.currentTarget.value) })}
            aria-label={t('settings.popup.opacityLabel')}
          />
        </label>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">{t('settings.popup.alwaysOnTop')}</p>
            <p class="text-[11px] text-muted-foreground">{t('settings.popup.alwaysOnTopHint')}</p>
          </div>
          <Switch
            checked={form.popupAlwaysOnTop}
            onCheckedChange={(checked) => patch({ popupAlwaysOnTop: checked })}
            aria-label={t('settings.popup.alwaysOnTop')}
          />
        </div>
      </CardContent>
    </Card>

    <!-- 主题 -->
    <Card data-section="appearance">
      <CardHeader>
        <CardTitle>{t('settings.appearance.title')}</CardTitle>
        <CardDescription>{t('settings.appearance.description')}</CardDescription>
      </CardHeader>
      <CardContent>
        <!-- 选中态读共享状态 theme.mode（与顶栏按钮、小窗同一份），不读本页表单副本 -->
        <div class="flex flex-wrap items-center gap-2" data-testid="theme-picker">
          {#each THEME_MODES as mode (mode)}
            <button
              type="button"
              aria-pressed={theme.mode === mode}
              data-theme-mode={mode}
              class={cn(
                'rounded-md border px-3 py-1.5 text-xs transition-colors',
                theme.mode === mode
                  ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                  : 'border-border hover:bg-accent'
              )}
              onclick={() => applyTheme(mode)}
            >
              {themeLabel(mode)}
            </button>
          {/each}
          <span class="text-[11px] text-muted-foreground">
            {t('settings.appearance.currentTheme', { theme: themeLabel(theme.mode) })}
          </span>
        </div>
      </CardContent>
    </Card>

    <!-- 界面语言 -->
    <Card data-section="language">
      <CardHeader>
        <CardTitle>{t('settings.language.title')}</CardTitle>
        <CardDescription>{t('settings.language.description')}</CardDescription>
      </CardHeader>
      <CardContent>
        <div class="flex flex-wrap items-center gap-2" data-testid="locale-picker">
          {#each availableLocales() as option (option.value)}
            <button
              type="button"
              aria-pressed={locale.value === option.value}
              data-locale={option.value}
              class={cn(
                'rounded-md border px-3 py-1.5 text-xs transition-colors',
                locale.value === option.value
                  ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                  : 'border-border hover:bg-accent'
              )}
              onclick={() => applyLocale(option.value)}
            >
              {option.label}
            </button>
          {/each}
          {#if availableLocales().length < 2}
            <span class="text-[11px] text-muted-foreground">
              {t('settings.language.onlyOne')}
            </span>
          {/if}
        </div>
      </CardContent>
    </Card>

    <!-- 默认分词参数 -->
    <Card data-section="tokenize">
      <CardHeader>
        <CardTitle>{t('settings.tokenize.title')}</CardTitle>
        <CardDescription>{t('settings.tokenize.description')}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-4">
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">{t('settings.tokenize.threads')}</span>
            <Input
              type="number"
              min="0"
              bind:value={form.threads}
              oninput={markDirty}
              class="text-xs"
            />
          </label>
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">{t('settings.tokenize.minCount')}</span>
            <Input
              type="number"
              min="1"
              bind:value={form.minCount}
              oninput={markDirty}
              class="text-xs"
            />
          </label>
        </div>

        <Separator />

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
            <div class="min-w-0">
              <p class="text-xs font-medium">{t('settings.tokenize.hmm')}</p>
              <p class="text-[11px] text-muted-foreground">{t('settings.tokenize.hmmHint')}</p>
            </div>
            <Switch
              checked={form.hmm}
              onCheckedChange={(checked) => patch({ hmm: checked })}
              aria-label={t('settings.tokenize.hmm')}
            />
          </div>

          <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
            <div class="min-w-0">
              <p class="text-xs font-medium">{t('settings.tokenize.keepDigit')}</p>
              <p class="text-[11px] text-muted-foreground">{t('settings.tokenize.keepDigitHint')}</p>
            </div>
            <Switch
              checked={form.keepDigit}
              onCheckedChange={(checked) => patch({ keepDigit: checked })}
              aria-label={t('settings.tokenize.keepDigit')}
            />
          </div>

          <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
            <div class="min-w-0">
              <p class="text-xs font-medium">{t('settings.tokenize.keepLatin')}</p>
              <p class="text-[11px] text-muted-foreground">{t('settings.tokenize.keepLatinHint')}</p>
            </div>
            <Switch
              checked={form.keepLatin}
              onCheckedChange={(checked) => patch({ keepLatin: checked })}
              aria-label={t('settings.tokenize.keepLatin')}
            />
          </div>

          <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
            <div class="min-w-0">
              <p class="text-xs font-medium">{t('settings.tokenize.skipSingleChar')}</p>
              <p class="text-[11px] text-muted-foreground">
                {t('settings.tokenize.skipSingleCharHint')}
              </p>
            </div>
            <Switch
              checked={form.skipSingleChar}
              onCheckedChange={(checked) => patch({ skipSingleChar: checked })}
              aria-label={t('settings.tokenize.skipSingleChar')}
            />
          </div>

          <div
            class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2 sm:col-span-2"
          >
            <div class="min-w-0">
              <p class="text-xs font-medium">{t('settings.tokenize.domainTables')}</p>
              <p class="text-[11px] text-muted-foreground">
                {t('settings.tokenize.domainTablesHint')}
              </p>
            </div>
            <Switch
              checked={!form.skipDomainTables}
              onCheckedChange={(checked) => patch({ skipDomainTables: !checked })}
              aria-label={t('settings.tokenize.domainTables')}
            />
          </div>
        </div>
      </CardContent>
    </Card>

    <!-- 目录 -->
    <Card data-section="paths">
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>{t('settings.paths.title')}</CardTitle>
          <Badge variant="outline">{t('settings.paths.dataDirBadge')}</Badge>
        </div>
        <CardDescription>{t('settings.paths.description')}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">{t('settings.paths.corpusDir')}</span>
          <span class="flex flex-wrap items-center gap-2">
            <Input
              value={form.corpusDir ?? ''}
              oninput={(event) => patch({ corpusDir: event.currentTarget.value || null })}
              placeholder={t('settings.paths.unset')}
              class="min-w-56 flex-1 font-mono text-xs"
            />
            <Button variant="outline" size="sm" onclick={() => void chooseDir('corpusDir')}>
              {t('common.browse')}
            </Button>
            {#if form.corpusDir}
              <Button variant="ghost" size="sm" onclick={() => patch({ corpusDir: null })}>{t('common.clear')}</Button>
            {/if}
          </span>
        </label>

        <!-- 数据文件夹：里面是 dicts\（词典库）与 tables\（词频表库） -->
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">{t('settings.paths.dataDir')}</span>
          <span class="flex flex-wrap items-center gap-2">
            <Input
              value={form.dataDir ?? ''}
              oninput={(event) => patch({ dataDir: event.currentTarget.value || null })}
              placeholder={t('settings.paths.unset')}
              class="min-w-56 flex-1 font-mono text-xs"
            />
            <Button variant="outline" size="sm" onclick={() => void chooseDir('dataDir')}>
              {t('common.browse')}
            </Button>
            <Button variant="outline" size="sm" disabled={!isTauri()} onclick={() => void openDataDir()}>
              {t('settings.paths.openDataDir')}
            </Button>
            {#if form.dataDir}
              <Button variant="ghost" size="sm" onclick={() => patch({ dataDir: null })}>{t('common.clear')}</Button>
            {/if}
          </span>
          <span class="text-[11px] leading-relaxed text-muted-foreground">
            {t('settings.paths.dataDirHint')}
          </span>
        </label>

        <Separator />

        <!-- `userDict` 已废弃：词典现在是数据文件夹里的条目，在扫描时勾选 -->
        <div
          class="flex flex-col gap-1 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] leading-relaxed text-amber-700 dark:text-amber-300"
          data-testid="userdict-deprecated"
        >
          <p class="font-medium">{t('settings.paths.userDictDeprecatedTitle')}</p>
          <p>{t('settings.paths.userDictDeprecatedBody')}</p>
          {#if form.userDict}
            <p>{t('settings.paths.userDictWillMigrate', { path: form.userDict })}</p>
          {/if}
        </div>

        {#if pickerHint}
          <p class="text-[11px] text-muted-foreground">{pickerHint}</p>
        {/if}
      </CardContent>
    </Card>

    <!-- 保存 -->
    <div class="flex flex-wrap items-center gap-3">
      <Button disabled={!canSave} onclick={() => void save()}>
        {saving ? t('settings.save.saving') : t('settings.save.save')}
      </Button>
      <Button variant="outline" onclick={resetToDefaults}>{t('settings.save.reset')}</Button>
      {#if dirty}
        <span class="text-[11px] text-amber-600 dark:text-amber-400">{t('settings.save.dirty')}</span>
      {/if}
      {#if !hotkeyValid}
        <span class="text-[11px] text-destructive">{t('settings.save.hotkeyInvalid')}</span>
      {/if}
    </div>

    <!-- 关于 -->
    <Card>
      <CardHeader>
        <CardTitle>{t('settings.about.title')}</CardTitle>
        <CardDescription>{t('settings.about.description')}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        {#if info}
          <dl class="grid grid-cols-2 gap-3 text-xs sm:grid-cols-4">
            <div>
              <dt class="text-muted-foreground">{t('settings.about.app')}</dt>
              <dd class="font-medium">{info.name}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">{t('settings.about.appVersion')}</dt>
              <dd class="font-medium tabular-nums">v{info.version}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">Tauri</dt>
              <dd class="font-medium tabular-nums">{info.tauri_version}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">vocfreq-core</dt>
              <dd class="font-medium tabular-nums">{info.core_version}</dd>
            </div>
          </dl>
        {:else}
          <p class="text-xs text-muted-foreground">
            {infoError
              ? t('settings.about.infoUnavailableWithError', { error: infoError })
              : t('settings.about.infoUnavailable')}
          </p>
        {/if}
        <p class="text-[11px] text-muted-foreground">
          {t('settings.about.footnote')}
        </p>
      </CardContent>
    </Card>
  {/if}
</div>
