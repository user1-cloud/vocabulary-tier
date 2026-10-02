/**
 * 界面多语言（i18n）内核。
 *
 * ## 用法
 *
 * ```svelte
 * <script>import { t } from '$lib/i18n.svelte';</script>
 * <h1>{t('nav.settings.title')}</h1>
 * ```
 *
 * `t()` 里读的是 `$state`，所以它是**响应式读点**：在模板或 `$derived` 里调用，
 * 切换语言就会自动重渲染，不需要刷新页面、也不需要手动广播。
 *
 * ## 反过来说：纯计算模块不要调 `t()`
 *
 * `format.ts::boundsInfo` 这类「算结果」的纯函数如果调 `t()`，就会把 locale 状态
 * 拖进计算层，还会让它的返回值依赖全局状态。那种地方一律返回**错误码**
 * （例如 `BoundsWarning { code, params }`），由展示层去 `t()`。
 *
 * ## 语言从哪来
 *
 * 两条路，和主题完全同构（见 `theme.svelte.ts`）：
 *   1. `localStorage`（key：`voctier-locale`）—— 首屏**同步**读，避免闪成错语言；
 *   2. 后端 `Settings.locale` —— 权威值，保证主窗口与小窗一致（两窗口的
 *      localStorage 可能被宿主隔开，见 `theme-sync.ts` 的说明）。
 *
 * ## 依赖方向（很重要，别破坏）
 *
 * 本模块**只**依赖 `./messages`（纯数据），刻意不依赖 `$lib/api/bridge`：
 * `TokenChips` / `TokenDetail` 这类被大量复用的组件要 import 它，把
 * `@tauri-apps/api` 拖进来会拖慢首屏并制造 import 环。跨窗口同步因此单独放在
 * `locale-sync.ts` 里。
 */
import {
  DEFAULT_LOCALE,
  isLocale,
  LOCALE_HTML_LANG,
  LOCALE_LABELS,
  LOCALES,
  MESSAGES,
  type Locale,
  type MessageKey,
} from './messages';

/** localStorage key（`index.html` 的首屏脚本用同一个） */
export const LOCALE_STORAGE_KEY = 'voctier-locale';

/** 同步读本地存的界面语言；读不到或非法时用源语言 */
export function readStoredLocale(): Locale {
  if (typeof localStorage === 'undefined') return DEFAULT_LOCALE;
  try {
    const raw = localStorage.getItem(LOCALE_STORAGE_KEY);
    return isLocale(raw) ? raw : DEFAULT_LOCALE;
  } catch {
    /* 隐私模式 / 存储被禁：用默认语言，不抛 */
    return DEFAULT_LOCALE;
  }
}

/** 当前界面语言（响应式）。初值同步取自 localStorage，所以首帧就是对的语言 */
export const locale = $state<{ value: Locale }>({ value: readStoredLocale() });

/** 当前语言（读值；给格式化函数这类非组件代码用） */
export function activeLocale(): Locale {
  return locale.value;
}

/**
 * 取文案。
 *
 * 查不到时逐级回落：当前语言 → 源语言 → **返回 key 本身**。最后那一步是故意的：
 * 界面上冒出一个 `nav.settings.title` 比静默显示别的语言更容易被发现。
 */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const table = MESSAGES[locale.value] ?? MESSAGES[DEFAULT_LOCALE];
  const text = table[key] ?? MESSAGES[DEFAULT_LOCALE][key] ?? key;
  return params ? interpolate(text, params) : text;
}

/** `{name}` 占位替换；参数缺失时**原样保留**占位符，方便一眼看出漏传了 */
function interpolate(text: string, params: Record<string, string | number>): string {
  return text.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole
  );
}

/** 某个 key 在当前语言下是否存在（给「这段文案要不要显示」这类判断用） */
export function hasMessage(key: MessageKey): boolean {
  return key in (MESSAGES[locale.value] ?? MESSAGES[DEFAULT_LOCALE]);
}

/**
 * 把一条消息按占位符切成若干段，供「句子中间要插入带样式的节点」的场景使用。
 *
 * ```svelte
 * {@const seg = splitMessage('wordfreq.plan.domains', ['count'], { count: n })}
 * <span class="text-muted-foreground">{seg[0]}<b class="text-foreground">{n}</b>{seg[1]}</span>
 * ```
 *
 * **为什么需要它**：界面里大量出现「把数字/关键词加粗」的句子，而消息表不放 HTML；
 * 若改成「前缀 + 值 + 后缀」几条消息，翻译就没法调整词序（英文常常是倒过来的）。
 * 按占位符切分之后，**整句仍然是一条消息**，翻译可以自由重排，只要保留占位符。
 *
 * `placeholders` 的先后顺序即切分顺序，返回值长度 = 占位符数 + 1。
 * `values` 里**除占位符以外**的参数会先在每段里替换掉（占位符本身留给调用方渲染）。
 */
export function splitMessage(
  key: MessageKey,
  placeholders: string[],
  values: Record<string, string | number> = {}
): string[] {
  const table = MESSAGES[locale.value] ?? MESSAGES[DEFAULT_LOCALE];
  const raw = table[key] ?? MESSAGES[DEFAULT_LOCALE][key] ?? key;
  let segments = [raw];
  for (const placeholder of placeholders) {
    const next: string[] = [];
    for (const segment of segments) next.push(...segment.split(`{${placeholder}}`));
    segments = next;
  }
  return segments.map((segment) => interpolate(segment, values));
}

/**
 * 切换界面语言。
 *
 * 写 localStorage 会触发 `StorageEvent`，从而被 `locale-sync.ts` 转发到另一个窗口
 * （小窗），所以这里**只**管本窗口的状态与 DOM。
 */
export function setLocale(next: Locale): void {
  if (!isLocale(next) || next === locale.value) return;
  locale.value = next;
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(LOCALE_STORAGE_KEY, next);
    } catch {
      /* 隐私模式忽略 */
    }
  }
  applyLocaleToDocument();
}

/** 把语言落到 DOM（`<html lang>` 与窗口标题）；幂等 */
export function applyLocaleToDocument(): void {
  if (typeof document === 'undefined') return;
  document.documentElement.lang = LOCALE_HTML_LANG[locale.value] ?? LOCALE_HTML_LANG[DEFAULT_LOCALE];
  document.title = t('app.title');
}

/**
 * 采用后端设置里的语言（权威值）。
 *
 * 与主题的处理方式一致：localStorage 只是首屏的快速通道，真正说了算的是设置文件；
 * 两者不一致时（例如换了机器、清了 storage）以后端为准。
 */
export function adoptLocaleFromSettings(raw: string | null | undefined): void {
  if (!isLocale(raw)) return;
  if (raw === locale.value) {
    applyLocaleToDocument();
    return;
  }
  setLocale(raw);
}

/** 设置页用的语言列表（含显示名） */
export function availableLocales(): { value: Locale; label: string }[] {
  return LOCALES.map((value) => ({ value, label: LOCALE_LABELS[value] }));
}

// ---------------------------------------------------------------------------
// 分组标签：组号 → 稳定标识 → 本地化文案
// ---------------------------------------------------------------------------

/**
 * 七组的分组标签（按组号 0..6 排列）。
 *
 * 优先用**本地化文案**（`tier.<key>`，key 来自产物自带的 `tier_keys`）；
 * 当前语言没翻这一条时回落到产物自带的中文组名，最后才用 `tier-colors.ts` 的兜底常量。
 *
 * 这是「组名只是文案」的落点：接入新语言时**不需要改任何页面**，只要消息表里有
 * `tier.*` 七条，全站的分组标签一起变。
 *
 * `metaLike` 用结构类型而不是 `Meta`，好让本模块保持不依赖 `$lib/types`。
 */
export function tierLabels(metaLike?: TierNamesHolder): string[] {
  const keys = metaLike?.tier_keys ?? [];
  const names = metaLike?.tier_names ?? [];
  const count = Math.max(keys.length, names.length, 7);
  const out: string[] = [];
  for (let index = 0; index < count; index += 1) {
    const key = keys[index];
    if (key) {
      const messageKey = `tier.${key}` as MessageKey;
      if (hasMessage(messageKey)) {
        out.push(t(messageKey));
        continue;
      }
    }
    out.push(names[index] ?? `tier[${index}]`);
  }
  return out;
}

/**
 * 单个组号的标签；没有对应分组时给「未收录」。
 *
 * `tierIndex` 为 null 表示语料库未收录（与「极少」不同，那一组是有排名的真实分组）。
 */
export function tierLabel(tierIndex: number | null | undefined, metaLike?: TierNamesHolder): string {
  if (tierIndex === null || tierIndex === undefined || !Number.isInteger(tierIndex) || tierIndex < 0) {
    return t('tier.unknown');
  }
  return tierLabels(metaLike)[tierIndex] ?? t('tier.unknown');
}

/** 只要求这两个可选字段，避免本模块依赖 `$lib/types` */
export type TierNamesHolder =
  | { tier_names?: string[] | null; tier_keys?: string[] | null }
  | null
  | undefined;
