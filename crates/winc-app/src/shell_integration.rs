//! 资源管理器的右键菜单和文件类型的登记（Windows），供设置中的开关读取和修改。
//!
//! 写入的注册表项与安装程序相同（`release/windows/installer.nsi`），都在当前用户之下
//! （`HKCU`），便携版同样可用。MSI 安装的写在全机之下，没有管理员权限改不了：关掉右键菜单时
//! 在当前用户之下写 `LegacyDisable` 把它盖住，打开时去掉。

#[cfg(any(windows, test))]
mod registry;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

/// 设置中「系统集成」一组的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Integration {
    /// 资源管理器的右键菜单是否显示。
    pub menu: bool,
    /// Winc 是否列在各文件类型的候选程序中。
    pub types: bool,
}

#[cfg(test)]
thread_local! {
    /// 本线程的测试是否把「系统集成」一组列出来（见 [`list_in_tests`]）。
    static LISTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 让本线程的测试列出「系统集成」一组及其全部各项：在其他平台上截图用。
#[cfg(test)]
pub(crate) fn list_in_tests(on: bool) {
    LISTED.set(on);
}

/// 本线程的测试是否要求列出这一组；不是测试时为否。
fn listed_in_tests() -> bool {
    #[cfg(test)]
    return LISTED.get();
    #[cfg(not(test))]
    false
}

/// 设置中是否列出「系统集成」一组：只在 Windows 上。
pub(crate) fn available() -> bool {
    cfg!(windows) || listed_in_tests()
}

/// 设置中是否列出「关联文件类型」：由安装程序装在当前用户之下时才有各类型的登记可列。
pub(crate) fn types_available() -> bool {
    #[cfg(windows)]
    return windows::per_user() || listed_in_tests();
    #[cfg(not(windows))]
    listed_in_tests()
}

/// 读出右键菜单和文件类型此刻的状态；其他平台上都是关。
pub(crate) fn read() -> Integration {
    #[cfg(windows)]
    return windows::read();
    #[cfg(not(windows))]
    Integration::default()
}

/// 打开或关闭右键菜单；其他平台上什么也不做。
pub(crate) fn show_menu(on: bool) {
    #[cfg(windows)]
    windows::show_menu(on);
    #[cfg(not(windows))]
    let _ = on;
}

/// 把 Winc 列入各文件类型的候选程序或从中去掉；其他平台上什么也不做。
pub(crate) fn list_types(on: bool) {
    #[cfg(windows)]
    windows::list_types(on);
    #[cfg(not(windows))]
    let _ = on;
}
