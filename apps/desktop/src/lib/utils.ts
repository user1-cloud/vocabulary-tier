import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

/**
 * shadcn-svelte 约定的 className 合并工具。
 *
 * - clsx 负责条件类名拼接
 * - tailwind-merge 负责消解 Tailwind 冲突（后写的赢）
 *
 * 用法：
 *   <div class={cn('rounded-lg border p-4', className)}>
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

/** 生成随机 id，供 label / aria-describedby 关联使用 */
export function uid(prefix = 'vt'): string {
  return `${prefix}-${Math.random().toString(36).slice(2, 9)}`;
}
