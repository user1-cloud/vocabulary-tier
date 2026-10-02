/**
 * token 序列的辅助判断（纯函数）。
 *
 * 划句分析 / 悬浮小窗 / 排行榜都要判断「这个 token 是不是标点」「是不是单个汉字」，
 * 归一化在 bridge.ts 里做过一次（以 Rust 返回的 accepted / single_cjk 为准），
 * 但浏览器 mock 与手输场景仍需要本地兜底，所以集中在这里。
 *
 * 注意：本文件无 runes。分组「身份」一律用组号，不用组名 —— 组名是文案，
 * 会随界面语言变化。`tableLabel` / `formatDomainRanks` 是**展示层**，会读 `t()`；
 * 其余（`normalizeToken` / `summarizeTokens` / `isCjkChar` …）保持纯函数。
 */

import type { TokenInfo } from './types';
import { t } from './i18n.svelte';
import { formatCount } from './number-locale';

/** 判断是否是 CJK 统一表意文字（含扩展 A 与基本区） */
export function isCjkChar(ch: string): boolean {
  if (!ch) return false;
  const code = ch.codePointAt(0) ?? 0;
  return (
    (code >= 0x4e00 && code <= 0x9fff) || // CJK 基本区
    (code >= 0x3400 && code <= 0x4dbf) || // 扩展 A
    (code >= 0xf900 && code <= 0xfaff) || // 兼容表意文字
    (code >= 0x20000 && code <= 0x2a6df) // 扩展 B
  );
}

/** 整串都是 CJK（长度按码点算），且只有 1 个字符 */
export function isSingleCjk(text: string): boolean {
  const chars = [...text];
  return chars.length === 1 && isCjkChar(chars[0]);
}

/** 标点 / 空白 / 纯符号 —— 不参与统计 */
export function isPunctuationOrSpace(text: string): boolean {
  if (!text) return true;
  // 去掉所有标点与空白后还有内容，说明是内容 token
  return !/[\p{L}\p{N}\p{Sc}]/u.test(text);
}

/** 归一化一个来自 Rust 的 token：字段缺失时按内容推断 */
export function normalizeToken(raw: Partial<TokenInfo> & { text: string }): TokenInfo {
  const text = raw.text ?? '';
  return {
    text,
    byte_start: raw.byte_start ?? 0,
    byte_end: raw.byte_end ?? 0,
    accepted: raw.accepted ?? !isPunctuationOrSpace(text),
    single_cjk: raw.single_cjk ?? isSingleCjk(text),
    table: raw.table ?? '',
    count: raw.count ?? null,
    rank: raw.rank ?? null,
    pct: raw.pct ?? null,
    top_pct: raw.top_pct ?? null,
    entries: raw.entries ?? null,
    tier: raw.tier ?? null,
    tier_name: raw.tier_name ?? null,
    in_dict: raw.in_dict ?? null,
    from_user: raw.from_user ?? null,
    table_ranks: raw.table_ranks ?? [],
  };
}

/** 摘要卡片用的 token 分类统计结果（由 `summarizeTokens` 产出） */
export type TokenSummary = {
  total: number;
  /** 计入统计的 token 数 */
  accepted: number;
  /** 标点/空白 token 数 */
  skipped: number;
  /** 语料库未收录的内容 token 数（去重） */
  unknownUnique: number;
  /** 语料库未收录的内容 token 出现次数 */
  unknownTotal: number;
  /**
   * 各分组命中的 token 次数，**key = 组号 0..6**（不是组名）。
   *
   * 用组号而不是组名：组名是文案，会随界面语言变化；组号才是身份。
   */
  byTier: Map<number, number>;
};

/**
 * 统计 token 分类结果（供摘要卡片使用）。
 *
 * `tierIndexOf` 用于「分组自定义」：不传时按后端给的组号 `token.tier` 统计
 * （用户没动过分组设置时的默认行为），传了回调就按生效阈值重算组号。
 */
export function summarizeTokens(
  tokens: TokenInfo[],
  tierIndexOf?: (token: TokenInfo) => number | null
): TokenSummary {
  const byTier = new Map<number, number>();
  const unknownSet = new Set<string>();
  let accepted = 0;
  let unknownTotal = 0;

  for (const token of tokens) {
    if (!token.accepted) continue;
    accepted += 1;
    const index = tierIndexOf ? tierIndexOf(token) : token.tier;
    if (index !== null && index !== undefined && Number.isInteger(index) && index >= 0) {
      byTier.set(index, (byTier.get(index) ?? 0) + 1);
    } else {
      unknownTotal += 1;
      unknownSet.add(token.text);
    }
  }

  return {
    total: tokens.length,
    accepted,
    skipped: tokens.length - accepted,
    unknownUnique: unknownSet.size,
    unknownTotal,
    byTier,
  };
}

/** 分域排名的展示串：`综合 #123 · 文学 未收录` */
export function formatDomainRanks(domainRanks: [string, number | null][]): string {
  if (domainRanks.length === 0) return '—';
  return domainRanks
    .map(([name, rank]) => `${name} ${rank === null ? t('tier.unknown') : `#${formatCount(rank)}`}`)
    .join(' · ');
}

/** token 的表名标签（展示层，随界面语言变） */
export function tableLabel(table: string): string {
  if (table === 'word') return t('table.word');
  if (table === 'char') return t('table.char');
  return t('table.none');
}
