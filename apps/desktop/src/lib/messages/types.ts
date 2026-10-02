import type { zhCN } from './zh-CN';

/**
 * 文案 key —— 由**源语言**（`zh-CN.ts`）的 key 集合推导。
 *
 * 新增文案只改那一个文件；在代码里写了一个不存在的 key，`pnpm check` 会直接报错。
 */
export type MessageKey = keyof typeof zhCN;

/**
 * 一份**完整**翻译必须覆盖的 key 集合。
 *
 * 新增语言时写成 `const en: Messages = { ... }`：漏 key、写错 key、多写 key 全都是
 * 编译错误。这就是「漏翻检测器」，不用另外维护脚本或对着两份文件数数。
 *
 * 刻意用 `Record` 而不是 `Partial`：允许漏翻的话，漏掉的那条会在界面上静默回落成中文，
 * 而这种问题只在用户那里才被发现。
 */
export type Messages = Record<MessageKey, string>;
