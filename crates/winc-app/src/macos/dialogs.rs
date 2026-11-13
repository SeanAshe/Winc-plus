//! 系统的打开、保存面板，提示框和弹出菜单（右键菜单、最近项目菜单）。
//!
//! 面板和提示框以 sheet 形式挂在窗口上，不阻塞事件循环；弹出菜单在关闭后才返回。

use super::app_menu::{DELEGATE, MENU_ACTION, app_delegate};
use super::files::{file_url, ns_string, url_path};
use super::*;

/// 系统的打开、保存面板。
pub enum Panel<'a> {
    /// 打开：`files` 为 `false` 时只能选目录；`dir` 是起始目录，`None` 时由系统决定。
    Open {
        /// 是否可以选文件。
        files: bool,
        /// 起始目录。
        dir: Option<&'a Path>,
    },
    /// 保存：默认文件名和起始目录。
    Save {
        /// 默认文件名。
        name: &'a str,
        /// 起始目录。
        dir: Option<&'a Path>,
    },
}

/// 在窗口上弹出打开或保存面板（从标题栏滑下的 sheet），选择完成或取消后在主线程上调用 `done`；
/// 取消时参数为 `None`。
///
/// 立即返回，不阻塞事件循环。返回是否弹出了面板：窗口上已挂着 sheet，或无法取得 `NSWindow`
/// 时返回 `false`。
pub fn show_panel(
    window: &Window,
    panel: Panel<'_>,
    done: impl Fn(Option<PathBuf>) + 'static,
) -> bool {
    let Some(ns_window) = ns_window(window) else {
        return false;
    };

    // SAFETY: 都是 NSWindow、NSOpenPanel、NSSavePanel、NSURL 的公开 API，在主线程上调用（winit
    // 只在主线程派发事件），返回值先判空再用。`openPanel` / `savePanel` 返回的是自动释放的对象，
    // 先 retain 一次交给完成回调持有，回调用完连同 block 一起释放。block 的签名和
    // `completionHandler:` 要求的 `void (^)(NSModalResponse)` 一致，NSModalResponse 是 NSInteger。
    unsafe {
        let sheet: *mut AnyObject = msg_send![ns_window, attachedSheet];
        if !sheet.is_null() {
            return false;
        }

        let raw: *mut AnyObject = match panel {
            Panel::Open { .. } => msg_send![class!(NSOpenPanel), openPanel],
            Panel::Save { .. } => msg_send![class!(NSSavePanel), savePanel],
        };
        let Some(object) = Retained::retain(raw) else {
            return false;
        };

        // 程序不是 .app 包时，系统按英文显示按钮，因此显式设置当前界面语言的文字。
        match panel {
            Panel::Open { files, dir } => {
                if let Some(url) = dir.and_then(|dir| file_url(dir, true)) {
                    let () = msg_send![&*object, setDirectoryURL: &*url];
                }
                let () = msg_send![&*object, setCanChooseFiles: files];
                let () = msg_send![&*object, setCanChooseDirectories: true];
                let () = msg_send![&*object, setAllowsMultipleSelection: false];
                if let Some(prompt) = ns_string(winc_i18n::t!("common-open")) {
                    let () = msg_send![&*object, setPrompt: &*prompt];
                }
            }
            Panel::Save { name, dir } => {
                if let Some(prompt) = ns_string(winc_i18n::t!("common-save")) {
                    let () = msg_send![&*object, setPrompt: &*prompt];
                }
                if let Some(label) = ns_string(winc_i18n::t!("common-save-as-label")) {
                    let () = msg_send![&*object, setNameFieldLabel: &*label];
                }
                if let Some(name) = ns_string(name) {
                    let () = msg_send![&*object, setNameFieldStringValue: &*name];
                }
                if let Some(url) = dir.and_then(|dir| file_url(dir, true)) {
                    let () = msg_send![&*object, setDirectoryURL: &*url];
                }
            }
        }

        let panel = object.clone();
        let handler = RcBlock::new(move |response: isize| {
            // NSModalResponseOK = 1，NSModalResponseCancel = 0。
            let path = (response == 1).then(|| panel_path(&panel)).flatten();
            done(path);
        });
        let () = msg_send![
            &*object,
            beginSheetModalForWindow: ns_window,
            completionHandler: &*handler
        ];
    }

    true
}

/// 返回面板上选中项（`URL` 属性）的路径；没有选中项时返回 `None`。
///
/// # Safety
///
/// `panel` 必须是有效的 NSSavePanel（或其子类 NSOpenPanel），并在主线程上调用。
unsafe fn panel_path(panel: &AnyObject) -> Option<PathBuf> {
    // SAFETY: 由调用方保证 `panel` 有效；`URL` 可能为空，不空时是 NSURL。
    unsafe {
        let url: *mut AnyObject = msg_send![panel, URL];
        url_path(url.as_ref()?)
    }
}

/// 菜单中的一项。
pub struct MenuItem<'a> {
    /// 标题。
    pub title: &'a str,
    /// 标题下面一行小字（使用系统的次要文字颜色），例如项目所在的目录。
    pub subtitle: Option<&'a str>,
    /// 鼠标停在该项上时的提示。
    pub tooltip: Option<&'a str>,
    /// 标题左边的图标。
    pub icon: Option<MenuIcon<'a>>,
    /// `NSMenuItem` 的 `keyEquivalent`：显示在菜单项右侧，菜单打开时也可按下。
    pub key: &'a str,
    /// `key` 的修饰键，取 `NSEventModifierFlags` 的位（见 `MOD_*`）。
    pub modifiers: usize,
    /// 是否可用。
    pub enabled: bool,
    /// 是否在左边打勾（多选的菜单，例如保存时跳过哪些语言）。
    pub checked: bool,
    /// 是否是前一项的备选项：按住 ⌥ 时代替前一项显示（`setAlternate:`）。
    ///
    /// 前一项不能有快捷键，
    /// 两项的 `keyEquivalent` 相同（都为空）、修饰键不同，系统才认作一对。
    pub alternate: bool,
    /// 菜单项的 `tag`，选中时传给 [`super::on_menu_command`] 注册的回调。
    pub tag: isize,
    /// 子菜单；不为空时该项本身不是命令，鼠标移上去时展开子菜单。
    pub submenu: Vec<Option<MenuItem<'a>>>,
}

/// 菜单项左侧的图标：已绘制好的像素（预乘过透明度），`size` 是逻辑尺寸。
pub struct MenuIcon<'a> {
    /// 图标的像素。
    pub image: &'a Image,
    /// 图标的逻辑尺寸（宽，高）。
    pub size: (f32, f32),
}

/// `NSEventModifierFlagShift`：Shift 键。
pub const MOD_SHIFT: usize = 1 << 17;
/// `NSEventModifierFlagControl`：Control 键。
pub const MOD_CONTROL: usize = 1 << 18;
/// `NSEventModifierFlagOption`：Option 键。
pub const MOD_OPTION: usize = 1 << 19;
/// `NSEventModifierFlagCommand`：Command 键。
pub const MOD_COMMAND: usize = 1 << 20;

/// 系统菜单弹出的位置和宽度。
pub struct MenuPlace {
    /// 窗口中的位置（`AppKit` 的点，左上角为原点），默认对准菜单的左上角。
    pub at: (f64, f64),
    /// 改为对准第一个打勾的项，让当前选择盖在触发菜单的按钮上。
    pub at_checked: bool,
    /// `at` 是菜单的右上角，按菜单宽度往左挪。
    pub right_aligned: bool,
    /// 菜单的最小宽度（`AppKit` 的点）；没有要求时为 0。
    pub min_width: f64,
}

/// 在窗口上按 `place` 弹出系统菜单，`items` 中的 `None` 表示分隔线。
///
/// 菜单关闭后才返回，选中的项通过 [`super::on_menu_command`] 注册的回调送出；返回是否弹出了菜单。
/// 可以在 `winit` 的事件回调中调用（菜单跟踪期间它把事件排队，回调返回后再派发）。
pub fn popup_menu(window: &Window, items: &[Option<MenuItem<'_>>], place: MenuPlace) -> bool {
    let Some(target) = AnyClass::get(DELEGATE).and_then(|_| app_delegate()) else {
        return false;
    };
    let Some(menu) = new_menu() else {
        return false;
    };
    // SAFETY: `menu` 是刚创建的有效 NSMenu，`fill_menu` 返回的打勾项由它持有；调用方在主线程
    // 上调用（winit 只在主线程派发事件）。
    unsafe {
        let checked = fill_menu(&menu, items, &target);
        let positioning = checked.as_deref().filter(|_| place.at_checked);
        let mut at = place.at;
        let () = msg_send![&*menu, setMinimumWidth: place.min_width];
        if place.right_aligned {
            let size: CGSize = msg_send![&*menu, size];
            at.0 -= size.width;
        }
        pop_up(window, &menu, positioning, at)
    }
}

/// 把 `items` 加进 `menu`，选中时的动作发给 `target`；返回第一个打勾的项。
///
/// # Safety
///
/// `menu` 必须指向有效的 `NSMenu`，`target` 必须是应用委托，并在主线程上调用。
unsafe fn fill_menu(
    menu: &AnyObject,
    items: &[Option<MenuItem<'_>>],
    target: &AnyObject,
) -> Option<Retained<AnyObject>> {
    let action = Sel::register(MENU_ACTION);
    // `setSubtitle:` 从 macOS 14.4 起才有；更早的系统把小字接在标题后面。
    // SAFETY: `instancesRespondToSelector:` 是公开的类方法，只读查询，参数是有效的 selector。
    let subtitles: bool =
        unsafe { msg_send![class!(NSMenuItem), instancesRespondToSelector: sel!(setSubtitle:)] };
    let mut checked = None;

    // SAFETY: NSMenuItem、NSMenu 的公开 API，由调用方保证 `menu`、`target` 有效且在主线程调用；
    // 菜单项、子菜单加进去以后由菜单持有。
    unsafe {
        for item in items {
            let Some(item) = item else {
                add_separator(menu);
                continue;
            };
            let title = match item.subtitle {
                Some(subtitle) if !subtitles => format!("{} — {subtitle}", item.title),
                _ => item.title.to_owned(),
            };
            let parent = !item.submenu.is_empty();
            let Some(entry) = menu_item(&title, (!parent).then_some(action), item.key) else {
                continue;
            };
            if subtitles && let Some(subtitle) = item.subtitle.and_then(ns_string) {
                let () = msg_send![&*entry, setSubtitle: &*subtitle];
            }
            if let Some(image) = item.icon.as_ref().and_then(icon_image) {
                let () = msg_send![&*entry, setImage: &*image];
            }
            if let Some(tooltip) = item.tooltip.and_then(ns_string) {
                let () = msg_send![&*entry, setToolTip: &*tooltip];
            }
            let modifiers = if item.alternate {
                MOD_OPTION
            } else {
                item.modifiers
            };
            let () = msg_send![&*entry, setKeyEquivalentModifierMask: modifiers];
            if item.alternate {
                let () = msg_send![&*entry, setAlternate: true];
            }
            let () = msg_send![&*entry, setTag: item.tag];
            let () = msg_send![&*entry, setEnabled: item.enabled];
            // NSControlStateValueOn = 1。
            let () = msg_send![&*entry, setState: isize::from(item.checked)];
            if parent {
                if let Some(submenu) = new_menu() {
                    fill_menu(&submenu, &item.submenu, target);
                    let () = msg_send![&*entry, setSubmenu: &*submenu];
                }
            } else {
                let () = msg_send![&*entry, setTarget: target];
            }
            let () = msg_send![menu, addItem: &*entry];
            if item.checked && checked.is_none() {
                checked = Some(entry);
            }
        }
    }
    checked
}

/// 把图标转成 NSImage：像素拷贝进一张 sRGB 位图，图片的大小取逻辑尺寸，Retina 屏上按位图的
/// 像素绘制。
///
/// 像素数与宽高不符，或创建失败时返回 `None`。
pub(super) fn icon_image(icon: &MenuIcon<'_>) -> Option<Retained<AnyObject>> {
    let (w, h) = (icon.image.width as usize, icon.image.height as usize);
    if icon.image.pixels.len() != w * h {
        return None;
    }
    let rgb = ns_string("NSDeviceRGBColorSpace")?;

    // SAFETY: NSBitmapImageRep、NSColorSpace、NSImage 的公开 API，返回值先判空再使用。
    // `planes` 为空时位图自己分配 `bytesPerRow × pixelsHigh` 字节，`bitmapData` 指向它，长度
    // 正好是 `w * h * 4`，拷贝完成之前位图一直由 `rep` 持有。`alloc` 加 `init…` 返回 +1 的对象，
    // 交给 `Retained` 管理；换了色彩空间的那张由自动释放池管理，`retain` 一次。
    unsafe {
        let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
        let planes = std::ptr::null_mut::<*mut u8>();
        let rep: *mut AnyObject = msg_send![
            rep,
            initWithBitmapDataPlanes: planes,
            pixelsWide: w as isize,
            pixelsHigh: h as isize,
            bitsPerSample: 8isize,
            samplesPerPixel: 4isize,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: &*rgb,
            bytesPerRow: (w * 4) as isize,
            bitsPerPixel: 32isize
        ];
        let rep = Retained::from_raw(rep)?;
        let data: *mut u8 = msg_send![&*rep, bitmapData];
        if data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts_mut(data, w * h * 4);
        for (out, &px) in bytes
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&icon.image.pixels)
        {
            out.copy_from_slice(&[
                (px >> 16) as u8,
                (px >> 8) as u8,
                px as u8,
                (px >> 24) as u8,
            ]);
        }
        // 图标颜色是 sRGB 的，标注色彩空间后，系统会按显示器换算。
        let srgb: *mut AnyObject = msg_send![class!(NSColorSpace), sRGBColorSpace];
        let rep: *mut AnyObject =
            msg_send![&*rep, bitmapImageRepByRetaggingWithColorSpace: srgb.as_ref()?];
        let rep = Retained::retain(rep)?;

        let (width, height) = icon.size;
        let image: *mut AnyObject = msg_send![class!(NSImage), alloc];
        let image: *mut AnyObject = msg_send![
            image,
            initWithSize: CGSize::new(f64::from(width), f64::from(height))
        ];
        let image = Retained::from_raw(image)?;
        let () = msg_send![&*image, addRepresentation: &*rep];
        Some(image)
    }
}

/// 创建一个空菜单；菜单项是否可用由调用方决定，不让 `AppKit` 沿响应链推断。
///
/// 须在主线程上调用。
pub(super) fn new_menu() -> Option<Retained<AnyObject>> {
    // SAFETY: NSMenu 的公开 API；`alloc` 加 `init` 返回 +1 的对象，交给 `Retained` 管理。
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), alloc];
        let menu: *mut AnyObject = msg_send![menu, init];
        let menu = Retained::from_raw(menu)?;
        let () = msg_send![&*menu, setAutoenablesItems: false];
        Some(menu)
    }
}

/// 创建一个菜单项；`action` 为 `None` 时选中该项不发送任何动作（说明文字、带子菜单的项）。
///
/// `title` 或 `key` 无法转成 `NSString` 时返回 `None`。
pub(super) fn menu_item(
    title: &str,
    action: Option<Sel>,
    key: &str,
) -> Option<Retained<AnyObject>> {
    let (title, key) = (ns_string(title)?, ns_string(key)?);
    // SAFETY: NSMenuItem 的公开 API；`alloc` 加 `init…` 返回 +1 的对象，交给 `Retained` 管理；
    // `title`、`key` 是有效的 NSString，在调用期间存活；动作可以为空（空 selector）。
    unsafe {
        let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let item: *mut AnyObject = msg_send![
            item,
            initWithTitle: &*title,
            action: action,
            keyEquivalent: &*key
        ];
        Retained::from_raw(item)
    }
}

/// 在菜单末尾加一条分隔线。
///
/// # Safety
///
/// `menu` 必须指向有效的 NSMenu，并在主线程上调用。
pub(super) unsafe fn add_separator(menu: &AnyObject) {
    // SAFETY: 由调用方保证 `menu` 是 NSMenu；`separatorItem` 返回自动释放的对象，加进菜单后由
    // 菜单持有。
    unsafe {
        let separator: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
        if let Some(separator) = separator.as_ref() {
            let () = msg_send![menu, addItem: separator];
        }
    }
}

/// 在窗口的 `at`（逻辑坐标，左上角为原点）处弹出菜单：`positioning` 项的左上角对准 `at`，
/// 为 `None` 时对准菜单的左上角；菜单关闭后才返回。
///
/// 返回是否弹出了菜单；无法取得窗口句柄时返回 `false`。
///
/// # Safety
///
/// `menu` 必须指向有效的 `NSMenu`，`positioning` 必须是其中的一项（或 `None`），并在主线程上调用。
pub(super) unsafe fn pop_up(
    window: &Window,
    menu: &AnyObject,
    positioning: Option<&AnyObject>,
    (x, y): (f64, f64),
) -> bool {
    let Some(view) = ns_view(window) else {
        return false;
    };

    // SAFETY: 由调用方保证 `menu` 和 `positioning` 有效（`positioning` 为菜单中的一项或空）；
    // 视图来自仍然存活的 winit 窗口，该视图是翻转的（左上角为原点），逻辑坐标可以直接使用；
    // 本函数只在主线程调用，满足 AppKit 的线程要求。
    unsafe {
        let _: bool = msg_send![
            menu,
            popUpMenuPositioningItem: positioning,
            atLocation: CGPoint::new(x, y),
            inView: view
        ];
    }
    true
}

/// 提示框上的一个按钮。
pub struct AlertButton<'a> {
    /// 按钮标题。
    pub title: &'a str,
    /// 快捷键及其修饰键（`NSEventModifierFlags`）；`None` 表示使用系统默认：第一个按钮是 ↩。
    pub key: Option<(&'a str, usize)>,
    /// 是否为危险操作；是时系统画成红字（macOS 11 起）。
    pub destructive: bool,
}

/// 在窗口上弹出提示框（sheet）。
///
/// `message` 是粗体的主句，`detail` 是下方的小字（空串不显示），按钮从右向左排列（第一个是默认
/// 按钮，按 ↩ 触发）。`suppression` 是按钮上方勾选框的文字（例如「不再询问」）和一开始是否勾上，
/// 为 `None` 时没有勾选框。立即返回；点击按钮后以按钮序号（从 0 起）和勾选框是否勾选调用 `done`。
/// 窗口上已挂着面板或提示框时不弹出，返回 `false`。
pub fn alert(
    window: &Window,
    message: &str,
    detail: &str,
    buttons: &[AlertButton<'_>],
    suppression: Option<(&str, bool)>,
    done: impl Fn(usize, bool) + 'static,
) -> bool {
    let Some(ns_window) = ns_window(window) else {
        return false;
    };

    // SAFETY: NSAlert、NSButton 的公开 API，在主线程上调用。`alloc` + `init` 返回 +1 的 NSAlert，
    // 交给完成回调持有，回调用完连同 block 一起释放。`setHasDestructiveAction:` 是 macOS 11
    // 才存在的，先检查再发送。block 签名与 `void (^)(NSModalResponse)` 一致。
    unsafe {
        let sheet: *mut AnyObject = msg_send![ns_window, attachedSheet];
        if !sheet.is_null() {
            return false;
        }

        let alert: *mut AnyObject = msg_send![class!(NSAlert), alloc];
        let alert: *mut AnyObject = msg_send![alert, init];
        let Some(alert) = Retained::from_raw(alert) else {
            return false;
        };

        if let Some(text) = ns_string(message) {
            let () = msg_send![&*alert, setMessageText: &*text];
        }
        if let Some(text) = ns_string(detail) {
            let () = msg_send![&*alert, setInformativeText: &*text];
        }
        // 程序不是 .app 包时，系统按英文显示勾选框，因此显式设置当前界面语言的文字。
        if let Some((text, checked)) = suppression
            && let Some(text) = ns_string(text)
        {
            let () = msg_send![&*alert, setShowsSuppressionButton: true];
            let checkbox: *mut AnyObject = msg_send![&*alert, suppressionButton];
            if let Some(checkbox) = checkbox.as_ref() {
                let () = msg_send![checkbox, setTitle: &*text];
                // NSControlStateValueOn = 1。
                let () = msg_send![checkbox, setState: isize::from(checked)];
            }
        }

        for spec in buttons {
            let Some(title) = ns_string(spec.title) else {
                continue;
            };
            let button: *mut AnyObject = msg_send![&*alert, addButtonWithTitle: &*title];
            let Some(button) = button.as_ref() else {
                continue;
            };

            if spec.destructive {
                let destructive = sel!(setHasDestructiveAction:);
                let can: bool = msg_send![button, respondsToSelector: destructive];
                if can {
                    let () = msg_send![button, setHasDestructiveAction: true];
                }
            }
            if let Some((key, modifiers)) = spec.key
                && let Some(key) = ns_string(key)
            {
                let () = msg_send![button, setKeyEquivalent: &*key];
                let () = msg_send![button, setKeyEquivalentModifierMask: modifiers];
            }
        }

        let keep = alert.clone();
        let handler = RcBlock::new(move |response: isize| {
            // 没有勾选框时 `suppressionButton` 仍然存在，只是不显示，读出的状态总是未勾选。
            // NSControlStateValueOn = 1。
            let checkbox: *mut AnyObject = msg_send![&*keep, suppressionButton];
            let checked = checkbox.as_ref().is_some_and(|checkbox| {
                let state: isize = msg_send![checkbox, state];
                state == 1
            });
            // NSAlertFirstButtonReturn = 1000，后续按钮依次加 1。
            if let Ok(index) = usize::try_from(response - 1000) {
                done(index, checked);
            }
        });
        let () = msg_send![
            &*alert,
            beginSheetModalForWindow: ns_window,
            completionHandler: &*handler
        ];
    }

    true
}

/// 把 `stroke` 转成菜单项的快捷键：AppKit 的键字符和修饰键位。
///
/// 无法表示的键（如小键盘键）返回 `None`。
pub fn key_equivalent(stroke: crate::keymap::Stroke) -> Option<(String, usize)> {
    use crate::keymap::{KeyCode, Mods};

    // 功能键使用 AppKit 定义在 Unicode 私有区的字符（如 `NSUpArrowFunctionKey` 为 U+F700）。
    let key: char = match stroke.key {
        KeyCode::Char(c) => c,
        KeyCode::Numpad(_) => return None,
        KeyCode::F(n) => char::from_u32(0xF704 + u32::from(n) - 1)?,
        KeyCode::Enter => '\r',
        KeyCode::Backspace => '\u{8}',
        KeyCode::Delete => '\u{F728}',
        KeyCode::Tab => '\t',
        KeyCode::Space => ' ',
        KeyCode::Escape => '\u{1b}',
        KeyCode::Up => '\u{F700}',
        KeyCode::Down => '\u{F701}',
        KeyCode::Left => '\u{F702}',
        KeyCode::Right => '\u{F703}',
        KeyCode::PageUp => '\u{F72C}',
        KeyCode::PageDown => '\u{F72D}',
    };
    let mut mods = 0;
    for (bit, mask) in [
        (Mods::CTRL, MOD_CONTROL),
        (Mods::SHIFT, MOD_SHIFT),
        (Mods::ALT, MOD_OPTION),
        (Mods::META, MOD_COMMAND),
    ] {
        if stroke.mods.has(bit) {
            mods |= mask;
        }
    }
    Some((key.to_string(), mods))
}

#[cfg(test)]
mod tests;
