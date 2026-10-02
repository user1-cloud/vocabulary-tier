/**
 * 「拿某张表重跑一次统计」的**一次性交接单**。
 *
 * 需求来源：词库外置之后，表与词库用内容指纹绑定。词库被改过（`drifted`）或删掉
 * （`missing`）时，那张表的频次就跟分词口径对不上了 —— 界面必须给**一键重新统计**。
 *
 * 为什么用一个模块级的一次性变量，而不是页面之间互相 import 或共享路由状态：
 *   - 页面组件之间刻意不互相 import（见 navigation.ts 的说明）；
 *   - 扫描表单的状态全在 `WordFreqPage` 自己的 `$state` 里，别处不该去改它；
 *   - `App.svelte` 在跳转同一个路由时会 bump `routeEpoch` **强制重新挂载**目标组件，
 *     所以「挂载时取一次」的语义是可靠的（这也是 `initialText` 那套的做法）。
 *
 * 用 `$state` 而不是普通对象：写与读都在组件生命周期里，保持响应式最省心
 * （读的那一方在 `$effect` / `$derived` 里读）。
 */
import type { ScanParams } from './types';

/** 重跑统计时能预填的那部分 `ScanParams`（其余字段沿用设置里的默认值） */
export type ScanPrefill = Partial<
  Pick<
    ScanParams,
    | 'corpus'
    | 'out'
    | 'dictFiles'
    | 'threads'
    | 'hmm'
    | 'minCount'
    | 'keepDigit'
    | 'keepLatin'
    | 'skipSingleChar'
    | 'onlyDomains'
    | 'skipDomainTables'
    | 'writeTsv'
  >
> & {
  /** 预填输出的表名（扫描页据此再问一次 `suggest_table_dir`） */
  tableName?: string;
};

export const scanPrefill = $state<{ value: ScanPrefill | null }>({ value: null });

/** 放下一张交接单（下一次 `WordFreqPage` 挂载时消费） */
export function requestScanPrefill(prefill: ScanPrefill): void {
  scanPrefill.value = prefill;
}

/** 取走交接单（消费一次即清空，避免下次进页面又被它覆盖） */
export function takeScanPrefill(): ScanPrefill | null {
  const value = scanPrefill.value;
  scanPrefill.value = null;
  return value;
}
