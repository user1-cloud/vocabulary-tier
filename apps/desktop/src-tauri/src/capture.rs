//! 全局取词。两条路，**优先 UI Automation，失败再用剪贴板模拟法**。
//!
//! 1. **UIA（首选）**：读焦点元素的 TextPattern 选区。不合成按键、不碰剪贴板，
//!    因此没有焦点/修饰键/UIPI/剪贴板占用这一整类问题。
//! 2. **剪贴板模拟法（兜底）**：暂存剪贴板 → 模拟 Ctrl+C → 等序列号变化 → 读走 → 还原。
//!    在 UIA 拿不到文本时用（老旧程序、自绘控件、部分终端不支持 TextPattern）。
//!
//! 为什么必须加第 1 条：实测 `SendInput` 注入的 Ctrl+C 在某些环境里会被**静默丢弃**
//! —— SendInput 返回成功、前台窗口与修饰键状态都正常，剪贴板却毫无变化；同一台机器上
//! pwsh 注入同样的事件却有效，且与应用内的调用线程、启动方式都无关。机制未查明，
//! 但 UIA 走的是完全不同的通路，因此不受影响。
//!
//! 剪贴板法的已知代价（都能接受）：
//!  1. 会短暂占用剪贴板，所以必须尽快还原；
//!  2. 若原内容不是文本（图片、文件），`arboard` 无法完整还原，此时我们
//!     **保留抓到的文本而不是清空剪贴板**，避免把用户的东西弄丢。

use std::time::{Duration, Instant};

/// 取词过程的日志。这条链路全是「外部环境相关」的失败点（目标程序、UIPI、
/// 剪贴板占用、热键焦点），**必须逐步留痕**，否则用户只说「小窗是空的」，
/// 完全无法判断是没取到、取错了、还是传丢了。
fn note(msg: impl AsRef<str>) {
    crate::log_line("startup.log", &format!("[取词] {}", msg.as_ref()));
}

/// UIA 控件类型的可读名。
///
/// 为什么需要：UIA 返回的是一串数字（`ControlType=50032`），而「焦点落在**窗口**上」
/// 与「焦点落在**文本框**里但没选中」是完全不同的两件事，用户看到数字无从判断。
/// 实测最常见的一种失败就是前者：用户以为选好了词，其实按热键时焦点在浏览器窗口上。
fn control_type_name(id: i32) -> &'static str {
    match id {
        50000 => "按钮",
        50003 => "下拉框",
        50004 => "文本框",
        50005 => "链接",
        50007 => "列表项",
        50008 => "列表",
        50013 => "单选框",
        50020 => "文本",
        50025 => "自定义控件",
        50026 => "分组",
        50030 => "文档",
        50032 => "窗口",
        50033 => "面板",
        50034 => "标题栏",
        50036 => "表格",
        50037 => "标题栏",
        _ => "未知控件",
    }
}

/// 用 UI Automation **直接读取**焦点元素的选中文本。
///
/// 返回 `(文本, 给用户看的诊断)`：
/// * 成功 → `(Some(text), "")`
/// * 失败 → `(None, 一句能指导下一步的话)`
///
/// 区分失败原因是关键——「没有选中文字」和「焦点压根不在文本框里」对用户来说
/// 需要做完全不同的事，而这两者以前在界面上长得一模一样。
///
/// 两个容易踩的点：
/// * COM 必须在**调用线程**上初始化，且必须用 **MTA**（STA 下跨进程取模式会失败）。
/// * Chromium 系浏览器的无障碍树是**按需**启用的，第一次查询常常只能拿到顶层窗口
///   （`Class="Chrome_WidgetWin_1"`、`ControlType=窗口`）。这不是本程序的 bug，
///   换成 TextGO 等同类工具也一样。
/// 把元素名截断，避免把一整段正文塞进小窗。
///
/// 实测踩过：在浏览器里选中一段文字时，焦点元素的 `Name` **就是那段被选中的文字**，
/// 于是失败提示变成了「焦点在『我想要制作一个划句子自动拆分词汇…』这个面板上」——
/// 一整段话，挤满小窗，反而看不出重点。
fn short_name(name: &str) -> String {
    let trimmed = name.trim();
    let mut out: String = trimmed.chars().take(16).collect();
    if trimmed.chars().count() > 16 {
        out.push('…');
    }
    out
}

#[cfg(windows)]
fn uia_selection() -> (Option<String>, String) {
    use windows::core::Interface;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
        UIA_TextPatternId,
    };

    unsafe {
        // 已初始化过会返回 S_FALSE 或 RPC_E_CHANGED_MODE，都无所谓
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let automation: IUIAutomation =
            match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                Ok(a) => a,
                Err(e) => {
                    let m = format!("无障碍接口不可用（创建 IUIAutomation 失败：{e}）");
                    note(&m);
                    return (None, m);
                }
            };
        let element = match automation.GetFocusedElement() {
            Ok(el) => el,
            Err(e) => {
                let m = format!("取不到焦点元素（{e}）");
                note(&m);
                return (None, m);
            }
        };

        // 从焦点元素开始，**向上逐级**找第一个支持 TextPattern 的元素。
        //
        // 为什么必须往上找：浏览器里被选中的文字挂在 **Document** 上，而
        // `GetFocusedElement()` 往往只返回页面里的某个 **Pane**
        // （实测其 Name 就是那段被选中的文字）。只问焦点元素必然一无所获——
        // 这也是同类开源工具（TextGO）在浏览器里取不到的原因。
        //
        // 只走 ControlView（用户可见元素），最多 8 级，避免在深层树里空转。
        let walker = automation.ControlViewWalker().ok();
        let mut current: IUIAutomationElement = element.clone();
        let mut text_pattern: Option<IUIAutomationTextPattern> = None;
        let mut where_found = String::new();
        let mut focus_note = String::new();

        for depth in 0..8u32 {
            let cname = current.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let cclass = current
                .CurrentClassName()
                .map(|b| b.to_string())
                .unwrap_or_default();
            let ctype = current.CurrentControlType().map(|c| c.0).unwrap_or(-1);
            let kind = control_type_name(ctype);

            match current
                .GetCurrentPattern(UIA_TextPatternId)
                .and_then(|p| p.cast::<IUIAutomationTextPattern>())
            {
                Ok(p) => {
                    where_found = if depth == 0 {
                        "焦点元素".to_string()
                    } else {
                        format!("焦点元素向上第 {depth} 级的 {kind}")
                    };
                    text_pattern = Some(p);
                    break;
                }
                Err(e) => {
                    note(format!(
                        "UIA 第 {depth} 级 Class={cclass:?} ControlType={ctype}({kind}) 无 TextPattern（{e}）"
                    ));
                    // 只把**焦点自身**的情况做成给用户看的说明；祖先不支持是常态。
                    if depth == 0 {
                        let named = if cname.trim().is_empty() {
                            "目标程序".to_string()
                        } else {
                            format!("「{}」", short_name(&cname))
                        };
                        focus_note = match ctype {
                            // 窗口 / 面板：最常见、也最容易被误解的一种失败——
                            // 用户以为选好了词，其实焦点根本不在任何文本框或页面文档上。
                            50032 | 50033 => format!(
                                "焦点在 {named} 这个{kind}上，而不是某个文本框或页面文档里。\
                                 请先在目标程序里点一下输入区域、选中文字，再按热键。"
                            ),
                            _ => format!("焦点控件（{kind}，Class={cclass}）不支持直接读取选区。"),
                        };
                    }
                }
            }

            let parent = match &walker {
                Some(w) => w.GetParentElement(&current).ok(),
                None => None,
            };
            match parent {
                Some(p) => current = p,
                None => break,
            }
        }

        let Some(pattern) = text_pattern else {
            // 整条祖先链都没有 TextPattern
            let msg = if focus_note.is_empty() {
                "焦点所在的可访问性树里找不到可读的文本控件。".to_string()
            } else {
                focus_note
            };
            return (None, msg);
        };

        let ranges = match pattern.GetSelection() {
            Ok(r) => r,
            Err(e) => {
                note(format!("UIA 在 {where_found} 取到 TextPattern，但取选区失败（{e}）"));
                return (None, format!("取不到选区（{e}）"));
            }
        };

        let count = ranges.Length().unwrap_or(0);
        note(format!(
            "UIA 在 {where_found} 取到 TextPattern，选中 {count} 段"
        ));
        if count == 0 {
            return (
                None,
                "可读的文本控件找到了，但当前选中 0 段——没有选中文字。".to_string(),
            );
        }

        let mut out = String::new();
        for i in 0..count {
            if let Ok(range) = ranges.GetElement(i) {
                if let Ok(text) = range.GetText(-1) {
                    out.push_str(&text.to_string());
                }
            }
        }
        (Some(out), String::new())
    }
}

#[cfg(not(windows))]
fn uia_selection() -> (Option<String>, String) {
    (None, "非 Windows 平台".into())
}

/// 取词结果。
pub struct Capture {
    /// 抓到的文本；空串表示当前没有选中内容。
    pub text: String,
    /// 取不到时给**用户看**的简短原因（取到内容时为空串）。
    ///
    /// 为什么要有这个：取词失败的原因全在环境里（焦点、UIPI、剪贴板占用、
    /// 目标程序不响应），而日志文件用户未必找得到。把原因直接显示在小窗里，
    /// 既省掉一轮「发日志给我」的往返，也让用户自己知道该怎么办。
    pub reason: String,
}

/// 取当前前台窗口的选中文本。
///
/// `timeout_ms` 是剪贴板法里等待目标程序写入的上限。太小会在慢程序里丢词，
/// 太大则在「没有选中内容」时会明显卡顿。参考同类工具（TextGO 默认 1000ms）。
pub fn capture_selection(timeout_ms: u64) -> Result<Capture, String> {
    // 第一路：UIA 直接读。不合成按键、不碰剪贴板，最干净也最快。
    let (uia_text, uia_note) = uia_selection();
    if let Some(text) = uia_text {
        if !text.trim().is_empty() {
            note(format!(
                "UIA 读取成功：{} 字符（未合成按键、未触碰剪贴板）",
                text.chars().count()
            ));
            return Ok(Capture {
                text,
                reason: String::new(),
            });
        }
    }
    note(format!("改用剪贴板法。UIA 未取到的原因：{uia_note}"));

    let mut via_clipboard = capture_via_clipboard(timeout_ms)?;
    if !via_clipboard.text.trim().is_empty() {
        return Ok(via_clipboard);
    }

    // 第三路兜底：直接用剪贴板里**已有**的内容。
    //
    // 这一条不注入任何按键、不读任何无障碍接口，因此不受「合成输入被丢弃」
    // 「UIA 被拦」「权限级别不一致」「安全软件 hook」这一整类问题影响。
    // 用户只要自己按一次 Ctrl+C，剪贴板里就是他要查的文字。
    //
    // 行业里也是这么兜的：「复制即翻译」类工具干脆只做这一步，完全不碰系统通路。
    if let Ok(mut cb) = arboard::Clipboard::new() {
        if let Ok(existing) = cb.get_text() {
            if !existing.trim().is_empty() {
                note(format!(
                    "取词失败，改用剪贴板里已有的内容：{} 字符",
                    existing.chars().count()
                ));
                return Ok(Capture {
                    text: existing,
                    // 必须说明来源：否则用户会以为这是自己刚选中的文字，
                    // 而实际上是剪贴板里可能放了很久的旧内容。
                    reason: "取词未成功，下面分析的是**剪贴板里已有的内容**。\
                             要取选中的文字，请先自己按一次 Ctrl+C，再按热键。"
                        .to_string(),
                });
            }
        }
    }

    // 三条路都空：把**最有指导性**的说明放到界面最前面。
    // UIA 的诊断能区分「焦点不在文本框里」与「没选中文字」，比笼统的
    // 「没取到内容」有用得多——前者要用户去点一下输入区，后者要用户去选字。
    if !uia_note.is_empty() {
        via_clipboard.reason = format!(
            "{uia_note}程序模拟的 Ctrl+C 也没有被响应。\
             也可以先自己按一次 Ctrl+C 再按热键，或直接在下面手输要查的词。"
        );
    }
    Ok(via_clipboard)
}

/// 剪贴板模拟法：暂存 → 模拟 Ctrl+C → 等序列号变化 → 读走 → 还原。
fn capture_via_clipboard(timeout_ms: u64) -> Result<Capture, String> {
    let before = platform::clipboard_seq();

    // 1) 先把原内容读出来保存。
    //
    // ⚠ 这里**刻意把 `Clipboard` 限制在一个块里**，读完立刻 drop，之后再注入按键。
    // 原因是 Windows 剪贴板同一时刻只允许一个进程打开：只要有进程占着不放，
    // 目标程序的复制就会**静默失败或延迟**（序列号不变），表现和「没有选中内容」
    // 一模一样。实测验证：主动 OpenClipboard 不释放时注入 Ctrl+C，剪贴板纹丝不动，
    // 释放后内容才写进去。
    //
    // arboard 本身是 RAII 开闭的，但把它限制成短生命周期是零成本的保险——
    // 这类故障完全无声，值得用显式的生命周期把它排除掉。
    let saved_text = {
        let mut cb = arboard::Clipboard::new().map_err(|e| {
            let m = format!("打开剪贴板失败：{e}");
            note(&m);
            m
        })?;
        cb.get_text().ok()
    };

    note(format!(
        "起点：剪贴板序列号={before}，原有文本={}；Ctrl+C 将发给 → {}",
        match &saved_text {
            Some(t) => format!("{} 字符", t.chars().count()),
            None => "无（空或非文本）".into(),
        },
        platform::foreground_desc()
    ));
    note(format!(
        "剪贴板体检：{}；本进程现在能否打开={}",
        platform::clipboard_holder(),
        platform::can_open_clipboard()
    ));

    // 2) 分两轮尝试「注入 Ctrl+C → 等剪贴板变化」。
    //    第一轮没动静就再发一次：有些程序（重负载的浏览器标签页）对第一次
    //    按键响应很慢，一次不成就判定「没选中」会频繁误报。
    let t0 = Instant::now();
    let mut changed = false;
    let first = (timeout_ms / 2).max(80);
    let second = timeout_ms.saturating_sub(first).max(80);

    for round in 0..2 {
        // 注入**前**再确认一次前台窗口与修饰键物理状态。
        // 起点那一次检查可能已经过时：从收到热键到真正注入之间，焦点或按键状态
        // 都可能变化，而这两者正是「Ctrl+C 变成 Ctrl+Alt+C」或「发给了错误的窗口」
        // 的唯二原因。不在这里量一次，就只能在故障发生后猜。
        note(format!(
            "第 {} 轮注入前：前台={} 修饰键={}",
            round + 1,
            platform::foreground_desc(),
            platform::modifier_state()
        ));

        if let Err(e) = platform::send_ctrl_c() {
            note(format!("第 {} 轮发送 Ctrl+C 失败：{e}", round + 1));
            if round > 0 {
                break;
            }
            continue;
        }

        let budget = if round == 0 { first } else { second };
        let deadline = Instant::now() + Duration::from_millis(budget);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(8));
            if platform::clipboard_seq() != before {
                changed = true;
                break;
            }
        }
        note(format!(
            "第 {} 轮结束：累计等待 {}ms，剪贴板{}",
            round + 1,
            t0.elapsed().as_millis(),
            if changed {
                format!("已变化（序列号 {}）", platform::clipboard_seq())
            } else {
                "未变化".into()
            }
        ));
        if changed {
            break;
        }
    }
    let waited = t0.elapsed().as_millis();
    let mut reason = String::new();
    if !changed {
        // 【临时诊断】如果设了 VOCTIER_DEBUG_INJECT，就额外注入一个普通字符键 'X'。
        // 目的：把「注入完全没效果」和「注入有效但 Ctrl+C 组合不对」这两类原因分开。
        // 目标文本框里出现了 X，说明注入链路是通的。
        if std::env::var_os("VOCTIER_DEBUG_INJECT").is_some() {
            match platform::type_probe_char() {
                Ok(()) => note("诊断：已注入探测字符 'X'（看目标文本框里有没有出现它）"),
                Err(e) => note(format!("诊断：注入探测字符失败：{e}")),
            }
        }
        note(format!(
            "两轮共等待 {waited}ms 后剪贴板序列号仍未变化 → 判定为「没有选中内容」。\
             请对照上面「注入前」那两行：若前台不是目标程序，是焦点被抢；\
             若修饰键显示 Alt=按下，则是 Ctrl+C 被解释成了 Ctrl+Alt+C。"
        ));
        // 给用户看的说法要**短**（小窗只有 460px 宽），并把排查顺序按「最可能」排好。
        // 详细的「发给谁、修饰键状态、UIA 焦点元素是谁」都留在日志里。
        reason = "没有取到选中内容：直接读取与模拟复制两种方式都没拿到文本。\
                  请先确认确实选中了文字；若目标程序以管理员身份运行，\
                  请也用管理员身份启动本程序。两者权限一致仍失败时，\
                  多半是安全软件（杀毒 / 电脑管家 / 输入法防护）拦截了本程序的\
                  按键注入与无障碍读取，把它加入白名单再试。\
                  也可以直接在下面手输要查的词。"
            .to_string();
    }

    // 4) 读走内容。有程序是「先改序列号再填数据」，所以要重试几次。
    //    同样用短生命周期句柄，读完立刻释放。
    let mut captured = String::new();
    if changed {
        for attempt in 0..8 {
            let got = match arboard::Clipboard::new() {
                Ok(mut cb) => cb.get_text().ok(),
                Err(_) => None,
            };
            match got {
                Some(t) => {
                    captured = t;
                    break;
                }
                None => {
                    if attempt == 7 {
                        note("序列号已变化，但连续 8 次读取剪贴板都失败");
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        note(format!(
            "成功：等待 {waited}ms，读到 {} 字符（序列号 {before} → {}）",
            captured.chars().count(),
            platform::clipboard_seq()
        ));
    }

    // 5) 还原。原本是文本才还原；原本不是文本就保留抓到的文本，绝不清空。
    if let Some(prev) = saved_text {
        for _ in 0..6 {
            let ok = match arboard::Clipboard::new() {
                Ok(mut cb) => cb.set_text(prev.clone()).is_ok(),
                Err(_) => false,
            };
            if ok {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    Ok(Capture {
        text: captured,
        reason,
    })
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardSequenceNumber, GetOpenClipboardWindow, OpenClipboard,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_C,
        VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, SetForegroundWindow,
    };

    /// 构造一个键盘事件。用 `zeroed` + 逐字段赋值，避免直接书写联合体类型名。
    fn key(vk: u16, up: bool) -> INPUT {
        let mut input: INPUT = unsafe { std::mem::zeroed() };
        input.r#type = INPUT_KEYBOARD;
        let ki = KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            dwExtraInfo: 0,
        };
        input.Anonymous.ki = ki;
        input
    }

    /// 注入一个普通字符键 'X'（仅用于诊断注入链路是否通）。
    pub fn type_probe_char() -> Result<(), String> {
        const VK_X: u16 = 0x58;
        send(&[key(VK_X, false), key(VK_X, true)])
    }

    /// 送一批键盘事件。
    fn send(inputs: &[INPUT]) -> Result<(), String> {
        let n = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        // 首次调用时记一次「体检」：cbSize 不对时 SendInput 会直接失败，
        // 而「事件被接受却毫无效果」这种只能靠 size 与 GetLastError 排除。
        static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            super::note(format!(
                "SendInput 体检：size_of::<INPUT>()={}（应为 40），本批 {} 个事件，返回 {n}，GetLastError={err}",
                std::mem::size_of::<INPUT>(),
                inputs.len()
            ));
        }
        if n != inputs.len() as u32 {
            return Err(format!(
                "SendInput 只发出 {n}/{} 个事件（可能被 UIPI 拦截：\
                 目标程序以管理员身份运行而本程序不是，试试也用管理员启动 VocTier）",
                inputs.len()
            ));
        }
        Ok(())
    }

    /// 注入一次 Ctrl+C。
    ///
    /// ⚠ **关键：先把可能还按着的修饰键松开。**
    ///
    /// 取词组件的热键是 `Alt+Q`。用户按下热键的那一刻，**Alt 仍然处于按下状态**；
    /// 如果这时直接注入 `Ctrl+C`，目标程序收到的是 `Ctrl+Alt+C` —— 那不是「复制」，
    /// 于是剪贴板不会有任何变化，最终表现为「按了热键但小窗是空的」。
    /// 这是热键取词类工具最经典的一个坑，而且它**不会报任何错**：SendInput 照样
    /// 返回成功，只是按下的组合键不是你想要的那个。
    ///
    /// 所以这里分两批发：
    ///   1. 把所有修饰键抬一遍（对本来就没按下的键发 KEYUP 是无害的）；
    ///   2. 等 30ms 让系统处理完抬起事件，再发干净的 Ctrl+C。
    pub fn send_ctrl_c() -> Result<(), String> {
        let releases = [
            key(VK_MENU, true),    // Alt —— 元凶就是这个
            key(VK_SHIFT, true),   // Shift
            key(VK_LWIN, true),    // 左 Win
            key(VK_RWIN, true),    // 右 Win
            key(VK_CONTROL, true), // Ctrl 也先抬一次，保证状态干净
        ];
        send(&releases)?;
        std::thread::sleep(std::time::Duration::from_millis(30));

        let copy = [
            key(VK_CONTROL, false),
            key(VK_C, false),
            key(VK_C, true),
            key(VK_CONTROL, true),
        ];
        send(&copy)
    }

    pub fn clipboard_seq() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    /// 当前前台窗口的句柄。
    pub fn foreground_hwnd() -> isize {
        unsafe { GetForegroundWindow() as isize }
    }

    /// 把焦点交还给某个窗口（用于隐藏小窗后归还焦点）。
    pub fn set_foreground(hwnd: isize) {
        if hwnd != 0 {
            unsafe {
                SetForegroundWindow(hwnd as HWND);
            }
        }
    }

    /// 谁正占着剪贴板。
    ///
    /// Windows 剪贴板同一时刻只能被一个进程打开。只要有进程占着不放，
    /// 目标程序的复制就会**静默失败或延迟**（序列号不变），症状与「没有选中内容」
    /// 完全一致，极难区分。实测：主动 OpenClipboard 不释放时注入 Ctrl+C，
    /// 剪贴板纹丝不动，释放后内容才写进去。所以把「是谁占着」记下来。
    pub fn clipboard_holder() -> String {
        unsafe {
            let h = GetOpenClipboardWindow();
            if h.is_null() {
                return "剪贴板无人持有".into();
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, &mut pid);
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            let title = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
            format!("⚠ 剪贴板被 pid={pid} 的窗口 {title:?} 占着")
        }
    }

    /// 本进程此刻能否打开剪贴板（能打开就说明没有别人占着）。
    pub fn can_open_clipboard() -> bool {
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                CloseClipboard();
                true
            } else {
                false
            }
        }
    }

    /// 各修饰键当前是否**物理**按下（不是注入出来的状态）。
    ///
    /// 这是排查「Ctrl+C 被解释成 Ctrl+Alt+C」的关键：如果用户按住 Alt 触发热键，
    /// 而注入时 Alt 还没松开，目标程序收到的就是 Ctrl+Alt+C —— 那不是复制。
    pub fn modifier_state() -> String {
        let down = |vk: i32| unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 };
        let f = |name: &str, vk: i32| {
            if down(vk) {
                format!("{name}=按下 ")
            } else {
                format!("{name}=松开 ")
            }
        };
        format!(
            "{}{}{}{}",
            f("Alt", VK_MENU as i32),
            f("Shift", VK_SHIFT as i32),
            f("Ctrl", VK_CONTROL as i32),
            f("Win", VK_LWIN as i32),
        )
    }

    /// 前台窗口的描述（进程号 + 标题）。
    ///
    /// 这是排查取词失败最关键的一条信息：`Ctrl+C` 是发给**当前前台窗口**的，
    /// 所以只要把它是谁记下来，就能立刻区分两种情况——
    ///   * 标题是目标程序 → 按键发出去了，问题在目标程序没响应（UIPI / 不支持复制）
    ///   * 标题是本应用自己的窗口 → **焦点被我们抢走了**，那次 Ctrl+C 复制的是我们自己
    pub fn foreground_desc() -> String {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return "<无前台窗口>".into();
            }
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            let title = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mine = if pid == std::process::id() { "（是本应用自己！）" } else { "" };
            format!("pid={pid}{mine} 标题={title:?}")
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn send_ctrl_c() -> Result<(), String> {
        Err("全局取词目前只实现了 Windows 端".into())
    }
    pub fn clipboard_seq() -> u32 {
        0
    }
    pub fn foreground_hwnd() -> isize {
        0
    }
    pub fn set_foreground(_hwnd: isize) {}
    pub fn foreground_desc() -> String {
        "<非 Windows 平台>".into()
    }
    pub fn modifier_state() -> String {
        "<非 Windows 平台>".into()
    }
    pub fn clipboard_holder() -> String {
        "<非 Windows 平台>".into()
    }
    pub fn can_open_clipboard() -> bool {
        false
    }
    pub fn type_probe_char() -> Result<(), String> {
        Err("非 Windows 平台".into())
    }
}

/// 当前前台窗口句柄（用于隐藏小窗后归还焦点）。
pub fn foreground_hwnd() -> isize {
    platform::foreground_hwnd()
}

/// 把焦点交还给指定窗口。
pub fn set_foreground(hwnd: isize) {
    platform::set_foreground(hwnd)
}
