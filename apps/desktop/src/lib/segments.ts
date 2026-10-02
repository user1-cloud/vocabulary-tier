/**
 * token 序列的辅助判断（纯函数）。
 *
 * 划句分析 / 悬浮小窗 / 排行榜都要判断「这个 token 是不是标点」「是不是单个汉字」，
 * 归一化在 bridge.ts 里做过一次（以 Rust 返回的 accepted / single_cjk 为准），
 * 但浏览器 mock 与手输场景仍需要本地兜底，所以集中在这里。
 */

import type { TokenInfo } from './types';

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
    tier: raw.tier ?? null,
    tier_name: raw.tier_name ?? null,
    in_dict: raw.in_dict ?? null,
    from_user: raw.from_user ?? null,
    domain_ranks: raw.domain_ranks ?? [],
  };
}

/**
 * 统计 token 分类结果（供摘要卡片使用）。
 *
 * `tierNameOf` 用于「分组自定义」：不传时按后端给的 `token.tier_name` 统计
 * （用户没动过分组设置时的默认行为），传了回调就按生效阈值统计。
 */
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
  /** 各分组命中的 token 次数，key = 组名 */
  byTier: Map<string, number>;
};

export function summarizeTokens(
  tokens: TokenInfo[],
  tierNameOf?: (token: TokenInfo) => string | null
): TokenSummary {
  const byTier = new Map<string, number>();
  const unknownSet = new Set<string>();
  let accepted = 0;
  let unknownTotal = 0;

  for (const token of tokens) {
    if (!token.accepted) continue;
    accepted += 1;
    const name = tierNameOf ? tierNameOf(token) : token.tier_name;
    if (name) {
      byTier.set(name, (byTier.get(name) ?? 0) + 1);
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
    .map(([name, rank]) => `${name} ${rank === null ? '未收录' : `#${rank.toLocaleString('zh-CN')}`}`)
    .join(' · ');
}

/** token 的表名中文标签 */
export function tableLabel(table: string): string {
  if (table === 'word') return '词表';
  if (table === 'char') return '字表';
  return '未查表';
}
