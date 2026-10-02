/**
 * 七组色阶 —— 唯一权威定义。
 *
 * ## 分组的「身份」是稳定标识（key），不是中文组名
 *
 * 身份 = `TIER_KEYS` 里的 ASCII 标识（`very_common` … `very_rare`），与
 * `crates/vocfreq-core/src/rank.rs::TIER_KEYS` **同序同值**。组名是**文案**
 * （本地化在 `i18n.svelte.ts::tierLabels()`，本模块刻意不依赖它），
 * 任何判断、索引、配色都不许读组名。
 *
 * 为什么必须这样：组名会随界面语言变化（i18n），而配色与统计口径不能跟着变。
 * 早期版本用 `Map<组名, 调色板>` 按中文名查色，一旦引入多语言就会整片串色。
 *
 * ## 组号 → key → 调色板
 *
 * 后端给的组号（`token.tier`、`tiers[i]`、`tier_stats[i]` 的下标）先经 `tierKeyAt()`
 * 换成 key，再由 key 查色（`paletteForTierKey()`）。key 优先取产物自带的
 * `meta.tier_keys`，缺省回落到本文件的 `TIER_KEYS`。于是：
 *   - 产物调整了分组顺序 → 组号映射到新的 key，配色依然正确；
 *   - 产物出现前端不认识的 key（新增档位）→ 退化成中性灰，而不是静默套错颜色。
 *
 * 颜色可以硬编码（需求明确允许），但**分组边界绝对不能硬编码**：换语料库后阈值会变，
 * 所以分组边界一律由 `format.ts::effectiveBounds` 解算、`meta.tables[*].tiers` 兜底。
 *
 * 本文件是纯函数模块（无 runes、不依赖 `Meta` 类型、不依赖 `i18n.svelte.ts`），
 * 当前主题由调用方传入。数字本地化走零依赖的 `./number-locale`，见那里的说明。
 */
import { formatCount } from './number-locale';

export type TierPalette = {
  /** 浅色模式前景色 */
  lightFg: string;
  /** 浅色模式背景色 */
  lightBg: string;
  /** 深色模式前景色 */
  darkFg: string;
  /** 深色模式背景色 */
  darkBg: string;
};

export type TokenColors = {
  fg: string;
  bg: string;
  /** 未收录词用虚线下划线区分 */
  dashed: boolean;
};

/** 未收录（语料库没有这个词）——浅色 #2d323c、深色 #d4dced，背景透明 + 虚线下划线 */
export const UNKNOWN_PALETTE: TierPalette = {
  lightFg: '#2d323c',
  lightBg: 'transparent',
  darkFg: '#d4dced',
  darkBg: 'transparent',
};

/**
 * 七组的稳定标识，与 `crates/vocfreq-core/src/rank.rs::TIER_KEYS` **同序同值**。
 *
 * ⚠️ 顺序即组号（0 → `very_common`）。改这里必须同步改 Rust 侧，否则老产物会串色/变灰。
 */
export const TIER_KEYS = [
  'very_common',
  'common',
  'fairly_common',
  'medium',
  'fairly_rare',
  'rare',
  'very_rare',
] as const;

export type TierKey = (typeof TIER_KEYS)[number];

/*
 * 七组色阶（2026 版重做：游戏稀有度色阶）：
 *
 *   语义**不是**「暖=常见、冷=罕见」的色温渐变，而是玩家一眼就懂的那套
 *   稀有度配色——越罕见，颜色越像游戏里的高稀有度物品：
 *
 *     极多 → 普通（灰白）   很多 → 优秀（绿）   较多 → 精良（蓝）
 *     中等 → 史诗（紫）     较少 → 传说（橙）   很少 → 神话（红）
 *     极少 → 神器（金）
 *
 *   即通行的「白 → 绿 → 蓝 → 紫 → 橙 → 红 → 金」七级稀有度表。
 *   最常见的词给最素的灰白，最罕见的词给最扎眼的金色，稀有度单调递增。
 *
 *   - 浅色模式 = 「淡色底 + 深色文字」，对底色对比度 ≥ 4.5（WCAG AA 正文）；
 *   - 深色模式 = 「暗色淡底 + 亮色文字」，对底色对比度 ≥ 6（AA 且余量充足）；
 *   - 橙 / 红 / 金 三档是相邻暖色，靠色相 + 明度拉开：橙偏深、红最正、金最亮，
 *     图例里并排仍能区分；若觉得糊，优先动这三档的明度。
 *
 * 改这里的颜色时请顺手核一下对比度（底色 / 卡片底），别低于 3:1。
 *
 * 用 `Record<TierKey, …>` 而不是数组：漏配/写错 key 会被 TypeScript 直接拦住，
 * 也天然免疫「数组顺序与 Rust 不一致」这种静默错位。
 */
const TIER_PALETTES: Record<TierKey, TierPalette> = {
  // 极多 · 普通 · 灰白
  very_common: { lightFg: '#475467', lightBg: '#f2f4f7', darkFg: '#cbd5e1', darkBg: '#2c323b' },
  // 很多 · 优秀 · 绿
  common: { lightFg: '#137a3a', lightBg: '#e6f7ea', darkFg: '#6ee7a0', darkBg: '#173a24' },
  // 较多 · 精良 · 蓝
  fairly_common: { lightFg: '#1c4bc4', lightBg: '#e4efff', darkFg: '#7cc4ff', darkBg: '#14304d' },
  // 中等 · 史诗 · 紫
  medium: { lightFg: '#6b28c9', lightBg: '#f3ecff', darkFg: '#c8a8ff', darkBg: '#2f2350' },
  // 较少 · 传说 · 橙
  fairly_rare: { lightFg: '#b03c08', lightBg: '#ffeeda', darkFg: '#ffb066', darkBg: '#46280f' },
  // 很少 · 神话 · 红
  rare: { lightFg: '#b91c1c', lightBg: '#ffe6e3', darkFg: '#ff8f85', darkBg: '#4a2320' },
  // 极少 · 神器 · 金
  very_rare: { lightFg: '#7a5c00', lightBg: '#fff3c4', darkFg: '#ffd75e', darkBg: '#4a3a0d' },
};

/** 是不是七组里认识的标识（未知 key = 产物比前端新，或 key 被改名） */
export function isKnownTierKey(key: string | null | undefined): key is TierKey {
  return typeof key === 'string' && key in TIER_PALETTES;
}

/** 稳定标识 → 调色板（未知 / 空 → 「未收录」中性色） */
export function paletteForTierKey(key: string | null | undefined): TierPalette {
  return isKnownTierKey(key) ? TIER_PALETTES[key] : UNKNOWN_PALETTE;
}

/** 稳定标识 → 该主题下最终写进 style 的 fg / bg */
export function colorsForTierKey(key: string | null | undefined, dark: boolean): TokenColors {
  const palette = paletteForTierKey(key);
  return {
    fg: dark ? palette.darkFg : palette.lightFg,
    bg: dark ? palette.darkBg : palette.lightBg,
    // 未收录 = 没有 key，或 key 不在七组里（例如产物新增了前端还不认识的档位）
    dashed: !isKnownTierKey(key),
  };
}

/** 只要求可选的 `tier_keys`，好让本模块不必依赖 `types.ts` 的 `Meta` */
export type TierKeysHolder = { tier_keys?: string[] | null } | null | undefined;function sameKeys(keys: readonly string[]): boolean {
  return keys.length === TIER_KEYS.length && keys.every((key, index) => key === TIER_KEYS[index]);
}

/** 同一份产物只报一次，避免每次重算都刷屏 */
const warnedTierKeys = new Set<string>();

/**
 * 取产物自带的七组标识；老产物没有这个字段（`#[serde(default)]` → 空数组）时
 * 回落到本文件的 `TIER_KEYS`。
 *
 * 与前端常量不一致时**每个不同的 key 列表只报一次错**：顺序变了不会串色（映射走 key
 * 而不是位置），取值变了只会让那几组退化成中性灰 —— 都是安全失败，所以这里只做诊断、
 * 不做拦截。刻意不用 `import.meta.env.DEV` 判定：本模块还会被 Node 工具直接 import
 * （例如 `dump-tier-colors.cjs`），那种环境里 `import.meta.env` 是 undefined。
 */
export function tierKeysFrom(meta: TierKeysHolder): readonly string[] {
  const keys = meta?.tier_keys;
  if (!keys || keys.length === 0) return TIER_KEYS;
  if (!sameKeys(keys)) {
    const signature = keys.join(',');
    if (!warnedTierKeys.has(signature)) {
      warnedTierKeys.add(signature);
      console.error(
        '[tier-colors] 产物里的 tier_keys 与前端 TIER_KEYS 不一致：未知档位会退化成中性灰。\n' +
          `  产物：${keys.join(', ')}\n` +
          `  前端：${TIER_KEYS.join(', ')}\n` +
          '  请同步 crates/vocfreq-core/src/rank.rs::TIER_KEYS 与本文件。'
      );
    }
  }
  return keys;
}

/**
 * 组号（0..6）→ 稳定标识。越界 / 非法 / 该档位不存在 → null。
 *
 * `keys` 传产物自带的 `tier_keys`（用 `tierKeysFrom` 取）；不传时用前端常量。
 */
export function tierKeyAt(
  index: number | null | undefined,
  keys?: readonly string[] | null
): string | null {
  if (index === null || index === undefined || !Number.isInteger(index) || index < 0) return null;
  const list = keys && keys.length > 0 ? keys : TIER_KEYS;
  return list[index] ?? null;
}

/** token 的内联样式（背景透明时不写 background-color，避免覆盖父级） */
export function tokenStyle(colors: TokenColors): string {
  const parts = [`color:${colors.fg}`];
  if (colors.bg !== 'transparent') parts.push(`background-color:${colors.bg}`);
  if (colors.dashed) parts.push('text-decoration:underline dashed 1px');
  parts.push('text-underline-offset:3px');
  return parts.join(';');
}

/** 图例色块样式 */
export function swatchStyle(palette: TierPalette, dark: boolean): string {
  const fg = dark ? palette.darkFg : palette.lightFg;
  const bg = dark ? palette.darkBg : palette.lightBg;
  return `background-color:${bg};color:${fg};border-color:${fg}`;
}

/**
 * 排行阈值的人类可读写法。
 *   最后一组（max_rank = u64::MAX）显示成「> 上一组上界」。
 */
export function tierRangeLabel(index: number, tiers: { name: string; max_rank: number }[]): string {
  const tier = tiers[index];
  if (!tier) return '';
  const isLast = index === tiers.length - 1;
  const prev = index > 0 ? tiers[index - 1].max_rank : 0;
  if (isLast || !Number.isFinite(tier.max_rank) || tier.max_rank >= 1e18) {
    return `> ${formatCount(prev)}`;
  }
  return `≤ ${formatCount(tier.max_rank)}`;
}

/**
 * 同一套阈值文案，但直接吃「6 个上界」数组（分组自定义后的生效阈值）。
 *
 * 第 7 组（index 6，没有上界）显示成「> 第 6 组上界」。
 */
export function tierRangeLabelOfBounds(index: number, bounds: number[]): string {
  if (index < 0 || index >= bounds.length + 1) return '';
  if (index >= bounds.length) {
    const prev = bounds[bounds.length - 1] ?? 0;
    return `> ${formatCount(prev)}`;
  }
  return `≤ ${formatCount(bounds[index])}`;
}

/** flags 位定义（与 crates/vocfreq-core/src/tokenize.rs 一致） */
export const FLAG_IN_DICT = 1;
export const FLAG_FROM_USER = 2;

export function inDictFromFlags(flags: number): boolean {
  return (flags & FLAG_IN_DICT) !== 0;
}

export function fromUserFromFlags(flags: number): boolean {
  return (flags & FLAG_FROM_USER) !== 0;
}
