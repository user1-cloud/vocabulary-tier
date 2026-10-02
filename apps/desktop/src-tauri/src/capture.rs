//! 全局取词：**剪贴板模拟法**。
//!
//! 流程：暂存剪贴板 → 模拟 Ctrl+C → 等剪贴板序列号变化 → 读走选区 → 立刻还原。
//!
//! 为什么不用 UI Automation：UIA 的 TextPattern 只对支持它的程序有效（老旧程序、
//! 游戏、部分 PDF 阅读器拿不到文本），而剪贴板法在浏览器 / Office / PDF 阅读器里
//! 通吃。代价有两个，都已知且可接受：
//!  1. 会短暂占用剪贴板，因此必须尽快还原；
//!  2. 若原剪贴板内容不是文本（图片、文件），`arboard` 无法完整还原，此时我们会
//!     **保留抓到的文本而不是清空剪贴板**，避免把用户的东西弄丢。

use std::time::{Duration, Instant};

/// 取当前前台窗口的选中文本。返回空串表示「当前没有选中内容」。
///
/// `timeout_ms` 是等待目标程序写入剪贴板的上限。太小会在慢程序里丢词，
/// 太大则在「没有选中内容」时会明显卡顿。
pub fn capture_selection(timeout_ms: u64) -> Result<String, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("打开剪贴板失败：{e}"))?;

    // 1) 记下序列号并暂存原内容
    let before = platform::clipboard_seq();
    let saved_text = cb.get_text().ok();

    // 2) 模拟 Ctrl+C
    platform::send_ctrl_c()?;

    // 3) 等目标程序真正写入：序列号变了才说明复制发生了。
    //    不能只靠 sleep 固定时长，不同程序差异很大。
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut changed = false;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(8));
        if platform::clipboard_seq() != before {
            changed = true;
            break;
        }
    }

    // 4) 读走内容。有程序是「先改序列号再填数据」，所以要重试几次。
    let mut captured = String::new();
    if changed {
        for _ in 0..8 {
            match cb.get_text() {
                Ok(t) => {
                    captured = t;
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    // 5) 还原。原本是文本才还原；原本不是文本就保留抓到的文本，绝不清空。
    if let Some(prev) = saved_text {
        for _ in 0..6 {
            if cb.set_text(prev.clone()).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    Ok(captured)
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_C, VK_CONTROL,
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

    pub fn send_ctrl_c() -> Result<(), String> {
        let inputs = [
            key(VK_CONTROL, false),
            key(VK_C, false),
            key(VK_C, true),
            key(VK_CONTROL, true),
        ];
        let n = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if n != inputs.len() as u32 {
            return Err(format!(
                "发送 Ctrl+C 失败：SendInput 只发出 {n}/{} 个事件（可能被 UIPI 拦截，\
                 试试以管理员身份运行 VocTier）",
                inputs.len()
            ));
        }
        Ok(())
    }

    pub fn clipboard_seq() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
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
}
