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
    getSettings,
    isTauri,
    pickDirectory,
    pickFile,
  } from '$lib/api/bridge';
  import { adoptSettings, saveSettingsRespectingTierState } from '$lib/tiers.svelte';
  import { THEME_LABELS, setTheme, theme, type ThemeMode } from '$lib/theme.svelte';
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

  function isThemeMode(value: string): value is ThemeMode {
    return value === 'light' || value === 'dark' || value === 'system';
  }

  async function bootstrap() {
    loading = true;
    const [settingsRes, infoRes] = await Promise.all([getSettings(), appInfo()]);
    if (settingsRes.ok) {
      form = { ...settingsRes.data };
      // 让顶栏的主题按钮与这里的选择保持同步
      if (isThemeMode(settingsRes.data.theme) && settingsRes.data.theme !== theme.mode) {
        setTheme(settingsRes.data.theme);
      }
    } else {
      statusTone = 'error';
      statusMessage = `读取设置失败：${settingsRes.error}`;
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
    const res = await saveSettingsRespectingTierState({ ...form, hotkey: form.hotkey.trim() }, [
      'tierMethod',
      'tierWordBounds',
      'tierCharBounds',
      'tierCoverage',
    ]);
    saving = false;
    if (!res.ok) {
      statusTone = 'error';
      statusMessage = `保存失败：${res.error}`;
      return;
    }
    form = { ...res.data };
    adoptSettings(res.data);
    dirty = false;
    statusTone = 'success';
    statusMessage = '设置已保存。';
    if (isThemeMode(res.data.theme)) setTheme(res.data.theme);
  }

  function resetToDefaults() {
    form = { ...defaultSettings(), theme: form.theme };
    markDirty();
    statusTone = 'info';
    statusMessage = '已恢复默认值，记得点「保存设置」。';
  }

  async function chooseDir(field: 'corpusDir' | 'dataDir') {
    pickerHint = '';
    const res = await pickDirectory(
      field === 'corpusDir' ? '选择默认语料库目录' : '选择默认输出目录'
    );
    if (!res.ok) {
      pickerHint = res.error;
      return;
    }
    if (res.data) patch({ [field]: res.data } as Partial<Settings>);
  }

  async function chooseUserDict() {
    pickerHint = '';
    const res = await pickFile('选择自定义词典');
    if (!res.ok) {
      pickerHint = res.error;
      return;
    }
    if (res.data) patch({ userDict: res.data });
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
      statusMessage = '没有检测到选中的文本。请先在别的程序里选一段文字再试。';
      return;
    }
    const copied = await copyText(res.data);
    statusTone = copied.ok ? 'success' : 'info';
    statusMessage = copied.ok
      ? `取词成功（${res.data.length} 字），已复制到剪贴板。`
      : `取词成功：${res.data}`;
  }

  function applyTheme(value: ThemeMode) {
    patch({ theme: value });
    setTheme(value);
  }
</script>

<div class="flex flex-col gap-4">
  {#if !isTauri()}
    <div class="rounded-lg border border-dashed border-border bg-surface-muted/40 px-3 py-2 text-xs text-muted-foreground">
      浏览器预览模式：设置可以编辑与「保存」，但不会写入磁盘（仅保存在内存里）。
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
    <p class="text-xs text-muted-foreground">正在读取设置…</p>
  {:else}
    <!-- 全局取词 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center gap-2">
          <CardTitle>全局取词</CardTitle>
          <Badge variant="outline">hotkey</Badge>
          {#if hotkeyValid}
            <Badge variant="success">格式正确</Badge>
          {:else}
            <Badge variant="outline" class="border-destructive/40 text-destructive">格式待修正</Badge>
          {/if}
        </div>
        <CardDescription>
          按下热键会模拟一次 Ctrl+C 取走选区文本，并在悬浮小窗里做频率分析。
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">全局热键</span>
          <Input
            bind:value={form.hotkey}
            oninput={markDirty}
            placeholder="Alt+Q"
            class="max-w-56 font-mono text-xs"
          />
          <span class="text-[11px] text-muted-foreground">
            格式：修饰键 + 主键，例如 <span class="font-mono">Alt+Q</span>、<span
              class="font-mono">Ctrl+Shift+Q</span
            >。
          </span>
        </label>

        <div class="flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" onclick={() => void testCapture()}>测试取词</Button>
          <span class="text-[11px] text-muted-foreground">
            测试会在当前前台程序里模拟一次复制，并把结果写入剪贴板。
          </span>
        </div>

        <!-- 管理员权限说明 -->
        <div
          class="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] leading-relaxed text-amber-700 dark:text-amber-300"
        >
          <p class="font-medium">关于「以管理员身份运行」</p>
          <p class="mt-1">
            全局取词的原理是向当前焦点窗口发送 Ctrl+C。Windows 的 UIPI（用户界面特权隔离）规定：
            <b>低完整性级别的进程不能向高完整性级别的进程发送输入</b>。因此当目标程序（例如以管理员身份运行的
            编辑器、IDE 或终端）权限高于 VocTier 时，热键取词会被系统静默拦截，表现为「按了没反应」。
          </p>
          <p class="mt-1">
            解决办法：右键 VocTier 快捷方式 →「以管理员身份运行」，让两者权限一致（都以管理员运行，
            或都不以管理员运行）。这一限制来自 Windows 本身，应用无法绕过。
          </p>
        </div>
      </CardContent>
    </Card>

    <!-- 悬浮小窗 -->
    <Card>
      <CardHeader>
        <CardTitle>悬浮小窗</CardTitle>
        <CardDescription>取词后弹出的小窗尺寸、透明度与自动关闭行为。</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-4">
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-3">
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">宽度（px）</span>
            <Input
              type="number"
              min="200"
              bind:value={form.popupWidth}
              oninput={markDirty}
              class="text-xs"
            />
          </label>
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">高度（px）</span>
            <Input
              type="number"
              min="150"
              bind:value={form.popupHeight}
              oninput={markDirty}
              class="text-xs"
            />
          </label>
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">自动关闭（毫秒，0 = 不自动关）</span>
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
          <p class="text-[11px] text-destructive">小窗至少 200 × 150 像素。</p>
        {/if}

        <label class="flex flex-col gap-2">
          <span class="flex items-center justify-between text-xs font-medium">
            <span>不透明度</span>
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
            aria-label="小窗不透明度"
          />
        </label>

        <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
          <div class="min-w-0">
            <p class="text-xs font-medium">窗口置顶</p>
            <p class="text-[11px] text-muted-foreground">小窗始终显示在其它窗口之上</p>
          </div>
          <Switch
            checked={form.popupAlwaysOnTop}
            onCheckedChange={(checked) => patch({ popupAlwaysOnTop: checked })}
            aria-label="窗口置顶"
          />
        </div>
      </CardContent>
    </Card>

    <!-- 主题 -->
    <Card>
      <CardHeader>
        <CardTitle>外观</CardTitle>
        <CardDescription>与顶栏的主题按钮共用同一份状态。</CardDescription>
      </CardHeader>
      <CardContent>
        <div class="flex flex-wrap items-center gap-2">
          {#each THEME_MODES as mode (mode)}
            <button
              type="button"
              aria-pressed={form.theme === mode}
              class={cn(
                'rounded-md border px-3 py-1.5 text-xs transition-colors',
                form.theme === mode
                  ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                  : 'border-border hover:bg-accent'
              )}
              onclick={() => applyTheme(mode)}
            >
              {THEME_LABELS[mode]}
            </button>
          {/each}
          <span class="text-[11px] text-muted-foreground">当前生效：{THEME_LABELS[theme.mode]}</span>
        </div>
      </CardContent>
    </Card>

    <!-- 默认分词参数 -->
    <Card>
      <CardHeader>
        <CardTitle>默认统计参数</CardTitle>
        <CardDescription>「生成词频表」页打开时会用这里的值作为初始值。</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-4">
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">默认线程数（0 = 自动）</span>
            <Input
              type="number"
              min="0"
              bind:value={form.threads}
              oninput={markDirty}
              class="text-xs"
            />
          </label>
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium">最小词频</span>
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
              <p class="text-xs font-medium">HMM 新词发现</p>
              <p class="text-[11px] text-muted-foreground">识别词典外的连续汉字组合</p>
            </div>
            <Switch
              checked={form.hmm}
              onCheckedChange={(checked) => patch({ hmm: checked })}
              aria-label="HMM 新词发现"
            />
          </div>

          <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
            <div class="min-w-0">
              <p class="text-xs font-medium">保留数字词</p>
              <p class="text-[11px] text-muted-foreground">如 2024、3.14</p>
            </div>
            <Switch
              checked={form.keepDigit}
              onCheckedChange={(checked) => patch({ keepDigit: checked })}
              aria-label="保留数字词"
            />
          </div>

          <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
            <div class="min-w-0">
              <p class="text-xs font-medium">保留拉丁词</p>
              <p class="text-[11px] text-muted-foreground">如 API、token</p>
            </div>
            <Switch
              checked={form.keepLatin}
              onCheckedChange={(checked) => patch({ keepLatin: checked })}
              aria-label="保留拉丁词"
            />
          </div>

          <div class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
            <div class="min-w-0">
              <p class="text-xs font-medium">跳过单字词</p>
              <p class="text-[11px] text-muted-foreground">只统计多字词（字表仍会产出）</p>
            </div>
            <Switch
              checked={form.skipSingleChar}
              onCheckedChange={(checked) => patch({ skipSingleChar: checked })}
              aria-label="跳过单字词"
            />
          </div>

          <div
            class="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2 sm:col-span-2"
          >
            <div class="min-w-0">
              <p class="text-xs font-medium">默认产出分域表</p>
              <p class="text-[11px] text-muted-foreground">关闭后只产出全库词表 / 字表，产物更小</p>
            </div>
            <Switch
              checked={!form.skipDomainTables}
              onCheckedChange={(checked) => patch({ skipDomainTables: !checked })}
              aria-label="默认产出分域表"
            />
          </div>
        </div>
      </CardContent>
    </Card>

    <!-- 目录 -->
    <Card>
      <CardHeader>
        <CardTitle>默认目录与词典</CardTitle>
        <CardDescription>划句分析页从这里读取产物目录。</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">默认语料库目录</span>
          <span class="flex flex-wrap items-center gap-2">
            <Input
              value={form.corpusDir ?? ''}
              oninput={(event) => patch({ corpusDir: event.currentTarget.value || null })}
              placeholder="未设置"
              class="min-w-56 flex-1 font-mono text-xs"
            />
            <Button variant="outline" size="sm" onclick={() => void chooseDir('corpusDir')}>
              浏览…
            </Button>
            {#if form.corpusDir}
              <Button variant="ghost" size="sm" onclick={() => patch({ corpusDir: null })}>清除</Button>
            {/if}
          </span>
        </label>

        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">默认输出目录（词频表产物）</span>
          <span class="flex flex-wrap items-center gap-2">
            <Input
              value={form.dataDir ?? ''}
              oninput={(event) => patch({ dataDir: event.currentTarget.value || null })}
              placeholder="未设置"
              class="min-w-56 flex-1 font-mono text-xs"
            />
            <Button variant="outline" size="sm" onclick={() => void chooseDir('dataDir')}>
              浏览…
            </Button>
            {#if form.dataDir}
              <Button variant="ghost" size="sm" onclick={() => patch({ dataDir: null })}>清除</Button>
            {/if}
          </span>
        </label>

        <label class="flex flex-col gap-1.5">
          <span class="text-xs font-medium">自定义词典（可选）</span>
          <span class="flex flex-wrap items-center gap-2">
            <Input
              value={form.userDict ?? ''}
              oninput={(event) => patch({ userDict: event.currentTarget.value || null })}
              placeholder="每行「词 频次」"
              class="min-w-56 flex-1 font-mono text-xs"
            />
            <Button variant="outline" size="sm" onclick={() => void chooseUserDict()}>选择…</Button>
            {#if form.userDict}
              <Button variant="ghost" size="sm" onclick={() => patch({ userDict: null })}>清除</Button>
            {/if}
          </span>
        </label>

        {#if pickerHint}
          <p class="text-[11px] text-muted-foreground">{pickerHint}</p>
        {/if}
      </CardContent>
    </Card>

    <!-- 保存 -->
    <div class="flex flex-wrap items-center gap-3">
      <Button disabled={!canSave} onclick={() => void save()}>
        {saving ? '保存中…' : '保存设置'}
      </Button>
      <Button variant="outline" onclick={resetToDefaults}>恢复默认值</Button>
      {#if dirty}
        <span class="text-[11px] text-amber-600 dark:text-amber-400">有未保存的修改</span>
      {/if}
      {#if !hotkeyValid}
        <span class="text-[11px] text-destructive">热键格式不正确，请改成 Alt+Q 这类写法。</span>
      {/if}
    </div>

    <!-- 关于 -->
    <Card>
      <CardHeader>
        <CardTitle>关于</CardTitle>
        <CardDescription>版本信息来自 app_info 命令。</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        {#if info}
          <dl class="grid grid-cols-2 gap-3 text-xs sm:grid-cols-4">
            <div>
              <dt class="text-muted-foreground">应用</dt>
              <dd class="font-medium">{info.name}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">应用版本</dt>
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
            暂时读不到版本信息{infoError ? `：${infoError}` : ''}。
          </p>
        {/if}
        <p class="text-[11px] text-muted-foreground">
          快捷键、目录与小窗外观会在保存后生效；热键的重新注册由 Rust 侧负责。
        </p>
      </CardContent>
    </Card>
  {/if}
</div>
