<#
.SYNOPSIS
  把 VocTier 全仓库的版本号一次性同步到同一个 SemVer。

.DESCRIPTION
  本仓库的版本号散落在 6 处，手动逐个改必漏，本脚本一次改齐：

    1. Cargo.toml                      [workspace.package] version
                                       （vocfreq-core / vocfreq-cli 用 version.workspace 继承它）
    2. Cargo.lock                      vocfreq-core、vocfreq-cli 两个 package 的 version
    3. apps/desktop/package.json       version
    4. apps/desktop/src-tauri/Cargo.toml    [package] version（独立 workspace）
    5. apps/desktop/src-tauri/tauri.conf.json  version（决定安装包的显示版本）
    6. apps/desktop/src-tauri/Cargo.lock    voctier-desktop、vocfreq-core 两个 package 的 version
                                       （vocfreq-core 是 path 依赖，也会出现在这里）

  只改文件、不做网络操作。改完建议先 git diff 复核；-Commit 才自动 git add+commit。
  **不自动 push、不打 tag**——tag 与 push 由你确认，避免误发布。

.PARAMETER Version
  目标版本号，必须是合法 SemVer：MAJOR.MINOR.PATCH[-pre][+build]。
  不要带 v 前缀（tag 才带 v）。

.PARAMETER Commit
  改完后自动 git add 上述 6 个文件并 commit（消息带新版本号）。

.EXAMPLE
  .\tools\bump-version.ps1 0.2.0
  .\tools\bump-version.ps1 0.2.1 -Commit
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Version,

    [switch]$Commit
)

$ErrorActionPreference = 'Stop'

# ---- 1. 校验 SemVer（不允许 v 前缀）----
$semver = '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+)?$'
if ($Version -notmatch $semver) {
    throw "『$Version』不是合法 SemVer：应为 0.2.0 / 0.2.0-rc.1 / 1.0.0+win.1，且不要带 v 前缀"
}

$repo = Split-Path -Parent $PSScriptRoot
$desktop = Join-Path $repo 'apps\desktop'
$files = @(
    (Join-Path $repo 'Cargo.toml'),
    (Join-Path $repo 'Cargo.lock'),
    (Join-Path $desktop 'package.json'),
    (Join-Path $desktop 'src-tauri\Cargo.toml'),
    (Join-Path $desktop 'src-tauri\tauri.conf.json'),
    (Join-Path $desktop 'src-tauri\Cargo.lock')
)
foreach ($f in $files) {
    if (-not (Test-Path $f)) { throw "缺少文件：$f" }
}

# ---- 2. 以根 Cargo.toml 的 workspace.package 为权威，取当前版本 ----
$rootText = [IO.File]::ReadAllText($files[0])
$m = [regex]::Match($rootText, '(?m)^\[workspace\.package\]\s*\r?\n(?:[^\[]*\r?\n)*?version\s*=\s*"([^"]+)"')
if (-not $m.Success) { throw "无法从根 Cargo.toml 解析当前版本（[workspace.package] 段找不到 version）" }
$old = $m.Groups[1].Value
if ($old -eq $Version) { throw "当前已是 $old，无需 bump" }
Write-Host "当前版本 $old  →  目标版本 $Version" -ForegroundColor Cyan

# ---- 3. 统一改文件：UTF-8 无 BOM，保留原换行 ----
function Write-Utf8NoBom {
    param([string]$Path, [string]$Text)
    [IO.File]::WriteAllText($Path, $Text, (New-Object System.Text.UTF8Encoding $false))
}

# 3a. 普通定义行：Cargo.toml 的 `version = "x"` 与 json 的 `"version": "x"`
function Set-DefLine {
    param([string]$Path, [string]$Pattern, [string]$Replacement)
    $t = [IO.File]::ReadAllText($Path)
    $count = ([regex]::Matches($t, $Pattern)).Count
    $new = [regex]::Replace($t, $Pattern, $Replacement)
    Write-Utf8NoBom $Path $new
    if ($count -gt 0) { Write-Host "  ✓ $Path  （$count 处）" }
    else              { Write-Host "  - $Path  （未命中，请人工检查）" }
}

Set-DefLine -Path $files[0] `
    -Pattern '(?m)^(version\s*=\s*")[^"]+(")' `
    -Replacement ('${1}' + $Version + '${2}')

Set-DefLine -Path $files[3] `
    -Pattern '(?m)^(version\s*=\s*")[^"]+(")' `
    -Replacement ('${1}' + $Version + '${2}')

Set-DefLine -Path $files[2] `
    -Pattern '(?m)("version"\s*:\s*")[^"]+(")' `
    -Replacement ('${1}' + $Version + '${2}')

Set-DefLine -Path $files[4] `
    -Pattern '(?m)(^\s*"version"\s*:\s*")[^"]+(")' `
    -Replacement ('${1}' + $Version + '${2}')

# 3b. Cargo.lock：按 `name = "X"` 定位其紧跟的 version 行
function Set-LockVersion {
    param([string]$Path, [string[]]$Names)
    $t = [IO.File]::ReadAllText($Path)
    foreach ($n in $Names) {
        $esc = [regex]::Escape($n)
        # 保留原换行（\r?\n），只替换版本号本体
        $pat = '(?m)(name = "' + $esc + '")(\r?\n)(version = ")[^"\r\n]+(")'
        $t = [regex]::Replace($t, $pat, ('${1}${2}${3}' + $Version + '${4}'))
    }
    Write-Utf8NoBom $Path $t
    Write-Host "  ✓ $Path  （$($Names -join ' / ')）"
}

Set-LockVersion -Path $files[1] -Names @('vocfreq-core', 'vocfreq-cli')
Set-LockVersion -Path $files[5] -Names @('voctier-desktop', 'vocfreq-core')

# ---- 4. 一致性复核：6 个文件的“自身版本定义”都应是新版本 ----
Write-Host ''
Write-Host '—— 复核 ——' -ForegroundColor Cyan
$ok = $true
foreach ($f in $files) {
    $t = [IO.File]::ReadAllText($f)
    $bad = @()
    if ($f -like '*Cargo.toml') {
        if ($t -match '(?m)^\s*version\s*=\s*"' + [regex]::Escape($old) + '"') { $bad += 'version 行残留旧值' }
    } elseif ($f -like '*package.json' -or $f -like '*tauri.conf.json') {
        if ($t -match '"version"\s*:\s*"' + [regex]::Escape($old) + '"') { $bad += 'version 残留旧值' }
    } else { # Cargo.lock
        $stale = [regex]::Matches($t, '(?m)(name = "([^"]+)")(\r?\nversion = ")' + [regex]::Escape($old) + '"')
        foreach ($s in $stale) {
            if ($s.Groups[2].Value -in @('vocfreq-core', 'vocfreq-cli', 'voctier-desktop')) {
                $bad += ('lock 中 {0} 仍是旧值' -f $s.Groups[2].Value)
            }
        }
    }
    if ($bad.Count -gt 0) {
        Write-Host "  ✗ $f  →  $($bad -join '；')" -ForegroundColor Red
        $ok = $false
    } else {
        Write-Host "  ✓ $f"
    }
}
if (-not $ok) { Write-Host ''; throw '复核未通过：仍有旧版本号残留，请人工检查（见上方 ✗）' }
Write-Host ("✅ 6 处版本号已全部同步为 {0}" -f $Version) -ForegroundColor Green

# ---- 5. 可选提交 ----
if ($Commit) {
    Push-Location $repo
    try {
        git add Cargo.toml Cargo.lock `
            apps/desktop/package.json `
            apps/desktop/src-tauri/Cargo.toml `
            apps/desktop/src-tauri/tauri.conf.json `
            apps/desktop/src-tauri/Cargo.lock
        git commit -m "chore(release): bump version to $Version"
        Write-Host ''
        Write-Host ('✅ 已提交。确认无误后：git tag v{0}  &&  git push origin main --tags' -f $Version) -ForegroundColor Green
    } finally {
        Pop-Location
    }
}
