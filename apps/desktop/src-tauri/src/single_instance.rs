//! 自研单实例：命名互斥体。
//!
//! 为什么不用官方 `tauri-plugin-single-instance`：它在 Windows 上是「先建命名互斥体、
//! 再建事件窗口」两步，两者间有一个时间窗。若第二个实例恰好在这个窗口内启动，
//! 它 `FindWindowW` 找不到第一个实例的事件窗口，就**不退出**，直接完整启动 → 双开
//! （且第二个实例的托盘往往没建好，表现为「多一个没有图标的窗口」）。
//!
//! 这里把判定放在 `setup` 开头（任何窗口 / 托盘 / 热键创建之前）：用命名互斥体判断
//! 是否已有实例，是则退出，**绝不创建任何窗口**。
//!
//! 互斥体名用 `Local\`（当前会话命名空间）：同一用户在同一会话内启动才会被拦，
//! 这正是「一次只跑一个」的目标场景；跨会话（如 RDP 双开）不拦截，符合预期。
//!
//! 唤起已有实例的主窗口**不在这里做**：唤起需要 Tauri 的窗口 API（`show_main_window`）
//! 来保持 tao 的窗口可见状态同步。若用 Win32 `ShowWindow/SetForegroundWindow` 直接
//! 戳窗口，会绕过 tao 的可见状态缓存——主窗口被隐藏过再恢复后，之后的 `hide()`
//! 会被 tao 当成「已经隐藏」而忽略，表现为「点关闭不隐藏」。所以本文件只负责
//! 「检测到已有实例」，唤起由调用方（lib.rs 的 setup）用 Tauri API 完成。

use std::ptr;

use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
    System::Threading::CreateMutexW,
};

/// 命名互斥体名。`Local\` = 当前会话命名空间（见模块注释）。
const MUTEX_NAME: &str = "Local\\com.voctier.desktop-single";

/// 在 `setup` 开头调用。
///
/// 返回 `true` = 已有实例在运行，调用方应唤起它的主窗口（用 Tauri API）并退出本进程；
/// 返回 `false` = 本进程是第一个实例，正常继续启动（已持有互斥体）。
pub fn detect_existing_instance() -> bool {
    unsafe {
        let name: Vec<u16> = MUTEX_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // bInitialOwner = TRUE：第一个实例创建时立即持有
        let hmutex = CreateMutexW(ptr::null(), 1, name.as_ptr());
        // GetLastError 必须紧接调用读取，不能被中间的任何系统调用覆盖
        let last_err = GetLastError();
        crate::log_line(
            "startup.log",
            &format!(
                "[单实例] CreateMutexW({MUTEX_NAME}) hmutex={:?} GetLastError={last_err:?} (ERROR_ALREADY_EXISTS={}, is_null={})",
                hmutex,
                last_err == ERROR_ALREADY_EXISTS,
                hmutex.is_null(),
            ),
        );
        if hmutex.is_null() {
            // 创建失败：无法判定，按「第一个实例」继续（不阻断启动）
            return false;
        }
        if last_err == ERROR_ALREADY_EXISTS {
            // 已有实例在跑：唤起它的主窗口（由调用方用 Tauri API 做），然后让调用方退出
            return true;
        }
        // 第一个实例：持有互斥体直到进程结束。
        // 句柄是裸指针（Copy 类型），`let _` 忽略绑定即可——句柄在进程句柄表中，
        // 不会因 Rust 变量丢弃而关闭；进程退出时内核自动释放互斥体。
        crate::log_line("startup.log", "[单实例] 本进程为第一个实例，持有互斥体");
        let _ = hmutex;
        false
    }
}
