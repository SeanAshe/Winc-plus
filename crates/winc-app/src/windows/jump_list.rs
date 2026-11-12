//! 任务栏的跳转列表：右键任务栏图标时列出最近打开的目录和「新建窗口」。
//!
//! 列表中的每一项是一个指向本程序的快捷方式：目录项以该目录为参数启动 winc，「新建窗口」不带
//! 参数启动；新进程把目标交给正在运行的 winc（见 `crate::instance`）。列表保存在系统中，winc
//! 未运行时同样可用，因此只在最近列表变化时重写。
//!
//! 用户可以在跳转列表中移除某一项。系统要求应用不再提供被移除的项，否则整个分类写入失败：
//! 重写列表时先取得被移除的项，从最近列表中一并移除。

// COM 接口调用；每个 `unsafe` 块都有 `// SAFETY:` 说明。
#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};
use windows::core::{GUID, HSTRING, Interface, Result};

/// 快捷方式的标题属性（`PKEY_Title`，`<propkey.h>`）：跳转列表中显示的文字。
const TITLE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9),
    pid: 2,
};

/// 读取快捷方式参数的缓冲区长度（UTF-16 码元）：Windows 的路径上限 32767 加上两侧的引号和
/// 结尾的 NUL。
const ARGUMENTS: usize = 32_770;

/// 最近一次写入跳转列表的目录；与当前的最近列表相同时不重写。
static WRITTEN: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

/// 把最近打开的目录 `recent`（最近的在前）写入任务栏的跳转列表，在后台线程中完成。
///
/// 与上一次写入的目录相同时不做任何事。用户在跳转列表中移除的目录同时从状态文件 `state` 的
/// 最近列表中移除。写入失败（如系统设置中关闭了跳转列表）时不提示。
pub fn sync_jump_list(recent: &[PathBuf], state: &Path) {
    let Ok(mut written) = WRITTEN.lock() else {
        return;
    };
    if written.as_deref() == Some(recent) {
        return;
    }
    *written = Some(recent.to_vec());
    drop(written);

    let (recent, state) = (recent.to_vec(), state.to_path_buf());
    let spawned = std::thread::Builder::new()
        .name("winc-jump-list".to_owned())
        .spawn(move || {
            // SAFETY: 本线程尚未初始化 COM；成功后在线程结束前调用一次 `CoUninitialize` 配对。
            if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
                return;
            }
            let removed = write(&recent).unwrap_or_default();
            // SAFETY: 与上面成功的 `CoInitializeEx` 配对；`write` 中的 COM 对象此时均已释放。
            unsafe { CoUninitialize() };
            if !removed.is_empty() {
                let forget = |state: &mut winc_config::State| {
                    for folder in &removed {
                        state.forget(folder);
                    }
                };
                let _ = winc_config::State::update(&state, forget);
            }
        });
    // 线程没有建成时清除记录，下一次变化时重试。
    if spawned.is_err()
        && let Ok(mut written) = WRITTEN.lock()
    {
        *written = None;
    }
}

/// 重写跳转列表：`recent` 中未被用户移除的目录，以及「新建窗口」。返回用户移除的目录。
///
/// 必须在已初始化 COM 的线程上调用。
fn write(recent: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let exe = std::env::current_exe().map_err(|_| windows::core::Error::empty())?;
    // SAFETY: 以下均为已初始化 COM 的线程上的接口调用；传入的字符串、属性值和接口引用在各次
    // 调用期间有效，接口对象由各自的智能指针持有并在离开作用域时释放。
    unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        let mut slots = 0;
        let dropped: IObjectArray = list.BeginList(&mut slots)?;
        let mut removed = Vec::new();
        for index in 0..dropped.GetCount()? {
            // 被移除的项也可能是外壳项（由系统的「最近」分类产生），只处理本程序写入的快捷方式。
            let Ok(link) = dropped.GetAt::<IShellLinkW>(index) else {
                continue;
            };
            let mut arguments = vec![0u16; ARGUMENTS];
            if link.GetArguments(&mut arguments).is_err() {
                continue;
            }
            let end = arguments.iter().position(|&c| c == 0).unwrap_or(0);
            let arguments = String::from_utf16_lossy(&arguments[..end]);
            removed.extend(
                recent
                    .iter()
                    .filter(|f| crate::recents::argument(f) == arguments)
                    .cloned(),
            );
        }

        let folders: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        let kept: Vec<PathBuf> = (recent.iter())
            .filter(|folder| !removed.contains(folder))
            .cloned()
            .collect();
        let labels = crate::recents::labels(&kept);
        for (folder, label) in kept.iter().zip(&labels).take(slots as usize) {
            let tip = folder.to_string_lossy();
            folders.AddObject(&link(&exe, &crate::recents::argument(folder), label, &tip)?)?;
        }
        // 分类写入失败（如用户关闭了「显示最近打开的项」）时仍然写入任务。
        if folders.GetCount()? > 0 {
            let name = HSTRING::from(winc_i18n::t!("workspace-jump-list-recent"));
            let _ = list.AppendCategory(&name, &folders);
        }

        let tasks: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        let new_window = winc_i18n::t!("cmd-new-window");
        tasks.AddObject(&link(&exe, "", new_window, new_window)?)?;
        list.AddUserTasks(&tasks)?;
        list.CommitList()?;
        Ok(removed)
    }
}

/// 创建以参数 `arguments` 启动 `exe` 的快捷方式，在跳转列表中显示为 `title`，提示文字为 `tip`。
///
/// # Safety
///
/// 必须在已初始化 COM 的线程上调用。
unsafe fn link(exe: &Path, arguments: &str, title: &str, tip: &str) -> Result<IShellLinkW> {
    // SAFETY: 调用方保证线程已初始化 COM；传入的字符串和属性值在各次调用期间有效。
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let exe = HSTRING::from(exe.as_os_str());
        link.SetPath(&exe)?;
        link.SetArguments(&HSTRING::from(arguments))?;
        link.SetIconLocation(&exe, 0)?;
        link.SetDescription(&HSTRING::from(tip))?;
        let store: IPropertyStore = link.cast()?;
        store.SetValue(&TITLE, &PROPVARIANT::from(title))?;
        store.Commit()?;
        Ok(link)
    }
}
