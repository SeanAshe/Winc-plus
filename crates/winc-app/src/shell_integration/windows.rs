//! 「系统集成」在 Windows 上的实现：读写注册表，改完通知资源管理器。

use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW,
    RegDeleteValueW, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

use super::Integration;
use super::registry::{Menu, Registry, menu_labels, menu_on, set_menu, set_types, types_on};

/// 返回以 NUL 结尾的 UTF-16 文字。
fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain([0])
        .collect()
}

/// 打开着的注册表项，离开作用域时关闭。
struct Key(HKEY);

impl Key {
    /// 按权限 `access` 打开 `root` 下的 `key`；不存在或没有权限时为 `None`。
    #[allow(unsafe_code)] // 读写注册表只能调用系统接口
    fn open(root: HKEY, key: &str, access: u32) -> Option<Key> {
        let path = wide(key);
        let mut opened: HKEY = std::ptr::null_mut();
        // SAFETY: 路径是以 NUL 结尾的 UTF-16，`opened` 是有效的输出位置。
        let status = unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, access, &mut opened) };
        (status == ERROR_SUCCESS).then_some(Key(opened))
    }

    /// 打开当前用户之下的 `key`，不存在时创建。
    #[allow(unsafe_code)] // 读写注册表只能调用系统接口
    fn create(key: &str) -> Option<Key> {
        let path = wide(key);
        let mut opened: HKEY = std::ptr::null_mut();
        // SAFETY: 路径是以 NUL 结尾的 UTF-16，`opened` 是有效的输出位置，其余可选参数为空。
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut opened,
                std::ptr::null_mut(),
            )
        };
        (status == ERROR_SUCCESS).then_some(Key(opened))
    }

    /// 把名为 `name` 的值写成类型 `kind` 的 `data`。
    #[allow(unsafe_code)] // 读写注册表只能调用系统接口
    fn set(&self, name: &str, kind: u32, data: &[u8]) {
        let name = wide(name);
        // SAFETY: `self.0` 已打开；名称以 NUL 结尾；`data` 和它的长度描述同一段内存。
        unsafe {
            RegSetValueExW(
                self.0,
                name.as_ptr(),
                0,
                kind,
                data.as_ptr(),
                data.len() as u32,
            )
        };
    }
}

impl Drop for Key {
    #[allow(unsafe_code)] // 读写注册表只能调用系统接口
    fn drop(&mut self) {
        // SAFETY: `self.0` 由 `open` 或 `create` 打开，只在这里关闭一次。
        unsafe { RegCloseKey(self.0) };
    }
}

/// 系统的注册表。
struct System;

impl Registry for System {
    fn has_key(&self, machine: bool, key: &str) -> bool {
        let root = if machine {
            HKEY_LOCAL_MACHINE
        } else {
            HKEY_CURRENT_USER
        };
        Key::open(root, key, KEY_READ).is_some()
    }

    #[allow(unsafe_code)] // 读写注册表只能调用系统接口
    fn has_value(&self, key: &str, name: &str) -> bool {
        let Some(key) = Key::open(HKEY_CURRENT_USER, key, KEY_READ) else {
            return false;
        };
        let name = wide(name);
        // SAFETY: `key` 已打开；名称以 NUL 结尾；不取类型和数据，各输出参数为空。
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        status == ERROR_SUCCESS
    }

    fn set_text(&mut self, key: &str, name: &str, data: &str) {
        let Some(key) = Key::create(key) else {
            return;
        };
        let data: Vec<u8> = wide(data)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        key.set(name, REG_SZ, &data);
    }

    fn set_number(&mut self, key: &str, name: &str, data: u32) {
        if let Some(key) = Key::open(HKEY_CURRENT_USER, key, KEY_SET_VALUE) {
            key.set(name, REG_DWORD, &data.to_le_bytes());
        }
    }

    #[allow(unsafe_code)] // 读写注册表只能调用系统接口
    fn delete_value(&mut self, key: &str, name: &str) {
        let Some(key) = Key::open(HKEY_CURRENT_USER, key, KEY_SET_VALUE) else {
            return;
        };
        let name = wide(name);
        // SAFETY: `key` 已打开；名称以 NUL 结尾。
        unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
    }

    #[allow(unsafe_code)] // 读写注册表只能调用系统接口
    fn delete_tree(&mut self, key: &str) {
        let path = wide(key);
        // SAFETY: 路径是以 NUL 结尾的 UTF-16。
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
    }

    #[allow(unsafe_code)] // 读写注册表只能调用系统接口
    fn delete_if_empty(&mut self, key: &str) {
        let Some(opened) = Key::open(HKEY_CURRENT_USER, key, KEY_READ) else {
            return;
        };
        let (mut keys, mut values) = (0u32, 0u32);
        // SAFETY: `opened` 已打开；只取子项数和值的个数，其余输出参数为空。
        let status = unsafe {
            RegQueryInfoKeyW(
                opened.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                &mut keys,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut values,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        drop(opened);
        if status == ERROR_SUCCESS && keys == 0 && values == 0 {
            let path = wide(key);
            // SAFETY: 路径是以 NUL 结尾的 UTF-16。
            unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, path.as_ptr()) };
        }
    }
}

/// 通知资源管理器文件关联变了：它重新读右键菜单和候选程序。
#[allow(unsafe_code)] // 通知资源管理器只能调用系统接口
fn notify() {
    // SAFETY: 这一事件不带参数，两个条目都为空。
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as _,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
}

/// 是否由安装程序装在当前用户之下（NSIS 安装包）；进程内只判断一次。
pub(super) fn per_user() -> bool {
    static PER_USER: OnceLock<bool> = OnceLock::new();
    *PER_USER.get_or_init(crate::updates::installed_per_user)
}

pub(super) fn read() -> Integration {
    Integration {
        menu: menu_on(&System),
        types: types_on(&System),
    }
}

pub(super) fn show_menu(on: bool) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let chinese = winc_i18n::Locale::system() == winc_i18n::Locale::ZhCn;
    let (file, folder) = menu_labels(chinese);
    let menu = Menu {
        exe: &exe,
        file,
        folder,
    };
    set_menu(&mut System, on.then_some(&menu), per_user());
    notify();
}

pub(super) fn list_types(on: bool) {
    set_types(&mut System, on);
    notify();
}
