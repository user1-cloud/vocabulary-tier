<#
.SYNOPSIS
  开发和测试辅助：管理 VocTier 桌面端的设置文件。

.DESCRIPTION
  桌面端把设置存在 %APPDATA%\com.voctier.desktop\settings.json。
  测试时经常需要「直接把产物目录指到已生成好的 data\」，而不是每次都在界面里点一遍，
  或者设置被填坏之后一键复位。这个脚本就干这两件事。

  注意：应用在运行时会自己读写这个文件，改之前请先退出应用，否则你的改动会被覆盖。

.EXAMPLE
  # 把产物目录指向仓库里的 data\，并指定语料库目录
  .\tools\dev-settings.ps1 -DataDir .\data -CorpusDir "D:\path\to\语料库"

.EXAMPLE
  # 只指向产物目录
  .\tools\dev-settings.ps1 -DataDir .\data

.EXAMPLE
  # 显示当前设置
  .\tools\dev-settings.ps1 -Show

.EXAMPLE
  # 删掉设置文件，回到全新状态
  .\tools\dev-settings.ps1 -Reset
#>
[CmdletBinding(DefaultParameterSetName = 'Set')]
param(
    [Parameter(ParameterSetName = 'Set')]
    [string]$DataDir,

    [Parameter(ParameterSetName = 'Set')]
    [string]$CorpusDir,

    [Parameter(ParameterSetName = 'Set')]
    [string]$Hotkey,

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
        Write-Host "已删除 $cfgFile（应用下次启动会用默认设置）"
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

if (-not $DataDir -and -not $CorpusDir -and -not $Hotkey -and -not $UserDict) {
    Write-Host '什么都没指定。用法见: Get-Help .\tools\dev-settings.ps1 -Full'
    return
}

# 应用没跑时读不到它的默认值，这里手写一份与 Rust 侧 Settings::default() 一致的默认值
$defaults = [ordered]@{
    corpusDir         = $null
    dataDir           = $null
    hotkey            = 'Alt+Q'
    popupWidth        = 460
    popupHeight       = 340
    popupOpacity      = 1.0
    popupAlwaysOnTop  = $true
    popupAutoCloseMs  = 0
    theme             = 'system'
    threads           = 0
    hmm               = $false
    keepDigit         = $false
    keepLatin         = $true
    skipSingleChar    = $false
    userDict          = $null
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

if ($DataDir) {
    if (-not (Test-Path $DataDir)) { throw "产物目录不存在：$DataDir" }
    if (-not (Test-Path (Join-Path $DataDir 'meta.json'))) {
        throw "该目录下没有 meta.json，不是 vocfreq 的产物目录：$DataDir`n先跑一次：vocfreq scan --corpus <语料库> --out $DataDir"
    }
    $settings['dataDir'] = (Resolve-Path $DataDir).Path
}
if ($CorpusDir) {
    if (-not (Test-Path $CorpusDir)) { throw "语料库目录不存在：$CorpusDir" }
    $settings['corpusDir'] = (Resolve-Path $CorpusDir).Path
}
if ($Hotkey)    { $settings['hotkey'] = $Hotkey }
if ($UserDict)  { $settings['userDict'] = (Resolve-Path $UserDict).Path }

New-Item -ItemType Directory -Force -Path $cfgDir | Out-Null
$settings | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $cfgFile -Encoding UTF8

Write-Host "已写入 $cfgFile`n"
Write-Host ("  dataDir   = {0}" -f $settings['dataDir'])
Write-Host ("  corpusDir = {0}" -f $settings['corpusDir'])
Write-Host ("  hotkey    = {0}" -f $settings['hotkey'])
Write-Host ''
Write-Host '现在启动桌面端就会自动载入该词频表（若应用已在运行，需要先退出再重启）。'
