; ============================================================================
; VocTier 安装钩子：把预置词典与预置词频表直接写进**用户数据目录**，
; 而不是释放到 $INSTDIR。
;
; 为什么不用 bundle.resources：那条路的语义就是"释放到 $INSTDIR"，而
; Program Files 下普通用户不可写、用户也就删不掉那份预置内容 —— 可预置内容
; 与用户自建内容**没有本质区别**，用户必须能删能换。所以走安装钩子直接落到
; %LOCALAPPDATA%\com.voctier.desktop\data\ 下，$INSTDIR 里一个字节都不留。
;
; 本文件被 installer.nsi 在**顶层** !include（tauri-bundler installer.nsi 的
; `{{#if installer_hooks}}!include "{{installer_hooks}}"{{/if}}`），所以：
;   ⚠ 顶层只能放 !define / !macro 这类预处理指令。
;     裸写 File / SetOutPath / CreateDirectory 是语法错误 —— 它们必须在
;     Section 或 Function 内。钩子宏体会在 Section 内展开，所以放进宏里合法。
;
; 已被实测确认的两个坑（见 tools/prepare-seed.ps1 的说明）：
;
; 1. **${__FILEDIR__} 在宏体里指向 installer.nsi 的目录**（
;    target\<profile>\nsis\<arch>，每次构建还会被清空），而不是本文件所在目录。
;    所以资产根路径必须在**本文件顶层**固化成普通 !define。
;
; 2. **installMode = perMachine 时 $LOCALAPPDATA 会指到 ProgramData**
;    （utils.nsh 的 SetContext 会 SetShellVarContext all）。钩子里必须显式
;    SetShellVarContext current 才落到发起安装的那个用户。
; ============================================================================

; --- 资产根路径 -------------------------------------------------------------
; 这一行在**本文件被读取时**求值，此时 ${__FILEDIR__} = 本文件所在目录
; （<src-tauri>\nsis）。往上一层是 src-tauri，再往里就是 seed\。
; 千万别把这段搬进宏体 —— 那样 ${__FILEDIR__} 会变成 installer.nsi 的目录。
!define VOCTIER_SEED_ROOT "${__FILEDIR__}\..\seed"

; --- 用户数据目录 -----------------------------------------------------------
; 必须与 Rust 侧 app_local_data_dir() 对齐，也就是
; %LOCALAPPDATA%\<identifier>；identifier 在 NSIS 模板里叫 ${BUNDLEID}
; （注意没有 ${IDENTIFIER} 这个名字，${PRODUCTNAME} 是 "VocTier" 不是标识符）。
; 末尾的 data\ 也要对齐（见 lib.rs 的 default_data_dir 注释：不套这层，
; 用户会在数据文件夹里看到 WebView2 的 EBWebView 缓存）。
!define VOCTIER_DATA_DIR "$LOCALAPPDATA\${BUNDLEID}\data"

; --- 安装 ---------------------------------------------------------------
; 用 POSTINSTALL 而不是 PREINSTALL：模板里 PREINSTALL 跑在
; CheckIfAppIsRunning **之前**，应用还在运行时就往数据目录写文件，
; 可能撞上正被 mmap 的 .vfr；POSTINSTALL 在那道检查之后。
!macro NSIS_HOOK_POSTINSTALL
  ; perMachine 模式下上面说的上下文问题；currentUser 下这行是幂等的。
  SetShellVarContext current

  CreateDirectory "${VOCTIER_DATA_DIR}"
  CreateDirectory "${VOCTIER_DATA_DIR}\dicts"
  CreateDirectory "${VOCTIER_DATA_DIR}\tables"

  ; 词典：整目录铺进去。用 *.* 而不是只给目录名 ——
  ; `File /r "<dir>"` 会多套一层同名目录，`File /r "<dir>\*.*"` 才是把
  ; 目录**内容**铺进 $OUTDIR。用通配符还有个好处：这里不出现中文文件名，
  ; 少一个编码相关的失败面。
  SetOutPath "${VOCTIER_DATA_DIR}\dicts"
  File /r "${VOCTIER_SEED_ROOT}\dicts\*.*"

  ; 词频表：同理。里面是 <表名>\meta.json + <表名>\full\*.vfr，以及
  ; <表名>.origin（来源标记，让应用知道这是预置的而只是"显示成预置"，
  ; 不附加任何权限限制）。
  SetOutPath "${VOCTIER_DATA_DIR}\tables"
  File /r "${VOCTIER_SEED_ROOT}\tables\*.*"

  ; 还原 $OUTDIR：后续模板代码还会往 $INSTDIR 写东西（卸载器等），
  ; 不还原会让它们落到数据目录里。
  SetOutPath "$INSTDIR"
!macroend

; --- 卸载 ---------------------------------------------------------------
; 刻意**什么都不做**。模板默认就会问用户"是否删除应用数据"，默认不勾选；
; 勾了会整个删掉 %LOCALAPPDATA%\<identifier>。这里不再额外删任何东西 ——
; 预置内容装完就是用户的东西，没有"这是我们的文件所以卸载要清掉"这一说。
;
; !macro NSIS_HOOK_POSTUNINSTALL
; !macroend
