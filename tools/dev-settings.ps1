<#
.SYNOPSIS
  开发和测试辅助：管理 VocTier 桌面端的设置文件。

.DESCRIPTION
  桌面端把设置存在 %APPDATA%\com.voctier.desktop\settings.json。
  测试时经常需要「直接把某份产物目录指上去」或「把数据文件夹换到别处」，
  而不是每次都在界面里点一遍；设置被填坏之后也需要一键复位。这个脚本干这些事。

  ⚠ 两种"目录"别搞混（词典外置那次改动之后才分开的）：
    -DataFolder  数据文件夹：里面是 dicts\（词典）与 tables\（词频表）两个子目录
    -TableDir    一份**产物目录**：目录里直接就有 meta.json + full\*.vfr，
                 用于指向数据文件夹之外的既有产物（例如仓库里的 data\）
  从前那个把产物目录写进 dataDir 的用法已经不对了：dataDir 现在是数据文件夹。

  注意：应用在运行时会自己读写这个文件，改之前请先退出应用，否则你的改动会被覆盖。

.EXAMPLE
  # 把数据文件夹指到本仓库的 .tmp\my-data（不存在会建出来），并指定语料库目录
  .\tools\dev-settings.ps1 -DataFolder .\.tmp\my-data -CorpusDir "D:\path\to\语料库"

.EXAMPLE
  # 指向仓库里已经生成好的 data\ 产物目录（它会被记成一张"数据文件夹之外的表"）
  .\tools\dev-settings.ps1 -TableDir .\data

.EXAMPLE
  # 显示当前设置
  .\tools\dev-settings.ps1 -Show

.EXAMPLE
  # 删掉设置文件，回到全新状态（数据文件夹里的东西不动）
  .\tools\dev-settings.ps1 -Reset
#>
[CmdletBinding(DefaultParameterSetName = 'Set')]
param(
    [Parameter(ParameterSetName = 'Set')]
    [string]$DataFolder,

    [Parameter(ParameterSetName = 'Set')]
    [string]$TableDir,

    [Parameter(ParameterSetName = 'Set')]
    [string]$CorpusDir,

    [Parameter(ParameterSetName = 'Set')]
    [string]$Hotkey,

    # 已废弃：词典现在必须是数据文件夹 dicts\ 下的 .dict 条目。保留这个参数只为给出一句提示。
    [Parameter(ParameterSetName = 'Set')]
    [string]$UserDict,

    [Parameter(ParameterSetName = 'Show')]
    [switch]$Show,

    [Parameter(ParameterSetName = 'Reset')]
    [switch]$Reset
)

$ErrorActionPreference = 'Stop'
$cfgDir = Join-Path $env:APPDATA 'com.voctier.desktop'
$cfgFile = Join-Path $cfgDir 'settings.json'

if ($Reset) {
    if (Test-Path $cfgFile) {
        Remove-Item $cfgFile -Force
        Write-Host "已删除 $cfgFile（应用下次启动会用默认设置；数据文件夹里的词典/词频表不受影响）"
    } else {
        Write-Host "设置文件本来就不存在：$cfgFile"
    }
    return
}

if ($Show) {
    if (Test-Path $cfgFile) {
        Write-Host "设置文件: $cfgFile`n"
        Get-Content $cfgFile -Raw
    } else {
        Write-Host "设置文件不存在：$cfgFile（应用还没运行过，或已被 -Reset 清掉）"
    }
    return
}

if (-not $DataFolder -and -not $TableDir -and -not $CorpusDir -and -not $Hotkey -and -not $UserDict) {
    Write-Host '什么都没指定。用法见: Get-Help .\tools\dev-settings.ps1 -Full'
    return
}

# 应用没跑时读不到它的默认值，这里手写一份与 Rust 侧 Settings::default() 一致的默认值
$defaults = [ordered]@{
    corpusDir         = $null
    dataDir           = $null
    hotkey            = 'Alt+Q'
    popupWidth        = 520
    popupHeight       = 560
    popupOpacity      = 1.0
    popupAlwaysOnTop  = $true
    popupAutoCloseMs  = 0
    theme             = 'system'
    locale            = 'zh-CN'
    threads           = 0
    hmm               = $false
    keepDigit         = $false
    keepLatin         = $true
    skipSingleChar    = $false
    scanDicts         = $null
    # v1 兼容字段：迁移时会被读走并清空，所以这里恒为 null
    userDict          = $null
    activeTable       = $null
    activeTablePath   = $null
    minCount          = 1
    skipDomainTables  = $false
    enabledTables     = $null
    tierMethod        = 'rank'
    tierWordBounds    = $null
    tierCharBounds    = $null
    tierCoverage      = $null
}

# 现有文件优先，避免把用户已经调好的其它字段冲掉
$settings = $defaults
if (Test-Path $cfgFile) {
    try {
        $existing = Get-Content $cfgFile -Raw | ConvertFrom-Json
        foreach ($k in $defaults.Keys.Clone()) {
            if ($existing.PSObject.Properties.Name -contains $k) { $settings[$k] = $existing.$k }
        }
    } catch {
        Write-Warning "现有设置文件无法解析，将按默认值重写：$($_.Exception.Message)"
    }
}

if ($DataFolder) {
    # 数据文件夹不存在就建出来 —— 它是"我们自己的地盘"，建 dicts\/tables\ 是应用的职责
    $full = if (Test-Path $DataFolder) { (Resolve-Path $DataFolder).Path } else { $null }
    if (-not $full) {
        New-Item -ItemType Directory -Force -Path $DataFolder | Out-Null
        $full = (Resolve-Path $DataFolder).Path
    }
    New-Item -ItemType Directory -Force -Path (Join-Path $full 'dicts') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $full 'tables') | Out-Null
    $settings['dataDir'] = $full
    # 换了数据文件夹，原来那张表多半已经不在这套里了
    $settings['activeTable'] = $null
}

if ($TableDir) {
    if (-not (Test-Path $TableDir)) { throw "产物目录不存在：$TableDir" }
    if (-not (Test-Path (Join-Path $TableDir 'meta.json'))) {
        throw @"
该目录下没有 meta.json，不是 vocfreq 的产物目录：$TableDir
先跑一次：vocfreq scan --corpus <语料库> --dict <词典> --out $TableDir
（词典现在必须显式指定，见 docs\DATA_LAYOUT.md）
"@
    }
    $settings['activeTablePath'] = (Resolve-Path $TableDir).Path
    $settings['activeTable'] = $null
}

if ($CorpusDir) {
    if (-not (Test-Path $CorpusDir)) { throw "语料库目录不存在：$CorpusDir" }
    $settings['corpusDir'] = (Resolve-Path $CorpusDir).Path
}
if ($Hotkey) { $settings['hotkey'] = $Hotkey }

if ($UserDict) {
    Write-Warning @"
-UserDict 已废弃，本次**没有写入**。
自定义词典现在是数据文件夹 dicts\ 下的一个 .dict 条目，在扫描时勾选。
要手动放一份进去的话：
    Copy-Item "$((Resolve-Path $UserDict).Path)" (Join-Path '$($settings['dataDir'] ?? '<数据文件夹>')' 'dicts\我的词典.dict')
（注意扩展名必须是 .dict，内容为 jieba 的「词 词频 词性」）
"@
}

New-Item -ItemType Directory -Force -Path $cfgDir | Out-Null
$settings | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $cfgFile -Encoding UTF8

Write-Host "已写入 $cfgFile`n"
Write-Host ("  dataDir         = {0}" -f $settings['dataDir'])
Write-Host ("  activeTable     = {0}" -f $settings['activeTable'])
Write-Host ("  activeTablePath = {0}" -f $settings['activeTablePath'])
Write-Host ("  corpusDir       = {0}" -f $settings['corpusDir'])
Write-Host ("  hotkey          = {0}" -f $settings['hotkey'])
Write-Host ''
Write-Host '现在启动桌面端就会自动载入这张表（若应用已在运行，需要先退出再重启）。'
