/**
 * 七组色阶 —— 唯一权威定义。
 *
 * 严格按需求表实现：每个分组一套「浅色 fg/bg」+「深色 fg/bg」。
 * 颜色可以硬编码（需求明确允许），但**分组边界绝对不能硬编码**：
 * 换语料库后阈值会变，所以分组靠 `meta.tables[*].tiers` 里的
 * `{ name, max_rank }` 来判断，本文件只负责「组名 → 颜色」的映射。
 *
 * 为什么用 Map 按组名索引而不是按下标：
 *   下标依赖 tier_names 的顺序，而组名是语义本身。用组名索引后，
 *   即使后端调整分组数量 / 顺序，配色依然正确；未知组名回落到中性灰。
 *
 * 本文件是纯函数模块（无 runes），当前主题由调用方传入。
 */

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

/*
 * 七组色阶（2024 版重做）：
 *
 *   - 浅色模式 = 「淡色底 + 深色文字」，对底色对比度 ≥ 4.5（WCAG AA 正文）；
 *   - 深色模式 = 「暗色淡底 + 亮色文字」，对底色对比度 ≈ 3.2~4.0（徽标级别，AA Large）；
 *   - 七组色相：红 25° → 橙 60° → 黄 90° → 绿 150° → 蓝 258° → 紫 295° → 中性 265°，
 *     相邻两组色相差 ≥ 30°，图例里一眼能分开；
 *   - 彩度比上一版低一半左右：底色不再是一块「高饱和色卡」，浅色模式下尤其明显。
 *
 * 改这里的颜色时请顺手核一下对比度（底色 / 卡片底），别低于 3:1。
 */
export const TIER_PALETTE: Map<string, TierPalette> = new Map<string, TierPalette>([
  ['极多', { lightFg: '#940c19', lightBg: '#ffe8e6', darkFg: '#ffafa6', darkBg: '#502825' }],
  ['很多', { lightFg: '#611e00', lightBg: '#ffecd7', darkFg: '#ffc588', darkBg: '#482f1a' }],
  ['较多', { lightFg: '#402900', lightBg: '#fff3ce', darkFg: '#f4d580', darkBg: '#41371b' }],
  ['中等', { lightFg: '#003b07', lightBg: '#e0f7e4', darkFg: '#a2ecb1', darkBg: '#213926' }],
  ['较少', { lightFg: '#002e7d', lightBg: '#e3f0ff', darkFg: '#a1daff', darkBg: '#213654' }],
  ['很少', { lightFg: '#411b7b', lightBg: '#f1edff', darkFg: '#dec4ff', darkBg: '#372f4f' }],
  ['极少', { lightFg: '#252a34', lightBg: '#ebedf1', darkFg: '#d1d8e5', darkBg: '#383b40' }],
]);

/** 图例顺序：以 meta.tier_names 为准，缺省时用这份标准顺序兜底 */
export const DEFAULT_TIER_NAMES: string[] = [
  '极多',
  '很多',
  '较多',
  '中等',
  '较少',
  '很少',
  '极少',
];

/** 组名 → 调色板（未知组名回落到「未收录」中性色） */
export function paletteForTierName(name: string | null | undefined): TierPalette {
  if (!name) return UNKNOWN_PALETTE;
  return TIER_PALETTE.get(name) ?? UNKNOWN_PALETTE;
}

/**
 * 解析最终要写进 style 的 fg / bg。
 *
 * `dark` 由调用方传入（组件里用 `isDark()` 派生），这样本文件保持纯函数、
 * 不持有响应式状态，测试与复用都更简单。
 */
export function colorsForTierName(name: string | null | undefined, dark: boolean): TokenColors {
  const palette = paletteForTierName(name);
  return {
    fg: dark ? palette.darkFg : palette.lightFg,
    bg: dark ? palette.darkBg : palette.lightBg,
    // 未收录 = 没有对应调色板（或组名不在七组里）
    dashed: !name || !TIER_PALETTE.has(name),
  };
}

/** 生成 token 的内联样式（背景透明时不写 background-color，避免覆盖父级） */
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
    return `> ${prev.toLocaleString('zh-CN')}`;
  }
  return `≤ ${tier.max_rank.toLocaleString('zh-CN')}`;
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
    return `> ${prev.toLocaleString('zh-CN')}`;
  }
  return `≤ ${bounds[index].toLocaleString('zh-CN')}`;
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
