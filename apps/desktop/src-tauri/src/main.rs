// Windows 发布构建时不要弹出额外的控制台窗口。
// release 下由 build.rs 生成的资源脚本会设置子系统，这里只做兜底。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    voctier_desktop_lib::run()
}
