//! 文件类型登记。安装时为文本扩展名建好 `ProgID`、文件图标和打开命令（参照 Notepads 的文件类型清单）；
//! 平时启动只刷新已经由 winc 打开的那些类型。别的程序占着的默认关联不改。

#![allow(unsafe_code)]

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

use crate::shell_icons::{
    APP_ICON_ID, FILE_ICONS, claims_extension, command_path, command_targets_winc, id_for_extension,
};

/// 按当前可执行文件里的图标，刷新外壳里 winc 负责的那些扩展名。
///
/// 登记失败时静默跳过，不影响启动。有改动才通知外壳重读图标。
pub fn register_file_icons() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let launch = launch_command(&exe);
    let mut changed = ensure(
        r"Software\Classes\Applications\winc.exe\DefaultIcon",
        &icon_of(&exe, APP_ICON_ID),
    );
    // 打开命令也指向这次运行的程序，这样「打开方式」取到的是嵌在这个文件里的应用图标。
    changed |= ensure(
        r"Software\Classes\Applications\winc.exe\shell\open\command",
        &launch,
    );
    for ext in extensions() {
        changed |= register_extension(&exe, &ext);
    }
    changed |= register_directory_menu(&exe);
    if changed {
        notify_shell();
    }
}

/// 安装程序登记全部文本文件类型：每种扩展名一条 `ProgID`、文件图标，并出现在「打开方式」里。
///
/// 还没有默认程序，或者默认程序已经是 winc 时，把默认程序设成这条 `ProgID`，资源管理器才会显示文件图标。
/// 别的程序占着的（例如用户选过的 VS Code）只加入可选列表。
pub fn register_file_types() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let launch = launch_command(&exe);
    let mut changed = ensure(
        r"Software\Classes\Applications\winc.exe\DefaultIcon",
        &icon_of(&exe, APP_ICON_ID),
    );
    changed |= ensure(
        r"Software\Classes\Applications\winc.exe\shell\open\command",
        &launch,
    );
    changed |= ensure_named(
        r"Software\Classes\Applications\winc.exe",
        "FriendlyAppName",
        "winc",
    );
    changed |= ensure_named(
        r"Software\DEEIX\winc\Capabilities",
        "ApplicationName",
        "winc",
    );
    changed |= ensure_named(
        r"Software\DEEIX\winc\Capabilities",
        "ApplicationDescription",
        "winc",
    );
    changed |= ensure_named(
        r"Software\DEEIX\winc\Capabilities",
        "ApplicationIcon",
        &icon_of(&exe, APP_ICON_ID),
    );
    changed |= ensure_named(
        r"Software\RegisteredApplications",
        "winc",
        r"Software\DEEIX\winc\Capabilities",
    );
    for icon in FILE_ICONS {
        for ext in icon.extensions {
            changed |= register_type(&exe, ext, icon.label, &launch);
        }
    }
    changed |= register_directory_menu(&exe);
    if changed {
        notify_shell();
    }
}

/// 卸载时去掉 [`register_file_types`] 写下的类型。指向别的 winc.exe 的登记留着。
pub fn unregister_file_types() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut changed = false;
    for icon in FILE_ICONS {
        for ext in icon.extensions {
            let progid = format!("Winc.{ext}");
            let class = format!(r"Software\Classes\.{ext}");
            if read_value(&class, None).as_deref() == Some(progid.as_str()) {
                changed |= delete_value(&class, None);
            }
            changed |= delete_value(&format!(r"{class}\OpenWithProgids"), Some(&progid));
            changed |= delete_value(
                r"Software\Classes\Applications\winc.exe\SupportedTypes",
                Some(&format!(".{ext}")),
            );
            changed |= delete_tree(&format!(r"Software\Classes\{progid}"));
        }
    }
    changed |= delete_tree(r"Software\DEEIX\winc\Capabilities");
    changed |= delete_value(r"Software\RegisteredApplications", Some("winc"));
    let command = open_command(r"Applications\winc.exe");
    if command.is_some_and(|command| command_uses_this_exe(&command, &exe)) {
        changed |= delete_tree(r"Software\Classes\Applications\winc.exe");
    }
    changed |= unregister_directory_menu(&exe);
    if changed {
        notify_shell();
    }
}

/// 为扩展名 `ext`（不带点）写 `ProgID`、图标和「打开方式」。
fn register_type(exe: &Path, ext: &str, label: &str, launch: &str) -> bool {
    let progid = format!("Winc.{ext}");
    let class = format!(r"Software\Classes\{progid}");
    let mut changed = ensure(&class, &type_name(label, ext));
    changed |= ensure(
        &format!(r"{class}\DefaultIcon"),
        &icon_of(exe, id_for_extension(ext)),
    );
    changed |= ensure(&format!(r"{class}\shell\open\command"), launch);
    changed |= ensure_named(
        r"Software\DEEIX\winc\Capabilities\FileAssociations",
        &format!(".{ext}"),
        &progid,
    );
    changed |= ensure_named(
        r"Software\Classes\Applications\winc.exe\SupportedTypes",
        &format!(".{ext}"),
        "",
    );
    let dotted = format!(".{ext}");
    let current = read_value(&format!(r"Software\Classes\{dotted}"), None);
    let command = current.as_deref().and_then(open_command);
    if user_choice(&dotted).is_none()
        && claims_extension(current.as_deref(), command.as_deref(), &progid)
    {
        changed |= ensure(&format!(r"Software\Classes\{dotted}"), &progid);
        changed |= ensure_named(
            &format!(r"Software\Classes\{dotted}\OpenWithProgids"),
            &progid,
            "",
        );
    }
    changed
}

/// 类型在属性窗口里的名字，随界面语言。
fn type_name(label: &str, ext: &str) -> String {
    let name = if label.is_empty() { ext } else { label };
    winc_i18n::t!("cmd-file-type-name", name = name)
}

/// 扩展名 `ext`（带点）若已由 winc 打开，把它的默认图标指到对应的文件图标。
fn register_extension(exe: &Path, ext: &str) -> bool {
    let Some(progid) = effective_progid(ext) else {
        return false;
    };
    let Some(command) = open_command(&progid) else {
        return false;
    };
    if !command_targets_winc(&command) {
        return false;
    }
    let icon = icon_of(exe, id_for_extension(ext));
    let launch = launch_command(exe);
    // `Applications\winc.exe` 是所有扩展名共用的一条，放不进各自的文件图标。
    // 没有用户选择（UserChoice）时改成这条扩展名自己的 `ProgID`。
    if progid.eq_ignore_ascii_case(r"Applications\winc.exe") {
        if user_choice(ext).is_some() {
            return false;
        }
        let own = format!("Winc.{}", ext.trim_start_matches('.'));
        let mut changed = ensure(
            &format!(r"Software\Classes\{own}\shell\open\command"),
            &launch,
        );
        changed |= ensure(&format!(r"Software\Classes\{own}\DefaultIcon"), &icon);
        changed |= ensure(&format!(r"Software\Classes\{ext}"), &own);
        return changed;
    }
    let mut changed = ensure(
        &format!(r"Software\Classes\{progid}\shell\open\command"),
        &launch,
    );
    changed |= ensure(&format!(r"Software\Classes\{progid}\DefaultIcon"), &icon);
    changed
}

/// 资源管理器里文件夹右键菜单的两项：点在文件夹上，以及点在文件夹空白处。
const DIRECTORY_MENUS: [&str; 2] = [
    r"Software\Classes\Directory\shell\Winc",
    r"Software\Classes\Directory\Background\shell\Winc",
];

/// 登记「通过 Winc 打开」。已经是这个命令时不动。
fn register_directory_menu(exe: &Path) -> bool {
    let label = winc_i18n::t!("menu-open-with-winc");
    let command = format!("\"{}\" \"%V\"", exe_text(exe));
    let icon = exe_text(exe);
    let mut changed = false;
    for key in DIRECTORY_MENUS {
        changed |= ensure(key, &label);
        changed |= ensure_named(key, "Icon", &icon);
        changed |= ensure(&format!(r"{key}\command"), &command);
    }
    changed
}

/// 去掉本程序写上的「通过 Winc 打开」。命令指向别的 winc.exe 时留着。
fn unregister_directory_menu(exe: &Path) -> bool {
    let mut changed = false;
    for key in DIRECTORY_MENUS {
        let command = read_value(&format!(r"{key}\command"), None);
        if command.is_some_and(|command| command_uses_this_exe(&command, exe)) {
            changed |= delete_tree(key);
        }
    }
    changed
}

/// `"路径\winc.exe" "%1"`，和系统「打开方式」写进注册表的格式一致。
fn launch_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe_text(exe))
}

/// 外壳引用可执行文件里编号为 `id` 的图标。负数表示资源编号，不随图标组的排列变化。
fn icon_of(exe: &Path, id: u16) -> String {
    format!("{},-{id}", exe_text(exe))
}

fn exe_text(exe: &Path) -> String {
    let text = exe.display().to_string();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

/// 用户曾经选过或登记过的扩展名（带点）。
fn extensions() -> Vec<String> {
    let mut names = enum_subkeys(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts",
        KEY_ENUMERATE_SUB_KEYS,
    );
    for name in enum_subkeys(r"Software\Classes", KEY_ENUMERATE_SUB_KEYS) {
        if name.starts_with('.') && !names.iter().any(|have| have.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names
}

/// 决定这个扩展名用哪条 `ProgID`：用户选择优先，其次是类登记，最后是 Windows 11 的 Latest。
fn effective_progid(ext: &str) -> Option<String> {
    if let Some(id) = user_choice(ext) {
        return Some(id);
    }
    if let Some(id) =
        read_value(&format!(r"Software\Classes\{ext}"), None).filter(|id| !id.is_empty())
    {
        return Some(id);
    }
    let path = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\{ext}\\UserChoiceLatest"
    );
    read_value(&format!("{path}\\ProgId"), Some("ProgId")).filter(|id| !id.is_empty())
}

/// `UserChoice` 里的 `ProgID`。有这一项时它压过类登记，不能改默认程序，只能改图标。
fn user_choice(ext: &str) -> Option<String> {
    read_value(
        &format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoice"),
        Some("ProgId"),
    )
    .filter(|id| !id.is_empty())
}

/// `ProgID` 的打开命令。
fn open_command(progid: &str) -> Option<String> {
    read_value(
        &format!(r"Software\Classes\{progid}\shell\open\command"),
        None,
    )
}

/// 读 `subkey` 下名为 `name` 的字符串；`name` 为空时读默认值。
fn read_value(subkey: &str, name: Option<&str>) -> Option<String> {
    let key = open(subkey, KEY_QUERY_VALUE)?;
    query(&key, name)
}

/// 把 `subkey` 的默认值写成 `value`。已经是这个值时不动，返回是否写过。
fn ensure(subkey: &str, value: &str) -> bool {
    write_value(subkey, None, value)
}

/// 同 [`ensure`]，写名为 `name` 的值。
fn ensure_named(subkey: &str, name: &str, value: &str) -> bool {
    write_value(subkey, Some(name), value)
}

fn write_value(subkey: &str, name: Option<&str>, value: &str) -> bool {
    let Some(key) = create(subkey) else {
        return false;
    };
    if query(&key, name).as_deref() == Some(value) {
        return false;
    }
    set_value(&key, name, value)
}

/// 打开或建好的注册表键，离开作用域时关掉。
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 句柄由 `open` 或 `create` 取得，离开作用域时关掉，且只关这一次。
            let _ = unsafe { RegCloseKey(self.0) };
        }
    }
}

fn open(subkey: &str, access: u32) -> Option<Key> {
    let name = wide(subkey);
    let mut handle = std::ptr::null_mut();
    // SAFETY: `name` 以 NUL 结尾，句柄在 `Key` 里关掉。
    let err = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut handle) };
    (err == 0 && !handle.is_null()).then_some(Key(handle))
}

fn create(subkey: &str) -> Option<Key> {
    let name = wide(subkey);
    let mut handle = std::ptr::null_mut();
    // SAFETY: 在 HKCU 下建键，失败则没有句柄；成功的句柄交给 `Key`。
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            name.as_ptr(),
            0,
            std::ptr::null::<u16>(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            std::ptr::null(),
            &mut handle,
            std::ptr::null_mut(),
        )
    };
    (err == 0 && !handle.is_null()).then_some(Key(handle))
}

fn enum_subkeys(subkey: &str, access: u32) -> Vec<String> {
    let Some(key) = open(subkey, access) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut index = 0u32;
    let mut buf = [0u16; 256];
    loop {
        let mut len = buf.len() as u32;
        // SAFETY: `buf` 活得比这次调用久，`len` 是它能装下的字符数。
        let err = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                buf.as_mut_ptr(),
                &mut len,
                std::ptr::null::<u32>(),
                std::ptr::null_mut::<u16>(),
                std::ptr::null_mut::<u32>(),
                std::ptr::null_mut::<FILETIME>(),
            )
        };
        if err != 0 {
            break;
        }
        names.push(String::from_utf16_lossy(&buf[..len as usize]));
        index += 1;
    }
    names
}

fn query(key: &Key, name: Option<&str>) -> Option<String> {
    let owned = name.map(wide);
    let name_ptr = owned
        .as_ref()
        .map_or(std::ptr::null::<u16>(), |value| value.as_ptr());
    let mut buf = [0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: 缓冲区和长度都指向本函数的局部变量，调用返回后才读。
    let err = unsafe {
        RegQueryValueExW(
            key.0,
            name_ptr,
            std::ptr::null(),
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if err != 0 || bytes < 2 {
        return None;
    }
    let chars = (bytes as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..chars]))
}

fn set_value(key: &Key, name: Option<&str>, value: &str) -> bool {
    let data = wide(value);
    let owned = name.map(wide);
    let name_ptr = owned
        .as_ref()
        .map_or(std::ptr::null::<u16>(), |item| item.as_ptr());
    // SAFETY: `data` 与 `owned` 都以 NUL 结尾，长度含结尾的 NUL，调用返回前一直有效。
    let err = unsafe {
        RegSetValueExW(
            key.0,
            name_ptr,
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    err == 0
}

/// 删掉 `subkey` 下名为 `name` 的值；`name` 为 `None` 时删默认值。键不存在时不算改动。
fn delete_value(subkey: &str, name: Option<&str>) -> bool {
    let Some(key) = open(subkey, KEY_SET_VALUE) else {
        return false;
    };
    let owned = name.map(wide);
    let name_ptr = owned
        .as_ref()
        .map_or(std::ptr::null::<u16>(), |item| item.as_ptr());
    // SAFETY: 名字指针在调用期间有效；键不存在或值不存在时系统返回错误码，不写内存。
    let err = unsafe { RegDeleteValueW(key.0, name_ptr) };
    err == 0
}

/// 删掉 `subkey` 及其下面的全部子键。本来就没有时不算改动。
fn delete_tree(subkey: &str) -> bool {
    let name = wide(subkey);
    // SAFETY: `name` 以 NUL 结尾，指向 HKCU 下的子键；失败只表示键不存在。
    let err = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, name.as_ptr()) };
    err == 0
}

fn command_uses_this_exe(command: &str, exe: &Path) -> bool {
    let Some(path) = command_path(command) else {
        return false;
    };
    exe_text(Path::new(path)).eq_ignore_ascii_case(&exe_text(exe))
}

/// 通知资源管理器重新读取文件图标。
fn notify_shell() {
    // SAFETY: 两个指针按文档传空，只通知「关联变了」，不引用调用方的内存。
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain([0])
        .collect()
}
