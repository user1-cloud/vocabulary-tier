/**
 * 极简类型化事件总线。
 *
 * 目前没有任何业务逻辑使用它，属于「预留挂载点」：等 Rust 侧的命令 /
 * 长任务进度事件接进来时，可以把 tauri 的 listen() 桥接到这里，
 * 让各个页面在不互相 import 的前提下订阅同一份进度状态。
 *
 *   // 未来在 src/lib/api/bridge.ts 里：
 *   import { listen } from '@tauri-apps/api/event';
 *   await listen<Progress>('vocfreq://progress', (e) => emit('progress', e.payload));
 *
 *   // 页面里：
 *   const off = on('progress', (p) => { progress = p; });
 *   $effect(() => off); // 组件卸载时取消订阅
 */

export type EventMap = {
  /** 词频统计进度（为 vocfreq-core 的长任务预留） */
  progress: { task: string; done: number; total: number; message?: string };
  /** 全局错误提示 */
  error: { message: string; detail?: string };
  /** 主题变化 */
  'theme-changed': { theme: 'light' | 'dark' | 'system' };
};

type Handler<K extends keyof EventMap> = (payload: EventMap[K]) => void;

const listeners = new Map<keyof EventMap, Set<(payload: never) => void>>();

/** 订阅事件，返回取消订阅函数 */
export function on<K extends keyof EventMap>(type: K, handler: Handler<K>): () => void {
  let set = listeners.get(type);
  if (!set) {
    set = new Set();
    listeners.set(type, set);
  }
  const raw = handler as (payload: never) => void;
  set.add(raw);
  return () => {
    set.delete(raw);
  };
}

/** 派发事件 */
export function emit<K extends keyof EventMap>(type: K, payload: EventMap[K]): void {
  const set = listeners.get(type);
  if (!set) return;
  for (const handler of set) {
    (handler as Handler<K>)(payload);
  }
}
