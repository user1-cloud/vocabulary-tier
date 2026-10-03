<#
.SYNOPSIS
  本地打包后一键发布 VocTier：把 NSIS 安装包上传到对应 tag 的 GitHub Release 并直接发布。

.DESCRIPTION
  本仓库的安装包必须在本地构建（预置 seed 依赖本地语料，CI 拿不到），
  因此"发布"这步也用本地脚本一键完成，替代手动打开网页 -> 拖拽上传 -> 点 Publish。

  完整流程（对照 docs/RELEASE.md）：
    1. .\tools\bump-version.ps1 0.2.0 -Commit        # bump 版本并提交
    2. git push origin main
    3. git tag v0.2.0 && git push origin main --tags  # 触发 release.yml 建草稿 + changelog
    4. .\tools\publish.ps1  [-Build]  [-Draft]       # 上传安装包并发布（本脚本）

  默认行为：
    - 不构建，直接上传 apps/desktop/src-tauri/target/release/bundle/nsis/*-setup.exe
    - 上传到 tag v<当前版本> 对应的 GitHub Release（若 release.yml 已建好草稿则复用；否则创建）
    - 默认直接把 Release 置为正式发布（非草稿），Draft 后缀加 -Draft 保留为草稿
    - 若 release 尚不存在且 tag 刚 push、Actions 还没跑完，脚本会轮询等待该草稿出现

.PARAMETER Build
  发布前先本地构建安装包：prepare-seed.ps1 + build-desktop.ps1 -Bundle。
  不加则只上传已有的 setup.exe（更快，适合已构建过、只想补发布的情况）。

.PARAMETER Draft
  上传后保留为草稿（不自动 Publish），让你在网页上再核一眼 changelog 后手动发布。

.PARAMETER Check
  不发布，只打印将要上传的安装包路径、目标版本与 tag。

.EXAMPLE
  .\tools\publish.ps1                  # 上传现有 setup.exe 并直接发布
  .\tools\publish.ps1 -Build           # 先打包再发布
  .\tools\publish.ps1 -Build -Draft    # 先打包，上传后保留草稿待人工确认
  .\tools\publish.ps1 -Check           # 只查看将发布的版本与安装包
#>
[CmdletBinding()]
param(
    [switch]$Build,
    [switch]$Draft,
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$repo   = Split-Path -Parent $PSScriptRoot
$desktop = Join-Path $repo 'apps\desktop'
$nsisDir = Join-Path $desktop 'src-tauri\target\release\bundle\nsis'

# ---- 0. 前置依赖：gh CLI（GitHub 官方命令行）----
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw @"
未找到 gh CLI。请先安装（任选其一）：

  # 方式一：winget（推荐）
  winget install GitHub.cli

  # 方式二：官方安装器（PowerShell）
  (Invoke-RestMethod https://api.github.com/repos/cli/cli/releases/latest).assets |
    Where-Object { $_.name -match 'windows_amd64.*msi$' } |
    ForEach-Object { $p = Join-Path $env:TEMP $_.name; Invoke-WebRequest $_.browser_download_url -OutFile $p; Start-Process msiexec -ArgumentList "/i `"$p`" /qn" -Wait }

装完后打开新终端执行一次： gh auth login
"@
}

# ---- 1. 从根 Cargo.toml 读当前版本（权威来源，与 bump-version 一致）----
$rootText = [IO.File]::ReadAllText((Join-Path $repo 'Cargo.toml'))
$m = [regex]::Match($rootText, '(?m)^\[workspace\.package\]\s*\r?\n(?:[^\[]*\r?\n)*?version\s*=\s*"([^"]+)"')
if (-not $m.Success) { throw '无法从根 Cargo.toml 解析当前版本（[workspace.package] 段找不到 version）' }
$version = $m.Groups[1].Value
$tag = "v$version"

# ---- 2. 定位安装包 ----
$setup = Get-ChildItem $nsisDir -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) {
    throw "未找到安装包：$nsisDir\*-setup.exe。请先 .\tools\build-desktop.ps1 -Bundle（或本脚本加 -Build）"
}

Write-Host ('目标版本: {0}   tag: {1}' -f $version, $tag) -ForegroundColor Cyan
Write-Host ('安装包  : {0}  ({1:N2} MB, {2})' -f $setup.FullName, ($setup.Length/1MB), $setup.LastWriteTime)

if ($Check) { return }

# ---- 3. 可选：本地先打包 ----
if ($Build) {
    Write-Host '==> 先构建安装包…' -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot 'prepare-seed.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'prepare-seed.ps1 失败' }
    & (Join-Path $PSScriptRoot 'build-desktop.ps1') -Bundle
    if ($LASTEXITCODE -ne 0) { throw 'build-desktop.ps1 -Bundle 失败' }
    # 重新定位安装包（可能刚生成）
    $setup = Get-ChildItem $nsisDir -Filter '*-setup.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $setup) { throw '构建完成后仍未找到 setup.exe' }
}

# ---- 4. 确认远程仓库与登录态 ----
$remote = git -C $repo remote get-url origin
if (-not $remote) { throw '未配置 git remote origin' }
Write-Host ('remote  : {0}' -f $remote)

gh auth status 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'gh 未登录，请先执行: gh auth login' }

# ---- 5. 找到或等待对应 tag 的 Release（release.yml 建的草稿）----
# release.yml 在 push v* tag 后建草稿 + changelog；可能 Actions 还在跑，轮询等待最多 ~5 分钟。
function Get-ReleaseStatus {
    $json = gh release view $tag --json isDraft,id 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return $json
}

$rel = Get-ReleaseStatus
if (-not $rel) {
    Write-Host "Release '$tag' 还不存在。若刚 push tag，Actions 正在跑 release.yml…" -ForegroundColor Yellow
    $deadline = (Get-Date).AddMinutes(5)
    while (-not $rel -and (Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 15
        $rel = Get-ReleaseStatus
    }
    if (-not $rel) {
        Write-Host '等待超时。将直接创建 Release（changelog 用 git-cliff 本地生成）。' -ForegroundColor Yellow
    }
}

# ---- 6. 上传安装包（幂等：已存在则覆盖）----
Write-Host '==> 上传安装包…'
gh release upload $tag $setup.FullName --clobber
if ($LASTEXITCODE -ne 0) { throw '上传失败' }

# ---- 7. 发布（默认直接正式发布；-Draft 则保留草稿）----
if ($rel) {
    if (-not $Draft) {
        Write-Host '==> 发布 Release…'
        gh release edit $tag --draft=false
        if ($LASTEXITCODE -ne 0) { throw '发布失败' }
        Write-Host ('✅ 已发布: {0}  {1}' -f $tag, "https://github.com/user1-cloud/vocabulary-tier/releases/tag/$tag") -ForegroundColor Green
    } else {
        Write-Host ('已上传到草稿 Release {0}。请在网页确认 changelog 后手动发布。' -f $tag) -ForegroundColor Yellow
    }
} else {
    # release.yml 没建出来（比如首次 / Actions 未跑），本地用 git-cliff 生成 changelog 并创建正式 Release
    Write-Host '==> 本地创建 Release…'
    $cliff = Join-Path $repo '.github\cliff.toml'
    $notes = if (Test-Path $cliff -and (Get-Command git-cliff -ErrorAction SilentlyContinue)) {
        git -C $repo cliff --config $cliff --tag $tag 2>$null
        if ($LASTEXITCODE -eq 0) { $notes = $_ } else { '' }
    } else { '' }
    $args = @('release', 'create', $tag, $setup.FullName, '--title', "VocTier $tag")
    if (-not $Draft) { $args += '--latest' } else { $args += '--draft' }
    if ($notes) { $args += '--notes', $notes }
    gh @args
    if ($LASTEXITCODE -ne 0) { throw '创建 Release 失败' }
    Write-Host ('✅ 已创建并发布: {0}' -f $tag) -ForegroundColor Green
}

Write-Host ''
Write-Host '用户即可从 Release 页下载安装包。'
