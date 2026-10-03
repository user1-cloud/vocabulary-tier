# VocTier 桌面端（`apps/desktop`）

VocTier 中文字词频率分析工具的桌面客户端前端。

- **前端**：Svelte 5（runes 语法）+ TypeScript + Vite 7
- **样式**：Tailwind CSS v4（CSS 优先配置，**没有** `tailwind.config.js`）
- **组件**：shadcn-svelte 风格（手写封装，底层用 [bits-ui](https://bits-ui.com/)）
- **壳**：Tauri v2（`src-tauri/`，一个**独立于上层 Cargo workspace** 的 Rust 工程）
- **界面语言**：中文（i18n 内核已就绪，`zh-CN` 为源语言）

> 当前状态：**业务功能已实现**——划句分词着色、排行榜、词典/表管理、设置、
> 全局热键取词与悬浮小窗均已可用。前端统一经 `src/lib/api/bridge.ts` 调用 Rust
> 侧命令（命令清单见 `docs/DESIGN.md` §8.1）。

---

## 1. 环境要求

| 依赖 | 版本 | 说明 |
| --- | --- | --- |
| Node.js | ≥ 20（本项目在 v24.15.0 上验证） | |
| pnpm | ≥ 10（本项目在 12.3.4 上验证） | |
| Rust | ≥ 1.77.2 | Tauri v2 的最低要求 |
| WebView2 | Windows 10/11 自带 | |

---

## 2. 安装依赖

```powershell
cd <仓库根>\apps\desktop      # 例如 D:\work\voctier\apps\desktop
pnpm install
```

`pnpm install` 会自动读取同目录下的 `pnpm-workspace.yaml`。**这个文件是必需的**：
pnpm 10+ 默认拦截依赖的 `postinstall` 脚本，esbuild 需要它来准备原生二进制。
文件里已经写好了：

```yaml
allowBuilds:
  esbuild: true
```

等价于非交互地执行 `pnpm approve-builds`。若删掉它，`pnpm install` 会以
`ERR_PNPM_IGNORED_BUILDS` 失败，`vite build` 也会报找不到 esbuild。

---

## 3. 开发

### 3.1 只跑前端（推荐先在浏览器里调 UI）

```powershell
pnpm dev            # http://localhost:1420
```

端口固定 1420（与 `src-tauri/tauri.conf.json` 的 `devUrl` 一致，`strictPort: true`）。
浏览器里 `isTauri()` 返回 `false`，所有 Tauri 调用会走 `src/lib/api/bridge.ts` 里的
占位实现 —— 也就是说**UI 可以完全不依赖 Rust 侧独立调试**。

### 3.2 跑桌面应用

```powershell
pnpm tauri:dev      # 会开窗口，并且会一直挂着
```

> CI / 无头环境请不要执行这条命令，它不会自己退出。

### 3.3 类型检查

```powershell
pnpm check          # svelte-check，当前 0 errors 0 warnings
```

---

## 4. 构建

### 4.1 纯前端构建（不需要 Rust，几秒钟）

```powershell
pnpm build          # 等价于 vite build → 产物在 dist/
```

### 4.2 Rust 侧检查

```powershell
cd src-tauri
cargo check
```

### 4.3 打包桌面安装包

```powershell
pnpm tauri:build
```

会先跑 `beforeBuildCommand`（`pnpm build`）再编译 Rust 并打包。
产物在 `src-tauri/target/release/bundle/`。

---

## 5. 目录结构

```
apps/desktop/
├─ index.html                  # Vite 入口；内含「首屏防闪烁」主题脚本
├─ package.json
├─ pnpm-workspace.yaml         # pnpm 12 配置：放行 esbuild 安装脚本（必需）
├─ vite.config.ts              # 含 $lib 别名、Tauri 的 1420 端口约定
├─ svelte.config.js            # compilerOptions.runes = true
├─ tsconfig.json               # strict + $lib path mapping
├─ public/
│  └─ favicon.svg
├─ src/
│  ├─ main.ts                  # mount(App, { target: #app })
│  ├─ App.svelte               # 根组件：侧边栏 + 内容区 + 顶栏
│  ├─ app.css                  # Tailwind v4：@import / @custom-variant / @theme
│  ├─ vite-env.d.ts
│  ├─ routes/                  # 六个功能页面
│  │  ├─ WordFreqPage.svelte   # 生成词频表
│  │  ├─ SentencesPage.svelte  # 划句分析（核心）
│  │  ├─ LeaderboardPage.svelte# 排行榜
│  │  ├─ SettingsPage.svelte   # 设置
│  │  ├─ DictsPage.svelte      # 词典管理
│  │  └─ TablesPage.svelte     # 表管理
│  └─ lib/
│     ├─ utils.ts                     # cn() —— clsx + tailwind-merge
│     ├─ navigation.ts                # 路由表 NAV_ITEMS（id / 标题 / 图标）
│     ├─ types.ts                     # 领域类型（与 vocfreq-core 的 serde 结构对齐）
│     ├─ events.ts                    # 类型化事件总线（scan 进度事件）
│     ├─ format.ts                    # 纯函数：boundsInfo / effectiveBounds / rankForCoverage
│     ├─ segments.ts                  # 划句分析：normalizeToken / summarizeTokens
│     ├─ tier-colors.ts               # 分组 → 配色调色板（唯一权威定义）
│     ├─ tiers.svelte.ts              # 分组状态
│     ├─ theme.svelte.ts              # 主题状态（runes）
│     ├─ theme-sync.ts                # 主窗口 ↔ 小窗主题同步
│     ├─ i18n.svelte.ts               # t() / locale / setLocale
│     ├─ locale-sync.ts               # 主窗口 ↔ 小窗语言同步
│     ├─ number-locale.ts             # 数字本地化（零依赖）
│     ├─ scan-prefill.svelte.ts       # 生成词频表页的扫描参数预填
│     ├─ messages/                    # i18n 消息表（zh-CN 源语言 + en）
│     ├─ api/
│     │  └─ bridge.ts                 # Tauri 调用唯一出口
│     └─ components/
│        ├─ analysis/                 # 划句分析组件
│        │  ├─ TierLegend.svelte
│        │  ├─ TierStatsTable.svelte
│        │  ├─ TokenChips.svelte
│        │  └─ TokenDetail.svelte
│        ├─ layout/
│        │  ├─ Sidebar.svelte         # 左侧固定导航
│        │  ├─ Topbar.svelte          # 内容区标题栏 + 搜索 + 主题切换
│        │  ├─ ThemeToggle.svelte     # 浅色 / 深色 / 跟随系统
│        │  └─ PageStub.svelte        # 占位外壳（仅兜底）
│        ├─ icons/                    # 手写内联 SVG（不依赖图标包）
│        │  ├─ index.ts
│        │  ├─ icon-types.ts
│        │  ├─ IconChart / IconSplit / IconTrophy / IconSettings
│        │  ├─ IconSun / IconMoon / IconBook / IconTable
│        │  └─ LogoMark.svelte
│        └─ ui/                       # shadcn-svelte 风格组件
│           ├─ index.ts
│           ├─ button/Button.svelte
│           ├─ card/{Card,CardHeader,CardTitle,CardDescription,CardContent,CardFooter}.svelte
│           ├─ input/Input.svelte
│           ├─ badge/Badge.svelte
│           ├─ separator/Separator.svelte
│           └─ switch/Switch.svelte   # 基于 bits-ui
└─ src-tauri/
   ├─ Cargo.toml                # ⚠️ 含空 [workspace] 表 → 独立 workspace
   ├─ build.rs
   ├─ tauri.conf.json           # productName / identifier / 窗口尺寸
   ├─ app-icon.png              # 图标源图（1024×1024），改图标后重跑 tauri icon
   ├─ icons/                    # pnpm tauri icon 生成的全套图标
   └─ src/
      ├─ main.rs                # 薄壳，调 lib::run()
      └─ lib.rs                 # Builder + #[tauri::command] 挂载点
```

---

## 6. 主题（浅色 / 深色）

- 令牌集中定义在 `src/app.css` 的 `:root` / `.dark` 里，用 `oklch()` 写色值，
  再通过 `@theme { --color-*: var(--*) }` 暴露成 Tailwind 工具类
  （`bg-background`、`text-muted-foreground`、`border-border` …）。
- 暗色通过 `<html class="dark">` 触发（`@custom-variant dark`），
  **不是** 只依赖 `prefers-color-scheme`，这样才有手动开关。
- `index.html` 里有一段内联脚本，在 CSS 加载前就把 class 打好，避免首屏白闪。
  它和 `src/lib/theme.svelte.ts` 共用同一个 localStorage key：`voctier-theme`。
- 手动切换入口：顶栏右侧的 `ThemeToggle`（浅色 / 深色 / 跟随系统），
  设置页也复用同一个组件。

改色只需要动 `src/app.css` 里那两个 block，不用碰任何组件。

---

## 7. 前后端对接（已实现）

前端通过 `src/lib/api/bridge.ts` 统一调用 Rust 侧 `#[tauri::command]`（返回
`Result<T, String>`，错误信息为中文）。命令清单与事件见 `docs/DESIGN.md` §8.1。

| 位置 | 职责 | 状态 |
| --- | --- | --- |
| `src/lib/api/bridge.ts` | 前端调用 Tauri 的唯一出口（`invoke` + 事件监听） | 已实现 |
| `src-tauri/src/lib.rs` | 注册全部 `#[tauri::command]` 与事件 | 已实现 |
| `src/lib/events.ts` | 类型化事件总线：`scan:progress` / `scan:done` / `scan:error` | 已实现 |
| `src/lib/navigation.ts` | 单一路由表：`RouteId` + `NAV_ITEMS` | 已可用 |
| `src/lib/theme.svelte.ts` | 主题状态：`theme` / `setTheme` / `cycleTheme` / `resolveTheme` | 已可用 |
| `src/lib/i18n.svelte.ts` | `t()` / locale / 分组标签解析 | 已可用（zh-CN 源语言） |

### 换成真实路由

当前用 `$state<RouteId>` 保存当前页，没引入路由库。要换的话：

1. `pnpm add -D svelte-routing`（或别的方案）
2. 把 `navigation.ts` 里的 `id` 换成 `path`
3. 把 `App.svelte` 里的 `{#if}` 分支换成 `<Route>`

页面组件本身不用改。

---

## 8. 关于 `src-tauri/Cargo.toml` 的 `[workspace]`

上层 `<仓库根>\Cargo.toml` 是一个 Cargo workspace，members 只有
`crates/vocfreq-core` 和 `crates/vocfreq-cli`。

Cargo 会**向上查找** workspace 根。如果不做处理，`apps/desktop/src-tauri` 会被
吸进上层 workspace，然后报：

```
error: current package believes it's in a workspace when it's not:
current: .../apps/desktop/src-tauri/Cargo.toml
workspace: .../voctier/Cargo.toml
this may be fixable by adding `apps/desktop/src-tauri` to the `workspace.members` array
```

**本项目刻意不修改上层 workspace**，而是在 `src-tauri/Cargo.toml` 里加了一个空的
`[workspace]` 表，让它自己成为 workspace 根：

```toml
[workspace]
```

效果：`cd src-tauri; cargo check` 正常通过，上层 `cargo build` 也不会把 Tauri
的依赖树（体积很大）拉进来。**请不要删掉这个空表。**

以后要复用 `crates/vocfreq-core`，加路径依赖即可（路径依赖不受 workspace 划分影响）：

```toml
vocfreq-core = { path = "../../../crates/vocfreq-core" }
```

---

## 9. 已锁定的版本

见 `package.json`。要点：

- `svelte` 5.57.1（runes 必须 5.x）
- `@sveltejs/vite-plugin-svelte` 6.2.4（peer 要求 `vite ^6.3 || ^7`）
- `vite` **7.3.6**（vite 8 已发布，但 `@sveltejs/vite-plugin-svelte` 6.x 的 peer
  范围还没包含 8，故降到 7）
- `typescript` **5.9.3**（TS 7 刚发布，svelte-check 4.7 尚未声明支持，故用 5.x）
- `tailwindcss` / `@tailwindcss/vite` 4.3.3
- `bits-ui` 2.19.4（peer: `svelte ^5.33`）
- `@tauri-apps/cli` / `@tauri-apps/api` 2.12.1，Rust `tauri` / `tauri-build` 2.x
  （`cargo check` 实际解析到 `tauri` 2.12.1 / `tauri-build` 2.7.1）
- `@types/node` 24.19.0（`vite.config.ts` 里用到 `process` / `node:url`）
- `clsx` 2.1.1 + `tailwind-merge` 3.7.0（`cn()` 的两个依赖）
- `bits-ui` 2.19.4 是唯一一个「样式无关」的运行时依赖，只被 `ui/switch` 用到

---

## 10. 版本控制

会入库：

- `package.json` / `pnpm-lock.yaml` / `pnpm-workspace.yaml`
- `src-tauri/Cargo.toml` / `src-tauri/Cargo.lock`（**应用工程应该提交 Cargo.lock**）
- `src-tauri/app-icon.png` 与 `src-tauri/icons/`（打包要用）

不入库（已在根 `.gitignore` 和 `apps/desktop/.gitignore` 里忽略）：

- `node_modules/`、`dist/`
- `src-tauri/target/`
- `src-tauri/gen/`（`tauri-build` 生成的 ACL / schema 缓存，会自动重建）

---

## 11. 图标

```powershell
# 改完 src-tauri/app-icon.png（1024×1024）后重新生成全套
pnpm exec tauri icon src-tauri/app-icon.png
```

会生成 `src-tauri/icons/` 下的 Windows / macOS / iOS / Android 全套图标。
`tauri.conf.json` 的 `bundle.icon` 已指向其中的 5 个。
