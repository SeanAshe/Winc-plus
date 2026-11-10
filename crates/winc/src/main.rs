//! `winc` 可执行文件的入口，程序的实现在 `winc-app` 中。

// 不加时 Windows 每次启动 winc 都会另开一个控制台窗口。
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    winc_app::run();
}
