<#
.SYNOPSIS
  正确构建 VocTier 桌面端。

.DESCRIPTION
  **必须用这个脚本（或直接 pnpm tauri build），不要用 cargo build。**

  原因：Tauri 用**编译期** `cfg(dev)` 决定加载 devUrl 还是内嵌前端资源：

      #[cfg(dev)]      let url = config.build.dev_url;   // → http://localhost:1420
      #[cfg(not(dev))] let url = ...frontend_dist...;    // → 内嵌的前端

  这个 cfg 由 tauri CLI 通过环境变量驱动 build script 设置。直接 `cargo build`
  会绕过 CLI，编成 **dev 模式**；这时脱离 dev server 运行就是
  「白屏 + 无法访问此页面 / localhost 拒绝连接」，
  而窗口、热键、后端日志、数据集载入全都正常，极难察觉。

  应用启动时会把构建模式写进 startup.log，可用 -Check 复核。

.PARAMETER Bundle
  同时生成安装包（NSIS / MSI）。不加则只出 exe，快得多。

.PARAMETER Check
  不构建，只打印当前 exe 的构建模式（读 startup.log 判断）。

.EXAMPLE
  .\tools\build-desktop.ps1              # 只出 exe
  .\tools\build-desktop.ps1 -Bundle      # 连同安装包
  .\tools\build-desktop.ps1 -Check       # 检查现有 exe 是不是生产模式
#>
[CmdletBinding()]
param(
    [switch]$Bundle,
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$desktop = Join-Path $repo 'apps\desktop'
$exe = Join-Path $desktop 'src-tauri\target\release\voctier-desktop.exe'

if ($Check) {
    if (-not (Test-Path $exe)) { throw "还没有构建过：$exe" }
    Write-Host "exe: $exe"
    Write-Host ("     构建于 {0}，{1:N2} MB" -f (Get-Item $exe).LastWriteTime, ((Get-Item $exe).Length / 1MB))
    $log = Join-Path $env:APPDATA 'com.voctier.desktop\startup.log'
    if (Test-Path $log) {
        $mode = Get-Content $log | Select-String '构建模式' | Select-Object -Last 1
        if ($mode) {
            Write-Host "上次启动记录：$($mode.Line -replace '^\[[^\]]+\]\s*', '')"
            if ($mode.Line -match 'dev') {
                Write-Host ''
                Write-Host '⚠ 上次运行是 dev 模式 —— 请用本脚本重新构建，否则脱离 dev server 会白屏。' -ForegroundColor Yellow
            }
        } else {
            Write-Host '（startup.log 里没有构建模式记录，可能是更早的版本）'
        }
    } else {
        Write-Host "（还没有 $log，先运行一次应用）"
    }
    return
}

Write-Host "==> 构建前端 + 桌面端（走 Tauri CLI，确保 production 模式）"

# 抹掉二进制里嵌入的构建机绝对路径。
#
# Rust 会把每个 panic 点的源文件**绝对路径**编译进二进制（panic 消息里要显示
# file:line）。这属于标准行为，不是代码里写死的，但会把构建机的目录结构泄漏给
# 拿到 exe 的人，也让不同机器构建出的二进制不可复现。
#
# `--remap-path-prefix` 把前缀改写成相对形式——注意是**改写**而不是删除，所以
# panic 消息仍然能定位到 `./apps/desktop/src-tauri/src/lib.rs:1234`，调试不受影响。
#
# 用实际路径在构建时生成，因此仓库里不存任何绝对路径。
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustflags = @(
    "--remap-path-prefix=$repo=."
    "--remap-path-prefix=$cargoHome=/cargo"
) -join ' '
if ($env:RUSTFLAGS -and $env:RUSTFLAGS -notlike '*remap-path-prefix*') {
    $rustflags = "$env:RUSTFLAGS $rustflags"
}
$env:RUSTFLAGS = $rustflags
Write-Host "    RUSTFLAGS = $rustflags"
Write-Host '    （首次会全量重编，因为 RUSTFLAGS 变了）'

# 构建临时目录指到工作区内。
# 某些环境里 MSVC 的 lib.exe 无法在系统 TEMP 建临时文件，报
# `LNK1104: 无法打开文件 ...\Temp\lnk{...}.tmp`（zstd-sys 这类带 C 代码的
# 依赖会因此构建失败）。换到工作区内可稳定绕开，对产物没有影响。
$buildTmp = Join-Path $repo '.tmp'
New-Item -ItemType Directory -Force -Path $buildTmp | Out-Null
$env:TMP = $buildTmp
$env:TEMP = $buildTmp

Push-Location $desktop
try {
    # 别用 cargo build：那会编成 dev 模式，运行白屏（见本脚本说明）
    if ($Bundle) {
        pnpm tauri build
    } else {
        pnpm tauri build --no-bundle
    }
    if ($LASTEXITCODE -ne 0) { throw "tauri build 失败（exit $LASTEXITCODE）" }
} finally {
    Pop-Location
}

if (Test-Path $exe) {
    Write-Host ''
    Write-Host ("✅ {0}" -f $exe)
    Write-Host ("   {0:N2} MB，构建于 {1}" -f ((Get-Item $exe).Length / 1MB), (Get-Item $exe).LastWriteTime)

    # 顺手复核：构建机路径确实被抹掉了。
    #
    # 注意：探查用的字符串**在运行时算出来**，绝不写死字面路径——否则这段校验代码
    # 自己就成了源码里的绝对路径泄漏点，正是它要防的问题。
    $bytes = [System.IO.File]::ReadAllBytes($exe)
    $text = [System.Text.Encoding]::UTF8.GetString($bytes)
    $needles = @(
        $repo
        $repo.Replace('\', '/')
        (Split-Path $repo -Parent)
        $env:USERNAME
    ) | Where-Object { $_ -and $_.Length -ge 4 } | Select-Object -Unique

    $found = @()
    foreach ($needle in $needles) {
        $n = ([regex]::Matches($text, [regex]::Escape($needle))).Count
        if ($n -gt 0) { $found += "'$needle' ×$n" }
    }
    if ($found.Count -gt 0) {
        Write-Host ''
        Write-Host "⚠ 二进制里仍能搜到构建机路径：$($found -join '、')" -ForegroundColor Yellow
        Write-Host '  （依赖 crate 若已编进缓存且未重编，可能需要 cargo clean 后再试）'
    } else {
        Write-Host '   ✅ 已复核：二进制里搜不到构建机绝对路径'
    }

    Write-Host ''
    Write-Host '可用 .\tools\build-desktop.ps1 -Check 复核构建模式。'

    # ==========================================================================
    # 把产物复制到「工作区之外」再交给用户运行。
    #
    # ⚠ 这一步不是可选项，是**必须**的。
    #
    # 原因（排查了很久才定位到）：某些开发环境会给整个仓库目录打上
    # 「低完整性强制标签」（`Mandatory Label\Low Mandatory Level:(NW)`），
    # 而 Windows 会让从这种文件启动的进程也运行在**低完整性级别**。
    # 低完整性进程会被 UIPI 挡住，**无法向普通窗口注入按键**，
    # 也写不进 %APPDATA% / %LOCALAPPDATA%：
    #
    #   * 取词（模拟 Ctrl+C）永远「没取到内容」，而且 SendInput 不报任何错；
    #   * 日志只能退回程序目录，设置也存不下来；
    #   * WebView2 在默认位置建不了数据目录（报「拒绝访问」或「灾难性故障」）。
    #
    # 复制到工作区外后，进程恢复正常完整性级别，上述问题一次性全部消失。
    # 实测对比：工作区内 = Low，工作区外 = Medium/High，取词立即成功。
    # ==========================================================================
    $runDir = Join-Path $env:USERPROFILE 'VocTier'
    New-Item -ItemType Directory -Force -Path $runDir | Out-Null
    $runExe = Join-Path $runDir 'voctier-desktop.exe'
    Copy-Item $exe $runExe -Force

    Write-Host ''
    Write-Host '✅ 已复制到工作区之外（请运行这一份）：' -ForegroundColor Green
    Write-Host "   $runExe"

    # 复核副本确实没有低完整性标签
    $label = (icacls $runExe 2>&1 | Select-String 'Mandatory Label')
    if ($label) {
        Write-Host ''
        Write-Host "⚠ 副本仍带强制标签：$($label.Line.Trim())" -ForegroundColor Yellow
        Write-Host '  这会导致取词失效（低完整性进程无法向普通窗口注入按键）。'
    } else {
        Write-Host '   ✅ 副本无低完整性标签，取词可正常工作'
    }
}
