/**
 * 图标公共 props 类型。
 *
 * 文件名叫 icon-types.ts 而不是 types.ts，是为了避免与图标目录下的
 * 汇总入口 `index.ts` 在模块解析时产生歧义。
 */

export type IconProps = {
  /** 边长（px），默认 18 */
  size?: number;
  /** 追加的 class，一般用来控制颜色/尺寸 */
  class?: string;
  /** 线宽，默认 1.75（比 lucide 默认的 2 更细腻，更贴合桌面 UI） */
  strokeWidth?: number;
};
