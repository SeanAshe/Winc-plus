//! 运行状态：最近打开的目录、每个目录上次的现场、退出时开着的目录和项目自带工具的选择。
//!
//! 状态由程序维护，保存为 `state.json`。读取时容错：格式错误时返回空状态，类型不符或越界的项
//! 被丢弃，其余仍然读取。

use std::path::{Path, PathBuf};

use crate::json::Json;

/// 最近目录和各目录现场的最大数量（个）。
///
/// 取 10（经验值），作用于最近项目菜单、每个目录现场的保存数量和徽章颜色分配。
pub const MAX_RECENT: usize = 10;

/// 项目徽章的颜色数量（种）。
///
/// 取 10（经验值）：最近列表里的目录各用一种颜色，互不相同，因此 [`MAX_RECENT`] 不能超过
/// 该值。
pub const BADGE_COLORS: usize = 10;
const _: () = assert!(
    MAX_RECENT <= BADGE_COLORS,
    "最近列表中的每个目录必须使用不同的颜色"
);

/// 由程序维护的运行状态：最近目录、各目录的现场和项目工具选择。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    /// 最近打开的目录，最新的在前，不重复。
    pub recent: Vec<PathBuf>,
    /// 每个目录上次的现场，最近的在前，每个目录一条，最多 [`MAX_RECENT`] 条。
    pub sessions: Vec<Session>,
    /// 上次退出时开着的目录；退出时没有打开目录则为 `None`，下次启动显示开始页。
    pub last: Option<PathBuf>,
    /// 上次退出时其他窗口开着的目录（`last` 是当前窗口的），下次启动时各开一个窗口；没有
    /// 打开目录的窗口不记录。
    ///
    /// 只在退出时写入。
    pub others: Vec<PathBuf>,
    /// 提交视图的改动列表是否按目录排成树显示。
    ///
    /// `false`（默认）时平铺，每个文件一行。所有窗口和仓库共用，下次启动沿用。
    pub commit_tree: bool,
    /// PDF 预览是否隐藏页面缩略图栏；默认显示。所有窗口共用，下次启动沿用。
    pub pdf_thumbnails_hidden: bool,
    /// 侧栏宽度（逻辑像素）；`None` 表示没有调整过。所有视图、窗口和仓库共用，下次启动沿用。
    pub panel_width: Option<f32>,
    /// 能预览的文件（SVG、Markdown、表格）打开时的显示方式：`"split"`（分栏）或 `"preview"`
    /// （只看预览）；`None` 表示源代码。记录用户最近一次在标签栏右端选择的方式，下次启动
    /// 沿用。
    pub preview_mode: Option<String>,
    /// 上次关闭的窗口的大小和位置；没有记录时为 `None`。
    pub window: Option<WindowFrame>,
    /// 上次运行的程序的版本号（如 `0.1.626`）；没有记录时为 `None`。启动时与当前版本比较，
    /// 得知这是否为更新之后的第一次启动。
    pub version: Option<String>,
    /// 首次设置（第一次运行时开始页上的语言、外观、主题和模块）是否尚未完成：第一次运行时
    /// 置位，用户点击「完成」后清除；未完成时下次启动仍然显示。
    pub setup_pending: bool,
    /// 从磁盘映像等位置运行时不询问是否移到「应用程序」文件夹：用户曾选择不移动并勾选「不再
    /// 询问」。仅在 macOS 上使用。
    pub stay_outside_applications: bool,
    /// 最近列表里每个目录的徽章颜色（值在 `0..BADGE_COLORS` 之内），互不相同。
    ///
    /// 目录第一次进入最近列表时，选一个其他目录未使用的颜色，之后保持不变，使项目名旁和最近
    /// 项目菜单中始终显示同一种颜色。
    badges: Vec<(PathBuf, usize)>,
    /// 用户对项目自带格式化工具（位于 `node_modules/.bin` 等目录）的确认结果，最近的在前。
    ///
    /// 每项为（工具所在目录，是否运行该目录中的程序）；`false` 表示改用系统安装的程序。
    /// 最多 [`MAX_TOOL_CHOICES`] 项。
    tool_choices: Vec<(PathBuf, bool)>,
    /// 用户信任了配置的 Git 仓库（根目录）和当时配置的指纹，最近的在前，最多
    /// [`MAX_TOOL_CHOICES`] 项。
    ///
    /// 仓库的配置变了（指纹不同）要重新确认。
    trusted_repos: Vec<(PathBuf, u64)>,
    /// 用户为远端主机指定的密钥文件：（连接目标的文字，密钥文件的路径），按目标排序。
    ///
    /// 只记路径，不读取也不保存密钥的内容；路径原样交给 `ssh`。
    remote_keys: Vec<(String, String)>,
    /// Vim 模式的大写标记：名字、文件路径、行号与列号（都从 0 起），按名字排序。
    pub vim_marks: Vec<(char, PathBuf, usize, usize)>,
    /// Vim 模式命令行的历史，从旧到新，最多 `MAX_VIM_HISTORY` 条。
    pub vim_commands: Vec<String>,
    /// Vim 模式的查找历史，从旧到新，最多 `MAX_VIM_HISTORY` 条。
    pub vim_searches: Vec<String>,
}

/// Vim 模式的两种历史各自记录的条数上限（条）。
///
/// 取自调参：与输入框的历史同一量级。
const MAX_VIM_HISTORY: usize = 100;

/// 最多记录的项目自带工具选择数量（个）。
///
/// 取 50（经验值），作用于工具选择和已信任仓库两项列表。
const MAX_TOOL_CHOICES: usize = 50;

/// 窗口没有最大化、全屏时的大小和位置，以及关闭时是否最大化。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowFrame {
    /// 窗口外框左上角的（横，纵）坐标（物理像素）；系统不告诉应用窗口的位置时为 `None`。
    pub position: Option<(f64, f64)>,
    /// 内容区的宽度（逻辑像素）。
    pub width: f64,
    /// 内容区的高度（逻辑像素）。
    pub height: f64,
    /// 记录时窗口所在显示器的缩放比例，用于在物理像素和逻辑像素之间换算。
    pub scale: f64,
    /// 关闭时是否最大化。
    pub maximized: bool,
}

/// 一个目录的现场：按顺序排列的标签、活动标签的下标，以及各文件的书签。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    /// 目录路径。
    pub folder: PathBuf,
    /// 打开着的标签，按标签栏中的顺序排列。
    pub tabs: Vec<Tab>,
    /// 活动标签在 `tabs` 中的下标（从 0 起）。
    pub active: Option<usize>,
    /// 各文件的书签（行号从 0 起），包括当前未打开的文件。
    pub bookmarks: Vec<(PathBuf, Vec<usize>)>,
    /// 各文件的大纲中收起的符号，包括当前未打开的文件；没有收起符号的文件不记录。
    ///
    /// 每个符号记为从最外层上级到自身的名字，以 `\u{1f}`（单元分隔符）连接。
    pub outline: Vec<(PathBuf, Vec<String>)>,
    /// 各文件中折叠的块（开头行，末尾行；行号从 0 起），包括当前未打开的文件。
    pub folds: Vec<(PathBuf, Vec<(usize, usize)>)>,
    /// 各 PDF 文件的阅读位置，包括当前未打开的文件；位于文档开头的文件不记录。
    ///
    /// 位置记为视口顶边所在的页（页号从 0 起），以及视口顶边到该页顶边的距离与页高之比。
    pub pdf_places: Vec<(PathBuf, (usize, f32))>,
    /// 终端面板；终端里没有标签时为 `None`。
    pub terminal: Option<TerminalSession>,
    /// 选中的运行配置的名字。
    pub run: Option<String>,
    /// 侧栏是否收起；没有记录过的目录为 `false`，打开时显示文件树。
    pub sidebar_hidden: bool,
}

/// 现场里的终端面板：各标签的工作目录（按标签栏中的顺序）、正在看的标签、面板开着没有、大小。
///
/// 只恢复标签和目录，里面的程序不恢复。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TerminalSession {
    /// 各标签的工作目录；空路径表示打开的目录。
    pub tabs: Vec<PathBuf>,
    /// 各标签上用户起的名字，和 `tabs` 一一对应；空的是没起，比 `tabs` 短时后面的都没起。
    pub names: Vec<String>,
    /// 各标签从菜单选的 shell 的启动命令，和 `tabs` 一一对应；空的是默认 shell，比 `tabs` 短时
    /// 后面的都是默认 shell。
    pub shells: Vec<String>,
    /// 正在看的标签在 `tabs` 中的下标（从 0 起）。
    pub active: usize,
    /// 面板是否展开。
    pub open: bool,
    /// 面板在编辑区下面时的高度（逻辑像素），0 表示默认值。
    pub height: f32,
    /// 面板在编辑区右边时的宽度（逻辑像素），0 表示默认值。
    pub width: f32,
}

/// 现场里的一个标签：已保存的文件，或未保存文档的备份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    /// 已保存到磁盘的文件。
    File(PathBuf),
    /// 未保存过的新文件：内容保存在备份中，值为备份文件名。
    Untitled(String),
}

impl State {
    /// 读取指定文件中的状态；文件不存在、无法读取或格式不对时返回空状态。
    pub fn load_from(path: &Path) -> State {
        std::fs::read_to_string(path)
            .ok()
            .map(|text| State::parse(&text))
            .unwrap_or_default()
    }

    /// 解析状态文件内容；格式错误或顶层不是对象时返回空状态。
    ///
    /// 类型不符或越界的项被丢弃，其余仍然读取。
    pub fn parse(text: &str) -> State {
        let Some(root) = Json::parse(text) else {
            return State::default();
        };
        let Some(root) = root.as_object() else {
            return State::default();
        };

        let paths = |value: Option<&Json>| -> Vec<PathBuf> {
            value
                .and_then(Json::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Json::as_str)
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default()
        };

        let mut recent = Vec::new();
        for path in paths(root.get("recent")) {
            if !recent.contains(&path) && recent.len() < MAX_RECENT {
                recent.push(path);
            }
        }

        // 已保存的文件写成路径字符串，未保存过的新文件写成 `{"untitled": 备份文件名}`。
        let tab = |value: &Json| -> Option<Tab> {
            if let Some(path) = value.as_str() {
                return (!path.is_empty()).then(|| Tab::File(PathBuf::from(path)));
            }
            let name = value.as_object()?.get("untitled")?.as_str()?;
            (!name.is_empty()).then(|| Tab::Untitled(name.to_owned()))
        };
        let session = |value: &Json| -> Option<Session> {
            let s = value.as_object()?;
            let folder = PathBuf::from(s.get("folder")?.as_str().filter(|f| !f.is_empty())?);
            let tabs: Vec<Tab> = s
                .get("files")
                .and_then(Json::as_array)
                .unwrap_or_default()
                .iter()
                .filter_map(tab)
                .collect();
            let active = s
                .get("active")
                .and_then(Json::as_number)
                .filter(|n| *n >= 0.0 && n.fract() == 0.0)
                .map(|n| n as usize)
                .filter(|&i| i < tabs.len());
            let bookmarks = s
                .get("bookmarks")
                .and_then(Json::as_object)
                .map(|files| {
                    files
                        .iter()
                        .filter(|(path, _)| !path.is_empty())
                        .map(|(path, lines)| {
                            let lines = lines
                                .as_array()
                                .unwrap_or_default()
                                .iter()
                                .filter_map(Json::as_number)
                                .filter(|n| *n >= 0.0 && n.fract() == 0.0)
                                .map(|n| n as usize)
                                .collect();
                            (PathBuf::from(path), lines)
                        })
                        .filter(|(_, lines): &(PathBuf, Vec<usize>)| !lines.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let folds = s
                .get("folds")
                .and_then(Json::as_object)
                .map(|files| {
                    let line = |n: &Json| {
                        let n = n.as_number().filter(|n| *n >= 0.0 && n.fract() == 0.0)?;
                        Some(n as usize)
                    };
                    let block = |pair: &Json| match pair.as_array()? {
                        [start, end] => Some((line(start)?, line(end)?)),
                        _ => None,
                    };
                    files
                        .iter()
                        .filter(|(path, _)| !path.is_empty())
                        .map(|(path, blocks)| {
                            let blocks = blocks.as_array().unwrap_or_default();
                            let blocks = blocks.iter().filter_map(block);
                            (PathBuf::from(path), blocks.filter(|(s, e)| s < e).collect())
                        })
                        .filter(|(_, blocks): &(PathBuf, Vec<(usize, usize)>)| !blocks.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let outline = s
                .get("outline")
                .and_then(Json::as_object)
                .map(|files| {
                    files
                        .iter()
                        .filter(|(path, _)| !path.is_empty())
                        .map(|(path, symbols)| {
                            let symbols = symbols
                                .as_array()
                                .unwrap_or_default()
                                .iter()
                                .filter_map(Json::as_str)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                                .collect();
                            (PathBuf::from(path), symbols)
                        })
                        .filter(|(_, symbols): &(PathBuf, Vec<String>)| !symbols.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let pdf_places = s
                .get("pdfPlaces")
                .and_then(Json::as_object)
                .map(|files| {
                    let place = |place: &Json| match place.as_array()? {
                        [page, at] => {
                            let page = page.as_number()?;
                            let at = at.as_number().filter(|n| n.is_finite())?;
                            (page >= 0.0 && page.fract() == 0.0)
                                .then_some((page as usize, at as f32))
                        }
                        _ => None,
                    };
                    files
                        .iter()
                        .filter(|(path, _)| !path.is_empty())
                        .filter_map(|(path, at)| Some((PathBuf::from(path), place(at)?)))
                        .collect()
                })
                .unwrap_or_default();
            let terminal = s.get("terminal").and_then(Json::as_object).and_then(|t| {
                let number = |key| {
                    t.get(key)
                        .and_then(Json::as_number)
                        .filter(|n| n.is_finite() && *n >= 0.0)
                };
                let tabs: Vec<PathBuf> = t
                    .get("tabs")?
                    .as_array()?
                    .iter()
                    .filter_map(Json::as_str)
                    .map(PathBuf::from)
                    .collect();
                let shells = t.get("shells").and_then(Json::as_array).map(|shells| {
                    let shells = shells.iter().map(|s| s.as_str().unwrap_or_default());
                    shells.map(str::to_owned).collect()
                });
                let names = t.get("names").and_then(Json::as_array).map(|names| {
                    let names = names.iter().map(|n| n.as_str().unwrap_or_default());
                    names.map(str::to_owned).collect()
                });
                (!tabs.is_empty()).then(|| TerminalSession {
                    active: number("active").map_or(0, |n| (n as usize).min(tabs.len() - 1)),
                    tabs,
                    names: names.unwrap_or_default(),
                    shells: shells.unwrap_or_default(),
                    open: t.get("open").and_then(Json::as_bool).unwrap_or(false),
                    height: number("height").unwrap_or(0.0) as f32,
                    width: number("width").unwrap_or(0.0) as f32,
                })
            });
            Some(Session {
                folder,
                tabs,
                active,
                bookmarks,
                outline,
                folds,
                pdf_places,
                terminal,
                run: s.get("run").and_then(Json::as_str).map(str::to_owned),
                sidebar_hidden: (s.get("sidebarHidden").and_then(Json::as_bool)).unwrap_or(false),
            })
        };

        let badges = root
            .get("badges")
            .and_then(Json::as_object)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|(path, color)| {
                        let color = color.as_number().filter(|n| n.fract() == 0.0)?;
                        Some((PathBuf::from(path), color as usize))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let tool_choices = root
            .get("toolChoices")
            .and_then(Json::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(|item| {
                let item = item.as_object()?;
                let folder = item.get("folder")?.as_str().filter(|f| !f.is_empty())?;
                Some((PathBuf::from(folder), item.get("run")?.as_bool()?))
            })
            .take(MAX_TOOL_CHOICES)
            .collect();

        // 指纹是 64 位，存成十六进制字符串：JSON 的数字按 f64 读，精度不够，装不下。
        let trusted_repos = root
            .get("trustedRepos")
            .and_then(Json::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(|item| {
                let item = item.as_object()?;
                let folder = item.get("folder")?.as_str().filter(|f| !f.is_empty())?;
                let fingerprint =
                    u64::from_str_radix(item.get("fingerprint")?.as_str()?, 16).ok()?;
                Some((PathBuf::from(folder), fingerprint))
            })
            .take(MAX_TOOL_CHOICES)
            .collect();

        let remote_keys = root
            .get("remoteKeys")
            .and_then(Json::as_object)
            .map(|items| {
                (items.iter())
                    .filter_map(|(target, key)| Some((target.clone(), key.as_str()?.to_owned())))
                    .filter(|(_, key)| !key.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let vim_marks = vim_marks(root);
        let vim_commands = vim_history(root, "vimCommands");
        let vim_searches = vim_history(root, "vimSearches");

        let mut state = State {
            recent,
            badges,
            tool_choices,
            trusted_repos,
            remote_keys,
            vim_marks,
            vim_commands,
            vim_searches,
            ..State::default()
        };
        // 旧文件和手工修改过的文件里，颜色可能缺失、重复或越界；读入后统一理顺。
        state.assign_badges();
        for s in root
            .get("sessions")
            .and_then(Json::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(session)
        {
            if !state.sessions.iter().any(|old| old.folder == s.folder)
                && state.sessions.len() < MAX_RECENT
            {
                state.sessions.push(s);
            }
        }
        state.last = root
            .get("last")
            .and_then(Json::as_str)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        // 旧版曾默认排成树，只记录选了平铺（`commitFlat`）；默认平铺后该项不再读入。
        state.commit_tree = root
            .get("commitTree")
            .and_then(Json::as_bool)
            .unwrap_or(false);
        state.pdf_thumbnails_hidden = root
            .get("pdfThumbnailsHidden")
            .and_then(Json::as_bool)
            .unwrap_or(false);
        state.stay_outside_applications = root
            .get("stayOutsideApplications")
            .and_then(Json::as_bool)
            .unwrap_or(false);
        state.panel_width = root
            .get("panelWidth")
            .and_then(Json::as_number)
            .map(|width| width as f32);
        state.window = root
            .get("window")
            .and_then(Json::as_object)
            .and_then(window_frame);
        state.setup_pending = root.get("setupPending").and_then(Json::as_bool) == Some(true);
        state.version = root
            .get("version")
            .and_then(Json::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        state.preview_mode = root
            .get("previewMode")
            .and_then(Json::as_str)
            .filter(|m| matches!(*m, "split" | "preview"))
            .map(str::to_owned);
        for path in paths(root.get("otherWindows")) {
            if !state.others.contains(&path) && state.others.len() < MAX_RECENT {
                state.others.push(path);
            }
        }

        // 旧格式只记单个 `session`，即上次退出时的目录。
        if let Some(old) = root.get("session").and_then(session)
            && state.sessions.is_empty()
        {
            state.last = Some(old.folder.clone());
            state.sessions.push(old);
        }

        state
    }

    /// 返回上次退出时开着的目录的现场，启动时据此恢复；没有时返回 `None`。
    pub fn last_session(&self) -> Option<&Session> {
        self.session_for(self.last.as_deref()?)
    }

    /// 返回目录 `folder` 上次的现场；没有记录时返回 `None`。
    pub fn session_for(&self, folder: &Path) -> Option<&Session> {
        self.sessions.iter().find(|s| s.folder == folder)
    }

    /// 记录一个目录的现场并移到最前面。
    ///
    /// 同目录的旧记录被替换，超出 [`MAX_RECENT`] 的从末尾移除。
    pub fn set_session(&mut self, session: Session) {
        self.sessions.retain(|s| s.folder != session.folder);
        self.sessions.insert(0, session);
        self.sessions.truncate(MAX_RECENT);
    }

    /// 把 Vim 模式的大写标记与两种历史加入状态文件的字段 `fields`，为空的不加。
    fn vim_fields(&self, fields: &mut Vec<(&'static str, Json)>) {
        if !self.vim_marks.is_empty() {
            fields.push(("vimMarks", vim_marks_json(&self.vim_marks)));
        }
        for (key, history) in [
            ("vimCommands", &self.vim_commands),
            ("vimSearches", &self.vim_searches),
        ] {
            if !history.is_empty() {
                let entries = history.iter().map(|entry| Json::String(entry.clone()));
                fields.push((key, Json::Array(entries.collect())));
            }
        }
    }

    /// 序列化为 `state.json` 的内容，末尾带一个换行符。
    ///
    /// `last`、`previewMode` 没有值，`otherWindows`、`toolChoices`、`trustedRepos`、`vimMarks`、
    /// `vimCommands`、`vimSearches` 为空，`commitTree`、`pdfThumbnailsHidden`、
    /// `stayOutsideApplications` 为假时省略对应的键。
    pub fn to_json(&self) -> String {
        let path = |p: &PathBuf| Json::String(p.to_string_lossy().into_owned());
        let mut fields = vec![(
            "recent",
            Json::Array(self.recent.iter().map(path).collect()),
        )];

        if let Some(last) = &self.last {
            fields.push(("last", path(last)));
        }
        if !self.others.is_empty() {
            fields.push((
                "otherWindows",
                Json::Array(self.others.iter().map(path).collect()),
            ));
        }

        if self.commit_tree {
            fields.push(("commitTree", Json::Bool(true)));
        }
        if self.pdf_thumbnails_hidden {
            fields.push(("pdfThumbnailsHidden", Json::Bool(true)));
        }
        if self.stay_outside_applications {
            fields.push(("stayOutsideApplications", Json::Bool(true)));
        }
        if let Some(width) = self.panel_width {
            fields.push(("panelWidth", Json::Number(f64::from(width))));
        }
        if let Some(mode) = &self.preview_mode {
            fields.push(("previewMode", Json::String(mode.clone())));
        }
        if let Some(version) = &self.version {
            fields.push(("version", Json::String(version.clone())));
        }
        if self.setup_pending {
            fields.push(("setupPending", Json::Bool(true)));
        }
        if let Some(frame) = &self.window {
            let mut window = vec![
                ("width", Json::Number(frame.width)),
                ("height", Json::Number(frame.height)),
                ("scale", Json::Number(frame.scale)),
                ("maximized", Json::Bool(frame.maximized)),
            ];
            if let Some((x, y)) = frame.position {
                window.push(("x", Json::Number(x)));
                window.push(("y", Json::Number(y)));
            }
            fields.push(("window", Json::object(window)));
        }

        let badges = self.badges.iter().map(|(folder, color)| {
            let folder = folder.to_string_lossy().into_owned();
            (folder, Json::Number(*color as f64))
        });
        fields.push(("badges", Json::object(badges)));

        let tab = |tab: &Tab| match tab {
            Tab::File(file) => path(file),
            Tab::Untitled(name) => Json::object(vec![("untitled", Json::String(name.clone()))]),
        };
        let sessions = self.sessions.iter().map(|session| {
            let mut items = vec![
                ("folder", path(&session.folder)),
                ("files", Json::Array(session.tabs.iter().map(tab).collect())),
            ];
            if let Some(active) = session.active {
                items.push(("active", Json::Number(active as f64)));
            }
            if !session.bookmarks.is_empty() {
                let files = session.bookmarks.iter().map(|(file, lines)| {
                    let lines = lines.iter().map(|&l| Json::Number(l as f64)).collect();
                    (file.to_string_lossy().into_owned(), Json::Array(lines))
                });
                items.push(("bookmarks", Json::Object(files.collect())));
            }
            if !session.folds.is_empty() {
                let files = session.folds.iter().map(|(file, blocks)| {
                    let pair = |&(start, end): &(usize, usize)| {
                        Json::Array(vec![Json::Number(start as f64), Json::Number(end as f64)])
                    };
                    let blocks = blocks.iter().map(pair).collect();
                    (file.to_string_lossy().into_owned(), Json::Array(blocks))
                });
                items.push(("folds", Json::Object(files.collect())));
            }
            if !session.outline.is_empty() {
                let files = session.outline.iter().map(|(file, symbols)| {
                    let symbols = symbols.iter().cloned().map(Json::String).collect();
                    (file.to_string_lossy().into_owned(), Json::Array(symbols))
                });
                items.push(("outline", Json::Object(files.collect())));
            }
            if !session.pdf_places.is_empty() {
                // 比例保留四位小数：一页高 10000 逻辑像素时误差为 1 逻辑像素。
                let files = session.pdf_places.iter().map(|(file, (page, at))| {
                    let at = (f64::from(*at) * 1e4).round() / 1e4;
                    let place = vec![Json::Number(*page as f64), Json::Number(at)];
                    (file.to_string_lossy().into_owned(), Json::Array(place))
                });
                items.push(("pdfPlaces", Json::Object(files.collect())));
            }
            if let Some(t) = &session.terminal {
                let number = |n: f32| Json::Number(f64::from(n));
                let mut fields = vec![
                    ("tabs", Json::Array(t.tabs.iter().map(path).collect())),
                    ("active", Json::Number(t.active as f64)),
                    ("open", Json::Bool(t.open)),
                    ("height", number(t.height)),
                    ("width", number(t.width)),
                ];
                // 都没起名字时不写。
                if t.names.iter().any(|n| !n.is_empty()) {
                    let names = t.names.iter().map(|n| Json::String(n.clone()));
                    fields.push(("names", Json::Array(names.collect())));
                }
                // 都是默认 shell 时不写。
                if t.shells.iter().any(|s| !s.is_empty()) {
                    let shells = t.shells.iter().map(|s| Json::String(s.clone()));
                    fields.push(("shells", Json::Array(shells.collect())));
                }
                items.push(("terminal", Json::object(fields)));
            }
            if let Some(run) = &session.run {
                items.push(("run", Json::String(run.clone())));
            }
            // 展开是默认状态，不写。
            if session.sidebar_hidden {
                items.push(("sidebarHidden", Json::Bool(true)));
            }
            Json::object(items)
        });
        fields.push(("sessions", Json::Array(sessions.collect())));

        if !self.remote_keys.is_empty() {
            let keys = (self.remote_keys.iter())
                .map(|(target, key)| (target.clone(), Json::String(key.clone())));
            fields.push(("remoteKeys", Json::object(keys)));
        }
        if !self.tool_choices.is_empty() {
            let choices = self.tool_choices.iter().map(|(folder, run)| {
                Json::object(vec![("folder", path(folder)), ("run", Json::Bool(*run))])
            });
            fields.push(("toolChoices", Json::Array(choices.collect())));
        }
        if !self.trusted_repos.is_empty() {
            let repos = self.trusted_repos.iter().map(|(folder, fingerprint)| {
                Json::object(vec![
                    ("folder", path(folder)),
                    ("fingerprint", Json::String(format!("{fingerprint:016x}"))),
                ])
            });
            fields.push(("trustedRepos", Json::Array(repos.collect())));
        }
        self.vim_fields(&mut fields);

        let mut out = Json::object(fields).write(0);
        out.push('\n');
        out
    }

    /// 原子地写入 `path`：先写临时文件再改名。
    ///
    /// 与 [`Config::save_to`](crate::Config::save_to) 相同，不提供不带路径的写法，避免测试写入
    /// 真实文件。
    ///
    /// # Errors
    ///
    /// 目录创建失败、没有写权限或磁盘错误时返回对应的 `std::io::Error`。
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        crate::files::write_atomic(path, &self.to_json())
    }

    /// 读出 `path` 中现在的状态，用 `change` 修改后写回，返回修改后的状态和写入的结果。
    ///
    /// 多个 winc 进程（各开一个目录）共用一个状态文件，每次写都重新读一遍再改，避免旧副本
    /// 覆盖其他进程写入的最近列表、现场和书签；读改写期间锁住旁边的 `<文件>.lock`（建议锁），
    /// 两个进程不会交错。锁文件打不开时仍然读改写。
    ///
    /// # Errors
    ///
    /// 写回失败时返回对应的 `std::io::Error`。
    pub fn update(path: &Path, change: impl FnOnce(&mut State)) -> (State, std::io::Result<()>) {
        let lock = path.parent().and_then(|dir| {
            std::fs::create_dir_all(dir).ok()?;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path.with_extension("lock"))
                .ok()?;
            file.lock().ok()?;
            Some(file)
        });
        let mut state = State::load_from(path);
        change(&mut state);
        let result = state.save_to(path);
        drop(lock);
        (state, result)
    }

    /// 记录打开了一个目录：移到最前面，超出 [`MAX_RECENT`] 的从末尾移除。
    ///
    /// 首次进入最近列表的目录会被分配一个徽章颜色。
    pub fn remember(&mut self, folder: &Path) {
        self.recent.retain(|p| p != folder);
        self.recent.insert(0, folder.to_path_buf());
        self.recent.truncate(MAX_RECENT);
        self.assign_badges();
    }

    /// 从最近列表中移除目录 `folder`（目录已不存在，或用户选择移除）。
    ///
    /// 同时丢弃该目录的现场；该目录是上次退出时的目录时，清空 `last`。其徽章颜色被释放，
    /// 供之后的目录使用。
    pub fn forget(&mut self, folder: &Path) {
        self.recent.retain(|p| p != folder);
        self.sessions.retain(|s| s.folder != folder);
        if self.last.as_deref() == Some(folder) {
            self.last = None;
        }
        self.assign_badges();
    }

    /// 返回用户为连接目标 `target` 指定的密钥文件；没有指定时返回 `None`。
    pub fn remote_key(&self, target: &str) -> Option<&str> {
        (self.remote_keys.iter())
            .find(|(known, _)| known == target)
            .map(|(_, key)| key.as_str())
    }

    /// 记下连接目标 `target` 使用的密钥文件 `key`；`key` 为空时去掉这条记录。
    pub fn set_remote_key(&mut self, target: &str, key: &str) {
        self.remote_keys.retain(|(known, _)| known != target);
        if !key.is_empty() {
            self.remote_keys.push((target.to_owned(), key.to_owned()));
            self.remote_keys.sort();
        }
    }

    /// 返回 `folder` 中项目自带格式化工具的上次选择：运行该目录中的程序（`true`），
    /// 或使用系统安装的程序（`false`）；没有记录时返回 `None`。
    pub fn tool_choice(&self, folder: &Path) -> Option<bool> {
        self.tool_choices
            .iter()
            .find(|(p, _)| p == folder)
            .map(|&(_, run)| run)
    }

    /// 记录对 `folder` 中项目自带工具的选择并移到最前面，超出 `MAX_TOOL_CHOICES` 的从末尾移除。
    pub fn set_tool_choice(&mut self, folder: &Path, run: bool) {
        self.tool_choices.retain(|(p, _)| p != folder);
        self.tool_choices.insert(0, (folder.to_path_buf(), run));
        self.tool_choices.truncate(MAX_TOOL_CHOICES);
    }

    /// 返回用户信任 Git 仓库 `root` 的配置时记下的指纹；没有信任过时返回 `None`。
    pub fn trusted_repo(&self, root: &Path) -> Option<u64> {
        self.trusted_repos
            .iter()
            .find(|(p, _)| p == root)
            .map(|&(_, fingerprint)| fingerprint)
    }

    /// 记下用户信任了指纹为 `fingerprint` 的 Git 仓库 `root` 配置，移到最前面。
    pub fn trust_repo(&mut self, root: &Path, fingerprint: u64) {
        self.trusted_repos.retain(|(p, _)| p != root);
        self.trusted_repos
            .insert(0, (root.to_path_buf(), fingerprint));
        self.trusted_repos.truncate(MAX_TOOL_CHOICES);
    }

    /// 返回目录 `folder` 的徽章颜色（值在 `0..BADGE_COLORS` 之内）；不在最近列表中时按名字
    /// 计算一个。
    pub fn badge_color(&self, folder: &Path) -> usize {
        self.badges
            .iter()
            .find(|(p, _)| p == folder)
            .map_or_else(|| preferred_color(folder), |&(_, color)| color)
    }

    /// 使最近列表里的目录各有一个互不相同的颜色。
    ///
    /// 已有的颜色保持不变，新来的目录从按名字算出的颜色起向后取第一个未占用的颜色；列表
    /// 最多 [`MAX_RECENT`] 个，且 `MAX_RECENT <= BADGE_COLORS`，颜色一定够用。
    fn assign_badges(&mut self) {
        // 先保留已有的颜色，再给新来的目录分配：新来的目录排在列表最前面，先分配会占用其他
        // 目录已有的颜色。
        let mut taken = [false; BADGE_COLORS];
        let mut colors: Vec<Option<usize>> = self
            .recent
            .iter()
            .map(|folder| {
                let color = self
                    .badges
                    .iter()
                    .find(|(p, _)| p == folder)
                    .map(|&(_, color)| color)
                    .filter(|&color| color < BADGE_COLORS && !taken[color])?;
                taken[color] = true;
                Some(color)
            })
            .collect();

        for (folder, color) in self.recent.iter().zip(&mut colors) {
            if color.is_none() {
                let start = preferred_color(folder);
                let free = (0..BADGE_COLORS)
                    .map(|i| (start + i) % BADGE_COLORS)
                    .find(|&c| !taken[c])
                    .unwrap_or(start);
                taken[free] = true;
                *color = Some(free);
            }
        }

        self.badges = self
            .recent
            .iter()
            .cloned()
            .zip(colors.into_iter().flatten())
            .collect();
    }
}

/// 返回按目录名计算的颜色：32 位 FNV-1a 哈希对 `BADGE_COLORS` 取余。
///
/// 选用 FNV-1a 是因为其结果在各平台、各版本间一致，标准库的哈希不保证这一点。
fn preferred_color(folder: &Path) -> usize {
    let name = folder.file_name().unwrap_or(folder.as_os_str());
    let hash = name
        .as_encoded_bytes()
        .iter()
        .fold(0x811c_9dc5_u32, |h, &b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
    hash as usize % BADGE_COLORS
}

/// 返回状态文件的路径；无法确定用户目录时返回 `None`。
///
/// 各平台按自己的约定：
///
/// - macOS：`~/Library/Application Support/winc/state.json`
/// - Linux：`$XDG_STATE_HOME/winc/state.json`，未设置时为 `~/.local/state/winc/state.json`
/// - Windows：`%LOCALAPPDATA%\winc\state.json`（不随账户漫游：内容都是本机路径）
///
/// 便携版放在便携数据目录中（见 [`crate::files::portable_dir`]）。
pub fn state_path() -> Option<PathBuf> {
    let dir = crate::files::portable_dir().or_else(user_state_dir)?;
    Some(dir.join("state.json"))
}

/// 返回用户目录中存放状态文件的目录，不考虑便携版；无法确定用户目录时返回 `None`。
pub(crate) fn user_state_dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);

    if cfg!(target_os = "macos") {
        crate::files::user_config_dir()
    } else if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(home)
            .map(|dir| dir.join("winc"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".local").join("state")))
            .map(|dir| dir.join("winc"))
    }
}

/// 从状态文件的根对象读出 `key` 下 Vim 模式的历史：非空的字符串，只留最新的
/// [`MAX_VIM_HISTORY`] 条。
fn vim_history(root: &std::collections::BTreeMap<String, Json>, key: &str) -> Vec<String> {
    let entries = root.get(key).and_then(Json::as_array).unwrap_or_default();
    let mut history: Vec<String> = entries
        .iter()
        .filter_map(|entry| entry.as_str().filter(|text| !text.is_empty()))
        .map(str::to_owned)
        .collect();
    history.split_off(history.len().saturating_sub(MAX_VIM_HISTORY))
}

/// 返回 Vim 模式的大写标记在状态文件中的写法。
fn vim_marks_json(marks: &[(char, PathBuf, usize, usize)]) -> Json {
    let items = marks.iter().map(|(name, path, line, col)| {
        Json::object(vec![
            ("name", Json::String(name.to_string())),
            ("path", Json::String(path.to_string_lossy().into_owned())),
            ("line", Json::Number(*line as f64)),
            ("col", Json::Number(*col as f64)),
        ])
    });
    Json::Array(items.collect())
}

/// 从状态文件的根对象读出 Vim 模式的大写标记：按名字排序，同名的只留一个。
fn vim_marks(
    root: &std::collections::BTreeMap<String, Json>,
) -> Vec<(char, PathBuf, usize, usize)> {
    let mut marks: Vec<(char, PathBuf, usize, usize)> = root
        .get("vimMarks")
        .and_then(Json::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| {
            let item = item.as_object()?;
            let mut name = item.get("name")?.as_str()?.chars();
            let name = name
                .next()
                .filter(|c| c.is_ascii_uppercase() && name.next().is_none())?;
            let path = item.get("path")?.as_str().filter(|p| !p.is_empty())?;
            let number = |key: &str| Some(item.get(key)?.as_number()?.max(0.0) as usize);
            Some((name, PathBuf::from(path), number("line")?, number("col")?))
        })
        .collect();
    marks.sort();
    marks.dedup_by_key(|mark| mark.0);
    marks
}

/// 从 `state.json` 的 `window` 一节读出窗口的大小和位置；缺项或者数值不合理时为 `None`。
fn window_frame(json: &std::collections::BTreeMap<String, Json>) -> Option<WindowFrame> {
    let number = |key: &str| json.get(key).and_then(Json::as_number);
    let (width, height, scale) = (number("width")?, number("height")?, number("scale")?);
    (width > 0.0 && height > 0.0 && scale > 0.0).then_some(WindowFrame {
        position: number("x").zip(number("y")),
        width,
        height,
        scale,
        maximized: json
            .get("maximized")
            .and_then(Json::as_bool)
            .unwrap_or(false),
    })
}

#[cfg(test)]
mod tests;
