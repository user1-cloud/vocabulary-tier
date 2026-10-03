# VocTier 发布 SOP

本文是「bump 版本号 → 打 tag → 自动建 Release → 本地出安装包 → 上传发布」的完整操作手册。
配套文件：

| 文件 | 作用 |
|---|---|
| `tools/bump-version.ps1` | 一次同步全部 6 处版本号 + 一致性复核，可选 `-Commit` |
| `.github/workflows/ci.yml` | PR / main push：fmt、clippy、单测、前端 check+build、桌面后端 cargo check |
| `.github/workflows/release.yml` | tag `v*` 推送：git-cliff 生成 changelog → 建草稿 Release + 上传指引 |
| `.github/cliff.toml` | git-cliff 分组配置（按 conventional commits） |

---

## 0. 一次性前置（只在首次做）

- 确认 GitHub Actions 在仓库里已启用（Settings → Actions → General → Allow all workflows）。
- 确认 `ci.yml` 首次 push 能通过（尤其 `desktop-backend` 的 `cargo check`，见「常见问题」）。
- 确认本机能跑构建：`pnpm`、Rust toolchain、MSVC 均已配置。

## 1. 日常：bump 版本号

```powershell
# 只改文件 + 复核，不改 git（先看清楚 diff 再提交）
.\tools\bump-version.ps1 0.2.0

# 复核改动
git diff

# 确认无误后，带 -Commit 直接提交（消息自动带版本号）
.\tools\bump-version.ps1 0.2.0 -Commit
```

> 版本号散落 6 处，bump 脚本会一次全改：根 `Cargo.toml`（workspace.package，core/cli 继承）、
> 根 `Cargo.lock`、`apps/desktop/package.json`、`src-tauri/Cargo.toml`、
> `src-tauri/tauri.conf.json`、`src-tauri/Cargo.lock`。不要手动逐个改。

## 2. 推送代码 + 打 tag

```powershell
git push origin main

git tag v0.2.0
git push origin main --tags
```

推送 tag 后，`release.yml` 会自动：
1. 用 git-cliff 从上一个 tag 到当前 tag 的 commits 生成 changelog；
2. 创建一个 **draft（草稿）** GitHub Release，带名字 `VocTier v0.2.0` + changelog + 安装包上传指引。

## 3. 本地构建安装包

安装包不在 CI 构建——预制 seed（约 92MB）依赖本地 0.5GB 语料生成，
CI 全新 clone 拿不到。必须在本地出包：

```powershell
# 组装预置词典 / 词频表（每次出包前都要）
.\tools\prepare-seed.ps1

# 构建 + 出 NSIS 安装包（必须用这个脚本，别用 cargo build，会编成 dev 模式白屏）
.\tools\build-desktop.ps1 -Bundle
```

产物位置：
```
apps/desktop/src-tauri/target/release/bundle/nsis/*-setup.exe
```

## 4. 上传并发布

1. 打开 GitHub → 仓库 → Releases，找到 `ci.yml` 自动建好的草稿 `VocTier v0.2.0`。
2. 点 Edit，把 `*-setup.exe` 拖进 Assets 上传。
3. 核对版本号、changelog，点 **Publish release**。

发布即完成。用户从 Release 页下载安装包。

---

## 常见问题

- **首次发 tag，changelog 很长？**
  正常。git-cliff 没有"上一个 tag"，会从仓库第一条 commit 开始生成；之后的 tag 只对比相邻 tag。
- **`ci.yml` 的 `desktop-backend` 的 `cargo check` 报错（找不到 frontendDist / dist）？**
  `src-tauri` 的 build.rs 可能要求前端产物 `apps/desktop/dist` 存在。解决：在该 job 里 `cargo check` 前加
  `pnpm install && pnpm build`（把前后端合并到一个 job）。
- **忘了 `-Commit`，文件改了没提交？**
  脚本只改文件；`git status` 能看到 6 个文件改动，手动 `git add` + `git commit` 即可。
- **bump 脚本说"复核未通过 / 仍有旧版本残留"？**
  脚本会列出处并抛错。检查对应文件里是否真的有 `version = "旧号"` 残留；若是依赖版本（如别的 crate 也叫 0.1.0）则是误报，可跳过。

## 回滚

- **版本号改错了、还没提交**：`git checkout -- .` 还原（注意会丢掉未提交改动）。
- **已提交但没打 tag**：`git reset --soft HEAD~1` 撤销最后一次 commit，再重新 bump。
- **tag 打错、已 push**：`git tag -d v0.2.0 && git push origin :refs/tags/v0.2.0`，删掉后重打。
