//! 伪终端接口的选择：优先使用随程序发布的 `conpty.dll`，其次使用系统接口。
//!
//! 系统接口的行为随 Windows 版本变化。发布的版本由 `release/windows/conpty.ps1` 固定，控制台在
//! 同目录的 `OpenConsole.exe` 中运行，各系统上行为一致。文件缺失、无法加载或创建失败时使用系统
//! 接口；环境变量 `WINC_SYSTEM_CONPTY=1` 强制使用系统接口，用于排查问题。

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::core::HRESULT;

/// 控制台按字素簇度量文字宽度，与终端的计算方式一致。
///
/// 取值见 ConPTY 的头文件 `conpty-static.h`。系统接口不一定支持，只传给发布的版本。
const PSEUDOCONSOLE_GLYPH_WIDTH_GRAPHEMES: u32 = 0x08;

type Create = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type Resize = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type Close = unsafe extern "system" fn(HPCON);

/// 一套伪终端接口。伪终端必须由创建它的那一套接口调整尺寸和关闭。
pub(super) struct Api {
    create: Create,
    pub(super) resize: Resize,
    pub(super) close: Close,
    flags: u32,
}

static SYSTEM: Api = Api {
    create: CreatePseudoConsole,
    resize: ResizePseudoConsole,
    close: ClosePseudoConsole,
    flags: 0,
};

/// 新建伪终端，返回它和创建它的接口；发布的版本创建失败时改用系统接口。
///
/// # Safety
///
/// `input`、`output` 必须是在调用期间有效的管道句柄。
///
/// # Errors
///
/// 系统接口也创建失败时返回它的 `HRESULT`。
pub(super) unsafe fn create(
    size: COORD,
    input: HANDLE,
    output: HANDLE,
) -> Result<(&'static Api, HPCON), HRESULT> {
    static BUNDLED: OnceLock<Option<Api>> = OnceLock::new();
    if let Some(api) = BUNDLED.get_or_init(load)
        // SAFETY: 调用方保证两个句柄有效。
        && let Ok(console) = unsafe { api.attempt(size, input, output) }
    {
        return Ok((api, console));
    }
    // SAFETY: 调用方保证两个句柄有效；上一次失败的尝试不占用它们。
    unsafe { SYSTEM.attempt(size, input, output) }.map(|console| (&SYSTEM, console))
}

impl Api {
    /// 用这一套接口新建伪终端。
    ///
    /// # Safety
    ///
    /// `input`、`output` 必须是在调用期间有效的管道句柄。
    unsafe fn attempt(&self, size: COORD, input: HANDLE, output: HANDLE) -> Result<HPCON, HRESULT> {
        let mut console: HPCON = 0;
        // SAFETY: 调用方保证两个句柄有效；`console` 指向本地变量，调用成功时写入新伪终端的句柄。
        let result = unsafe { (self.create)(size, input, output, self.flags, &mut console) };
        if result < 0 { Err(result) } else { Ok(console) }
    }
}

/// 加载 `winc-pty` 旁边的 `conpty.dll`；未发布、被环境变量禁用或无法加载时返回 `None`。
fn load() -> Option<Api> {
    if std::env::var_os("WINC_SYSTEM_CONPTY").is_some_and(|value| value == "1") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let library = bundled_library(exe.parent()?)?;
    let path: Vec<u16> = (library.as_os_str().encode_wide())
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `path` 是以 NUL 结尾的完整路径，在调用期间有效；搜索范围只有库所在的目录和系统目录，
    // 不包括当前目录和 `PATH`。模块不释放，取到的函数在进程存续期间一直有效。
    let module = unsafe {
        LoadLibraryExW(
            path.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return None;
    }
    // SAFETY: 模块句柄有效，名字是以 NUL 结尾的字符串常量；三个函数的签名与同名系统接口相同，
    // 见 ConPTY 的头文件 `conpty-static.h`。
    unsafe {
        let create = GetProcAddress(module, c"ConptyCreatePseudoConsole".as_ptr().cast())?;
        let resize = GetProcAddress(module, c"ConptyResizePseudoConsole".as_ptr().cast())?;
        let close = GetProcAddress(module, c"ConptyClosePseudoConsole".as_ptr().cast())?;
        Some(Api {
            create: std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(create),
            resize: std::mem::transmute::<unsafe extern "system" fn() -> isize, Resize>(resize),
            close: std::mem::transmute::<unsafe extern "system" fn() -> isize, Close>(close),
            flags: PSEUDOCONSOLE_GLYPH_WIDTH_GRAPHEMES,
        })
    }
}

/// 返回目录 `dir` 中发布的 `conpty.dll` 的路径；缺少它或 `OpenConsole.exe` 时返回 `None`。
///
/// 缺少 `OpenConsole.exe` 时该库不报错，而是启动系统的控制台，因此两个文件都存在才使用。
fn bundled_library(dir: &Path) -> Option<PathBuf> {
    let library = dir.join("conpty.dll");
    (library.is_file() && dir.join("OpenConsole.exe").is_file()).then_some(library)
}

#[cfg(test)]
#[path = "api/tests.rs"]
mod tests;
