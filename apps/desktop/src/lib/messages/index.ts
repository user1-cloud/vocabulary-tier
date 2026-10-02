import { zhCN } from './zh-CN';
import { en } from './en';
import type { Messages } from './types';

export type { MessageKey, Messages } from './types';
export { zhCN, en };

/**
 * 已注册的界面语言，顺序即设置页里的显示顺序。
 *
 * ## 新增一种语言只需要两步（英文由豆包补）
 *
 *   1. 新建 `./en.ts`：`export const en: Messages = { ... }` —— 漏 key 会**编译失败**；
 *   2. 这里加上 `'en'`，并在 `MESSAGES` / `LOCALE_LABELS` 各加一行。
 *
 * 其它地方（设置页下拉、首屏 `<html lang>`、小窗跨窗口同步、分组标签）全部从这个
 * 注册表读，不需要改。
 *
 * 只有 `zh-CN` 时设置页就只显示一个选项 —— 这是**刻意的**：宁可不显示，
 * 也不给出一个点了没反应的「English」。
 */
export const LOCALES = ['zh-CN', 'en'] as const;

export type Locale = (typeof LOCALES)[number];

/** 源语言：任何语言缺 key 时都回落到它 */
export const DEFAULT_LOCALE: Locale = 'zh-CN';

/**
 * 语言 → 文案表。
 *
 * 类型是 `Record<Locale, Messages>`：注册了语言却没给完整翻译，编译不过。
 */
export const MESSAGES: Record<Locale, Messages> = {
  'zh-CN': zhCN,
  en,
};

/**
 * 语言在设置页里的显示名。
 *
 * 刻意**不放进消息表**：语言名按惯例用它自己的语言书写（English 永远写 English，
 * 切到日文界面也还是「简体中文」），所以它不是需要翻译的文案，而是语言自身的属性。
 */
export const LOCALE_LABELS: Record<Locale, string> = {
  'zh-CN': '简体中文',
  en: 'English',
};

/** `<html lang>` 用的 BCP 47 标签；与 i18n 的 key 恰好同形，但语义不同，单独列一份防漂移 */
export const LOCALE_HTML_LANG: Record<Locale, string> = {
  'zh-CN': 'zh-CN',
  en: 'en',
};

export function isLocale(value: unknown): value is Locale {
  return typeof value === 'string' && (LOCALES as readonly string[]).includes(value);
}
