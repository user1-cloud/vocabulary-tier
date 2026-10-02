/**
 * 数字本地化 —— 唯一来源。
 *
 * ## 为什么单独一个无依赖的小模块
 *
 * `tier-colors.ts` 必须保持**叶子纯模块**：它会被 Node 工具直接 import
 * （`.tmp/pw/dump-tier-colors.cjs`），一旦间接依赖 `i18n.svelte.ts`（里面有 `$state`
 * 符文）或 `format.ts`（会拉起 i18n），Node 侧就崩了。而 `format.ts` / `segments.ts` /
 * `tier-colors.ts` / 表管理页都要用同一个数字 locale，于是抽到这里 —— 零依赖，谁都能 import。
 *
 * ## 什么时候需要改
 *
 * `zh-CN` 与 `en` 的千分位、小数点完全一致，所以现在钉死一个常量没有任何可见差异。
 * 接入 `de-DE`（`1.000,5`）或 `ar-EG`（阿拉伯数字）这类语言时才必须动：
 *   - 要么改这个常量（但那会让所有语言共用一种分隔符，通常不对）；
 *   - 要么把它换成读 `i18n.svelte.ts::activeLocale()`，同时把 `tier-colors.ts` 的
 *     数字格式化改成**由调用方传入 locale**（保持它不依赖 i18n）。
 */
export const NUMBER_LOCALE = 'zh-CN';

/** 千分位整数（哨兵/空值判断留给调用方，这里只管格式化） */
export function formatCount(n: number): string {
  return Math.round(n).toLocaleString(NUMBER_LOCALE);
}
