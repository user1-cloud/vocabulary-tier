<#
.SYNOPSIS
  组装安装包要随包携带的「预置词库」与「预置词表」。

.DESCRIPTION
  产物落在 apps\desktop\src-tauri\seed\，安装钩子
  （apps\desktop\src-tauri\nsis\installer-hooks.nsh）会把它们直接写进
  %LOCALAPPDATA%\com.voctier.desktop\data\，**不经过 $INSTDIR**。

  组装出来的东西和用户自己新建的**没有任何区别**：装完就是普通文件，能删能改能替换。
  .origin 边车文件只让界面显示成"预置"，不附加任何权限限制。

  词库来源：assets\seed\dicts\ 下随仓库携带的那一份（jieba 的 dict.txt，MIT，
  349,046 条）。用 -RefreshDict 可以从本机 cargo 注册表重新取一份刷新它。

  词表怎么生成：调 `vocfreq prepare-seed`，由 Rust 侧改写 meta.json。
  ⚠ 这一步**不能**用 PowerShell 拼 JSON：meta.json 里 tier_stats[].max_rank 是
  u64::MAX，ConvertFrom-Json 会读成 Double，回写成 1.8446744073709552E+19，
  Rust 侧再也反序列化不回来。

.PARAMETER FromData
  预置词表从哪个产物目录摘，默认仓库根下的 data\（vocfreq scan 的输出）。
  ⚠ `scan` 现在**默认不产出 `full`**（它改由「merge 合流 + compose 相加」得到），
  所以这个目录必须先有一条 `full`：要么把各作用域相加出来，要么用 `--full` 扫一次。

.PARAMETER IntoDataFolder
  组装完顺手铺一份到当前用户的数据文件夹，这样不装安装包也能开发调试。

.PARAMETER RefreshDict
  从本机 cargo 注册表里的 jieba-rs 重新复制 dict.txt 到 assets\seed\dicts\。

.EXAMPLE
  .\tools\prepare-seed.ps1
  .\tools\prepare-seed.ps1 -IntoDataFolder
  .\tools\prepare-seed.ps1 -RefreshDict
#>
[CmdletBinding()]
param(
    [string]$FromData,
    [switch]$IntoDataFolder,
    [switch]$RefreshDict
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $repo 'assets\seed\dicts'
$seed = Join-Path $repo 'apps\desktop\src-tauri\seed'
if (-not $FromData) { $FromData = Join-Path $repo 'data' }

$tableName = '预制表'
$dictName = '预制词库.dict'

function Write-Step($msg) { Write-Host "==> $msg" }

# ---------------------------------------------------------------- 词库
Write-Step '准备预置词库'

if ($RefreshDict) {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $src = Get-ChildItem (Join-Path $cargoHome 'registry\src') -Recurse -File -Filter 'dict.txt' -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match 'jieba-rs' } |
        Select-Object -First 1
    if (-not $src) {
        throw "在 $cargoHome\registry\src 里找不到 jieba-rs 的 dict.txt。先跑一次 cargo build 让依赖下载下来，或手动放一份到 $assets。"
    }
    New-Item -ItemType Directory -Force -Path $assets | Out-Null
    Copy-Item $src.FullName (Join-Path $assets $dictName) -Force
    Write-Host "    已从 $($src.FullName) 刷新"
}

if (-not (Test-Path $assets)) { throw "缺少 $assets。先跑一次 .\tools\prepare-seed.ps1 -RefreshDict 把词库带进仓库。" }
$dictsInAssets = @(Get-ChildItem $assets -File -Filter '*.dict')
if ($dictsInAssets.Count -eq 0) { throw "$assets 里没有 .dict。先跑一次 -RefreshDict。" }
if ($dictsInAssets.Count -gt 1) {
    throw "$assets 里有 $($dictsInAssets.Count) 个 .dict，预置词库只能有一份：$($dictsInAssets.Name -join '、')"
}
$seedDict = $dictsInAssets[0]
Write-Host ("    {0}  {1:N2} MB" -f $seedDict.Name, ($seedDict.Length / 1MB))

# ---------------------------------------------------------------- 组装目录
if (Test-Path $seed) { Remove-Item $seed -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $seed 'dicts') | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $seed 'tables') | Out-Null

$seedDictPath = Join-Path $seed "dicts\$dictName"
Copy-Item $seedDict.FullName $seedDictPath -Force
# 来源边车：让界面把这个标成"预置"。纯展示，不加任何限制。
Set-Content -Path "$seedDictPath.origin" -Value 'seeded' -NoNewline -Encoding utf8

# ---------------------------------------------------------------- 预置词表
Write-Step '准备预置词表'

# 预置表要带的是**全部作用域**（各分域 + `full`）。`full` 现在不是 scan 的默认产物
# （见 docs/DATA_LAYOUT.md §七），所以这里仍然要求它——它是出厂表的主作用域默认值，
# 缺了用户在「表管理」里就没有一张现成的全量表。
$haveSource = (Test-Path (Join-Path $FromData 'meta.json')) -and
              (Test-Path (Join-Path $FromData 'full\word.vfr')) -and
              (Test-Path (Join-Path $FromData 'full\char.vfr'))

if (-not $haveSource) {
    Write-Warning @"
没有找到可用的源产物目录：$FromData
需要 meta.json + full\word.vfr + full\char.vfr。
注意：scan 现在默认不产出 full（它改由「merge 合流 + compose 相加」得到），所以
这个目录要么先跑一条
  vocfreq compose --from-data <产物目录> --source <域1>/word --scope full
要么用
  vocfreq scan --corpus <语料> --out <产物目录> --dict <词库> --full
扫一份。
本次只会组装词库，安装包不含预置词表 —— 用户装完得自己跑一次统计才能查词。
"@
} else {
    $tableOut = Join-Path $seed "tables\$tableName"
    Push-Location $repo
    try {
        cargo run -q --offline -p vocfreq-cli -- prepare-seed `
            --from-data $FromData `
            --dict $seedDictPath `
            --out $tableOut `
            --table-name $tableName
        if ($LASTEXITCODE -ne 0) { throw "prepare-seed 失败（exit $LASTEXITCODE）" }
    } finally {
        Pop-Location
    }
    # 表目录的来源边车是它的**兄弟**文件（与 library.rs 的 origin_sidecar 一致：
    # 往目录路径后面直接接 ".origin"）
    Set-Content -Path (Join-Path $seed "tables\$tableName.origin") -Value 'seeded' -NoNewline -Encoding utf8
}

# ---------------------------------------------------------------- 复核
Write-Step '复核组装结果'
$total = 0
Get-ChildItem $seed -Recurse -File | ForEach-Object {
    $total += $_.Length
    Write-Host ("    {0,-46} {1,10:N2} MB" -f $_.FullName.Replace("$seed\", ''), ($_.Length / 1MB))
}
Write-Host ("    合计 {0:N1} MB（会压缩进安装包）" -f ($total / 1MB))

# 安装钩子里的 File /r 源目录不能为空，否则 makensis 直接报错
if (-not (Get-ChildItem (Join-Path $seed 'dicts') -File -ErrorAction SilentlyContinue)) {
    throw "seed\dicts\ 是空的 —— NSIS 的 File /r 遇到空目录会构建失败"
}
if (-not (Get-ChildItem (Join-Path $seed 'tables') -ErrorAction SilentlyContinue)) {
    throw "seed\tables\ 是空的 —— NSIS 的 File /r 遇到空目录会构建失败"
}

# ---------------------------------------------------------------- 可选：铺进数据文件夹
if ($IntoDataFolder) {
    Write-Step '铺一份到当前用户的数据文件夹（开发调试用）'
    $dataRoot = Join-Path $env:LOCALAPPDATA 'com.voctier.desktop\data'
    foreach ($sub in 'dicts', 'tables') {
        $dst = Join-Path $dataRoot $sub
        New-Item -ItemType Directory -Force -Path $dst | Out-Null
        # **会覆盖同名文件** —— 故意与安装器保持一致（NSIS 的 File 就是覆盖语义），
        # 这样"重跑一次"和"重装一次"得到的结果相同，不会出现
        # "装完是新表、跑脚本却是旧表"这种只在某一侧复现的怪问题。
        Copy-Item (Join-Path $seed "$sub\*") $dst -Recurse -Force
    }
    Write-Host "    已覆盖到 $dataRoot（与安装器同语义：同名文件会被刷新）"
}

Write-Host ''
Write-Host '✅ 预置内容就绪。接下来：' -ForegroundColor Green
Write-Host "   .\tools\build-desktop.ps1 -Bundle    # 出安装包（只出 NSIS）"
Write-Host '   或 .\tools\prepare-seed.ps1 -IntoDataFolder 后直接跑 exe（开发调试）'
