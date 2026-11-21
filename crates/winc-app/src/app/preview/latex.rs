//! LaTeX 的预览：用本机的 TeX 编译并显示 PDF，编译不了时显示转换出的阅读视图。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, mpsc};

use winc_core::Lines;
use winc_editor::format::Cancel;
use winc_editor::latex::{self, Compiled, Job, Missing, Problem, Progress, Toolchain};
use winc_editor::{DocumentId, PreviewKind, PreviewMode};
use winc_i18n::t;
use winc_preview::synctex::Synctex;

use crate::app::animation::{Clock, LATEX_STATUS_MOTION};
use crate::app::input::context_menu::{MenuLabel, MenuTarget};
use crate::app::{App, UserEvent};

/// 找主文件时向上看的目录层数。
const ROOT_SEARCH_DEPTH: usize = 3;
/// 找主文件时读的 `.tex` 文件最大字节数。
const MAX_CANDIDATE_BYTES: usize = 1 << 20;
/// 错误列表的菜单最多列出的项数。
const MENU_PROBLEMS: usize = 20;
/// 行号栏里错误红点的点击范围：中心左右各多宽（逻辑像素）。
const PROBLEM_HIT: f32 = 7.0;
/// 分栏对齐时基线往上让出的距离（bp）。
const SYNC_LEAD: f32 = 12.0;

/// 各文档的编译。
pub(crate) struct LatexBuilds {
    /// 找到的 TeX 程序；还没找到时为 `None`。
    tool: Option<Toolchain>,
    /// 上一次找的时候是否没有找到。
    missing: bool,
    /// 设置的程序改过几次，旧设置下的结果不再记下。
    generation: u64,
    /// 正在为设置页找程序时，按第几次的设置找。
    finding: Option<u64>,
    /// 上一帧设置是否开着。
    settings_open: bool,
    /// 显示过预览的各文档的编译。
    builds: HashMap<DocumentId, Build>,
    /// 最近一次编译的序号。
    serial: u64,
    sender: mpsc::Sender<Done>,
    receiver: mpsc::Receiver<Done>,
    /// 上一次对齐时的（文档、PDF 的序号、左侧的滚动位置）。
    synced: Option<(DocumentId, u64, u64)>,
    /// 右上角编译状态的宽度（逻辑像素）；没有显示时为 `None`。
    status_width: Option<f32>,
    /// 编译状态换了文字后宽度的过渡：（起始宽度、目标宽度、时钟）。
    pub(crate) status_motion: Option<(f32, f32, Clock)>,
}

impl Default for LatexBuilds {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        LatexBuilds {
            tool: None,
            missing: false,
            generation: 0,
            finding: None,
            settings_open: false,
            builds: HashMap::new(),
            serial: 0,
            sender,
            receiver,
            synced: None,
            status_width: None,
            status_motion: None,
        }
    }
}

impl LatexBuilds {
    /// 取消文档 `id` 的编译并丢掉结果。
    pub(crate) fn forget(&mut self, id: DocumentId) {
        if let Some(build) = self.builds.remove(&id)
            && let Some((_, cancel)) = build.running
        {
            cancel.cancel();
        }
    }

    /// 是否有编译正在进行。
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn running(&self) -> bool {
        self.builds.values().any(|b| b.running.is_some())
    }

    /// 设置的 TeX 程序改了，丢掉找到的程序。
    pub(crate) fn forget_tool(&mut self) {
        self.generation += 1;
        self.tool = None;
        self.missing = false;
    }
}

/// 一个文档的编译。
#[derive(Default)]
struct Build {
    /// 正在进行的编译的序号和取消标记，以及它进行到哪一步。
    running: Option<(u64, Cancel)>,
    progress: Option<Progress>,
    /// 是否完成过一次编译。
    done: bool,
    /// 结果是否可能过时。
    stale: bool,
    /// 最近一次可以显示的 PDF 和它的序号。
    pdf: Option<(PathBuf, u64)>,
    synctex: Option<Arc<Synctex>>,
    /// 最近一次编译的错误，和下一次点击跳到第几个。
    problems: Vec<Problem>,
    next: usize,
    /// 编译没能完成又读不出错误时的原因。
    failure: Option<String>,
}

/// 后台完成的工作。
enum Done {
    /// 为设置页找到的程序，和找的时候设置改过几次。
    Found(u64, Result<Toolchain, Missing>),
    /// 文档、编译的序号和进行到的一步。
    Progress(DocumentId, u64, Progress),
    Built(Box<Built>),
}

/// 后台完成的一次编译。
struct Built {
    doc: DocumentId,
    serial: u64,
    /// 用的程序，和开始编译时设置改过几次。
    tool: Result<Toolchain, Missing>,
    generation: u64,
    job: Option<Job>,
    compiled: Option<Compiled>,
    synctex: Option<Synctex>,
}

impl App {
    /// 返回当前标签里能编译成 PDF 的 LaTeX 文档和它的路径。
    fn latex_doc(&self) -> Option<(DocumentId, PathBuf)> {
        let doc = self.editor.active()?;
        if doc.preview_kind() != Some(PreviewKind::Latex) || !winc_preview::pdf::supported() {
            return None;
        }
        let path = doc.path.clone().filter(|p| latex::compiles_here(p))?;
        Some((doc.id, path))
    }

    /// 当前标签显示编译出的 PDF 时，返回文档、PDF 的路径和序号。
    pub(crate) fn latex_pdf(&self) -> Option<(DocumentId, PathBuf, u64)> {
        let doc = self.editor.active()?;
        let shown = doc.preview_kind() == Some(PreviewKind::Latex)
            && doc.preview_mode() != PreviewMode::Source;
        let (path, serial) = self.preview.latex.builds.get(&doc.id)?.pdf.clone()?;
        shown.then_some((doc.id, path, serial))
    }

    /// 按需开始编译，设置刚打开时重新找 TeX 程序；每次绘制前调用。
    pub(crate) fn advance_latex(&mut self) {
        let open = self.settings.open;
        if open && !std::mem::replace(&mut self.preview.latex.settings_open, true) {
            self.find_latex_tool();
        }
        self.preview.latex.settings_open = open;
        let Some((id, _)) = self.latex_doc() else {
            return;
        };
        if self.editor.active().map(|d| d.preview_mode()) == Some(PreviewMode::Source) {
            return;
        }
        let build = self.preview.latex.builds.get(&id);
        if build.is_none_or(|b| b.stale && b.running.is_none()) {
            self.build_latex(id);
        }
    }

    /// 文档 `id` 保存后，`.tex` 文件的预览重新编译。
    pub(in crate::app) fn latex_saved(&mut self, id: DocumentId) {
        let tex = self.editor.document(id).and_then(|d| d.preview_kind());
        if tex != Some(PreviewKind::Latex) {
            return;
        }
        for build in self.preview.latex.builds.values_mut() {
            build.stale = true;
        }
        if let Some((active, _)) = self.latex_doc()
            && self.preview.latex.builds.contains_key(&active)
            && self.editor.active().map(|d| d.preview_mode()) != Some(PreviewMode::Source)
        {
            self.build_latex(active);
        }
    }

    /// 在后台编译文档 `id`。
    fn build_latex(&mut self, id: DocumentId) {
        let Some(doc) = self.editor.document(id) else {
            return;
        };
        let (Some(path), Some(rope)) = (doc.path.clone(), doc.text().rope().cloned()) else {
            return;
        };
        let shell = self.format.formatter.shell().clone();
        let setting = self.config.latex_path.clone();
        let folder = self.editor.root().map(Path::to_path_buf);
        let cache = self.latex_cache();
        let proxy = self.link.proxy.clone();
        let latex = &mut self.preview.latex;
        latex.serial += 1;
        let serial = latex.serial;
        let build = latex.builds.entry(id).or_default();
        if let Some((_, cancel)) = build.running.take() {
            cancel.cancel();
        }
        let cancel = Cancel::default();
        build.running = Some((serial, cancel.clone()));
        build.progress = None;
        build.stale = false;
        let (tool, sender, generation) =
            (latex.tool.clone(), latex.sender.clone(), latex.generation);
        // 进度在读取输出的线程上送来，和上一步相同的不送。
        let report = Mutex::new((sender.clone(), proxy.clone(), None));
        let progress = move |step: Progress| {
            let Ok(mut report) = report.lock() else {
                return;
            };
            let (sender, proxy, last) = &mut *report;
            if last.as_ref() == Some(&step) {
                return;
            }
            *last = Some(step.clone());
            if sender.send(Done::Progress(id, serial, step)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::Latex);
            }
        };
        winc_editor::background::spawn(move || {
            let text = rope.to_string();
            let tool = tool.map_or_else(|| latex::find(&shell, setting.as_deref()), Ok);
            let mut built = Built {
                doc: id,
                serial,
                tool: tool.clone(),
                generation,
                job: None,
                compiled: None,
                synctex: None,
            };
            if let Ok(tool) = &tool
                && let Some((job, compiled)) = compile(
                    &shell,
                    tool,
                    &path,
                    &text,
                    (folder.as_deref(), &cache),
                    &cancel,
                    progress,
                )
            {
                built.synctex = compiled.pdf.then(|| read_synctex(&job)).flatten();
                (built.job, built.compiled) = (Some(job), Some(compiled));
            }
            if cancel.is_cancelled() {
                return;
            }
            if sender.send(Done::Built(Box::new(built))).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::Latex);
            }
        });
    }

    /// 在后台找 TeX 程序。
    pub(in crate::app) fn find_latex_tool(&mut self) {
        let latex = &mut self.preview.latex;
        let generation = latex.generation;
        if latex.finding == Some(generation) {
            return;
        }
        latex.finding = Some(generation);
        let (sender, proxy) = (latex.sender.clone(), self.link.proxy.clone());
        let shell = self.format.formatter.shell().clone();
        let setting = self.config.latex_path.clone();
        winc_editor::background::spawn(move || {
            let found = latex::find(&shell, setting.as_deref());
            if sender.send(Done::Found(generation, found)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::Latex);
            }
        });
    }

    /// 返回设置里「LaTeX 程序」留空时显示的文字。
    pub(crate) fn latex_tool_label(&self) -> String {
        let latex = &self.preview.latex;
        let name = |path: &Path| path.file_name().map(|n| n.to_string_lossy().into_owned());
        let found = match &latex.tool {
            Some(Toolchain::Latexmk(path) | Toolchain::Tectonic(path)) => name(path),
            Some(Toolchain::Engines(engines)) => engines.first().and_then(|(_, p)| name(p)),
            None => None,
        };
        match found {
            Some(program) => t!("settings-latex-path-found", program = program),
            None if latex.missing => t!("settings-latex-path-missing").to_owned(),
            None => t!("settings-latex-path-auto").to_owned(),
        }
    }

    /// 返回编译产物的存放处：系统缓存目录下的 `winc`；没有状态文件时在临时目录。
    fn latex_cache(&self) -> PathBuf {
        if self.store.state_path.is_none() {
            return std::env::temp_dir();
        }
        let var = |key: &str| {
            let value = std::env::var_os(key).filter(|v| !v.is_empty());
            value.map(PathBuf::from)
        };
        let base = if cfg!(target_os = "macos") {
            var("HOME").map(|home| home.join("Library/Caches"))
        } else if cfg!(windows) {
            var("LOCALAPPDATA")
        } else {
            var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))
        };
        base.map_or_else(std::env::temp_dir, |base| base.join("winc"))
    }

    /// 收下后台完成的编译。
    pub(crate) fn poll_latex(&mut self) {
        let latex = &mut self.preview.latex;
        let mut changed = false;
        while let Ok(done) = latex.receiver.try_recv() {
            let built = match done {
                Done::Found(generation, found) => {
                    if generation == latex.generation {
                        latex.finding = None;
                        latex.missing = found.is_err();
                        latex.tool = found.ok();
                        changed = true;
                    }
                    continue;
                }
                Done::Progress(id, serial, step) => {
                    if let Some(build) = latex.builds.get_mut(&id)
                        && build.running.as_ref().is_some_and(|(s, _)| *s == serial)
                    {
                        build.progress = Some(step);
                        changed = true;
                    }
                    continue;
                }
                Done::Built(built) => *built,
            };
            if built.generation == latex.generation {
                latex.missing = built.tool.is_err();
                if let Ok(tool) = &built.tool {
                    latex.tool = Some(tool.clone());
                }
            }
            let Some(build) = latex.builds.get_mut(&built.doc) else {
                continue;
            };
            if build.running.as_ref().map(|(serial, _)| *serial) != Some(built.serial) {
                continue;
            }
            build.running = None;
            build.progress = None;
            build.done = true;
            changed = true;
            let (Some(job), Some(compiled)) = (built.job, built.compiled) else {
                // 没有 TeX 或主文件时不提示。
                build.problems.clear();
                build.failure = None;
                continue;
            };
            if compiled.pdf {
                build.pdf = Some((job.pdf, built.serial));
                build.synctex = built.synctex.map(Arc::new);
            }
            build.problems = compiled.problems;
            build.failure = compiled.failure;
            build.next = 0;
        }
        if changed {
            self.request_redraw();
        }
    }

    /// 返回当前标签的编译状态和它是否是错误。
    pub(crate) fn latex_status(&self) -> Option<(String, bool)> {
        let (id, _) = self.latex_doc()?;
        if self.editor.active()?.preview_mode() == PreviewMode::Source {
            return None;
        }
        let build = self.preview.latex.builds.get(&id)?;
        if build.running.is_some() {
            let label = match &build.progress {
                Some(Progress::Download(name)) => t!("latex-progress-download", name = name),
                Some(Progress::Pass(pass)) => t!("latex-progress-pass", pass = *pass),
                Some(Progress::Page(page)) => t!("latex-progress-page", page = *page),
                None => t!("latex-compiling").to_owned(),
            };
            return Some((label, false));
        }
        match (build.problems.len(), &build.failure) {
            (0, None) => None,
            (0, Some(_)) => Some((t!("latex-failed").to_owned(), true)),
            (n, _) => Some((t!("latex-problems", count = n), true)),
        }
    }

    /// 返回编译状态这一帧的宽度，宽度 `target` 变了时从当前宽度过渡过去。
    pub(crate) fn latex_status_width(&mut self, target: f32) -> f32 {
        let latex = &self.preview.latex;
        let shown = match latex.status_motion {
            Some((from, to, clock)) => clock.between(from, to),
            None => latex.status_width.unwrap_or(target),
        };
        if latex.status_width != Some(target) {
            let appearing = latex.status_width.is_none();
            let motion = match appearing {
                true => None,
                false => self.clock(LATEX_STATUS_MOTION).map(|c| (shown, target, c)),
            };
            let animated = motion.is_some();
            let latex = &mut self.preview.latex;
            (latex.status_width, latex.status_motion) = (Some(target), motion);
            if !animated {
                return target;
            }
        }
        shown
    }

    /// 编译状态不显示了。
    pub(crate) fn hide_latex_status(&mut self) {
        let latex = &mut self.preview.latex;
        (latex.status_width, latex.status_motion) = (None, None);
    }

    /// 是否在等第一次编译的结果。
    pub(crate) fn latex_waiting(&self) -> bool {
        let Some((id, _)) = self.latex_doc() else {
            return false;
        };
        let shown = self.editor.active().map(|d| d.preview_mode()) != Some(PreviewMode::Source);
        let latex = &self.preview.latex;
        shown
            && !latex.missing
            && latex
                .builds
                .get(&id)
                .is_none_or(|b| b.pdf.is_none() && !b.done)
    }

    /// 返回悬停在编译状态上时的说明。
    pub(crate) fn latex_status_tip(&self) -> Option<String> {
        let (id, _) = self.latex_doc()?;
        let build = self.preview.latex.builds.get(&id)?;
        match build.problems.is_empty() {
            true => build.failure.clone(),
            false => Some(t!("latex-problems-tip").to_owned()),
        }
    }

    /// 返回当前标签最近一次编译的前 [`MENU_PROBLEMS`] 条错误。
    fn latex_problems(&self) -> &[Problem] {
        let build = (self.latex_doc()).and_then(|(id, _)| self.preview.latex.builds.get(&id));
        let problems = build.map_or(&[][..], |b| b.problems.as_slice());
        &problems[..problems.len().min(MENU_PROBLEMS)]
    }

    /// 返回错误列表的项数。
    pub(crate) fn latex_problem_count(&self) -> usize {
        self.latex_problems().len()
    }

    /// 返回错误列表第 `i` 项的文字。
    pub(crate) fn latex_problem_label(&self, i: usize) -> MenuLabel {
        let Some(problem) = self.latex_problems().get(i) else {
            return MenuLabel::plain("");
        };
        let place = problem_place(problem);
        MenuLabel {
            title: problem.message.clone(),
            subtitle: (!place.is_empty()).then_some(place),
            tooltip: None,
        }
    }

    /// 在编译状态下方列出各处错误。
    pub(in crate::app) fn open_latex_problems(&mut self) {
        if self.latex_problem_count() == 0 {
            return;
        }
        let bar = self.hits.latex_status;
        let at = (bar.right(), bar.bottom() + 4.0);
        self.open_context_menu(MenuTarget::LatexProblems, at);
    }

    /// 打开第 `i` 个错误所在的文件，跳到那一行。
    pub(in crate::app) fn show_latex_problem(&mut self, i: usize) {
        let Some((_, path)) = self.latex_doc() else {
            return;
        };
        let Some(problem) = self.latex_problems().get(i).cloned() else {
            return;
        };
        let file = problem.file.unwrap_or(path);
        self.show_source_line(&file, problem.line.unwrap_or(1));
    }

    /// 返回窗口坐标 (`x`, `y`) 处错误红点所在的行（从 0 起）。
    pub(in crate::app) fn latex_problem_at(&self, x: f32, y: f32) -> Option<usize> {
        let doc = self.editor.active()?;
        let l = self.layout;
        let center = l.gutter.x + self.blame_width() + crate::theme::GUTTER_PADDING / 2.0 + 1.0;
        if (x - center).abs() > PROBLEM_HIT || y < l.text.y || y > l.text.bottom() {
            return None;
        }
        let lh = self.fonts.line_height;
        let row = (f64::from((y - l.text.y) / lh) + doc.scroll_line).floor() as usize;
        let line = doc.line_of_row(row);
        let lines = self.latex_problem_lines(doc.id);
        lines.binary_search(&line).is_ok().then_some(line)
    }

    /// 返回第 `line` 行（从 0 起）错误红点的位置和说明。
    pub(crate) fn latex_problem_tip(&self, line: usize) -> Option<(winc_render::Rect, String)> {
        let doc = self.editor.active()?;
        let path = doc.path.as_deref()?;
        let messages: Vec<&str> = (self.preview.latex.builds.values())
            .flat_map(|b| &b.problems)
            .filter(|p| p.file.as_deref() == Some(path) && p.line == Some(line + 1))
            .map(|p| p.message.as_str())
            .collect();
        if messages.is_empty() {
            return None;
        }
        let (l, lh) = (self.layout, self.fonts.line_height);
        let center = l.gutter.x + self.blame_width() + crate::theme::GUTTER_PADDING / 2.0 + 1.0;
        let y = l.text.y + (doc.row_of(line) as f64 - doc.scroll_line) as f32 * lh;
        let rect = winc_render::Rect::new(center - PROBLEM_HIT, y, PROBLEM_HIT * 2.0, lh);
        let mut unique: Vec<&str> = Vec::new();
        for message in messages {
            if !unique.contains(&message) {
                unique.push(message);
            }
        }
        Some((rect, unique.join("\n")))
    }

    /// 把光标移到第 `line` 行（从 0 起）的开头。
    pub(in crate::app) fn go_to_latex_problem_line(&mut self, line: usize) {
        if let Some(path) = self.editor.active().and_then(|d| d.path.clone()) {
            self.show_source_line(&path, line + 1);
        }
    }

    /// 返回文档 `id` 中出错的行（从 0 起）。
    pub(crate) fn latex_problem_lines(&self, id: DocumentId) -> Vec<usize> {
        let Some(doc) = self.editor.document(id) else {
            return Vec::new();
        };
        if doc.preview_kind() != Some(PreviewKind::Latex) {
            return Vec::new();
        }
        let Some(path) = doc.path.as_deref() else {
            return Vec::new();
        };
        let mut lines: Vec<usize> = (self.preview.latex.builds.iter())
            .flat_map(|(&build_id, build)| {
                build.problems.iter().filter(move |p| match &p.file {
                    Some(file) => file == path,
                    None => build_id == id,
                })
            })
            .filter_map(|p| p.line.map(|l| l.saturating_sub(1)))
            .collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    /// 分栏时让 PDF 对齐左侧顶端那一行；每次绘制前调用。
    pub(crate) fn sync_latex_pdf(&mut self) {
        let Some((id, _, serial)) = self.latex_pdf() else {
            return;
        };
        let Some(doc) = self.editor.active() else {
            return;
        };
        if doc.preview_mode() != PreviewMode::Split {
            return;
        }
        let key = (id, serial, doc.scroll_line.to_bits());
        if self.preview.latex.synced == Some(key) {
            return;
        }
        let Some(synctex) = (self.preview.latex.builds.get(&id)).and_then(|b| b.synctex.clone())
        else {
            return;
        };
        let Some(Ok(pages)) = self.preview.pdf.pages.clone() else {
            return;
        };
        let (Some(path), line) = (doc.path.clone(), super::markdown::text_line(doc)) else {
            return;
        };
        let Some((page, _, v)) = synctex.forward(&path, line.floor() as usize + 1) else {
            return;
        };
        let Some(size) = pages.get(page) else {
            return;
        };
        let top = ((v - SYNC_LEAD) / size.height as f32).clamp(0.0, 1.0);
        if let Some(y) = self.preview.pdf.place_y(page, top) {
            self.preview.pdf.scroll = y.clamp(0.0, self.preview.pdf.max_scroll());
            self.preview.latex.synced = Some(key);
            self.request_redraw();
        }
    }

    /// 跳到 PDF 上 (`x`, `y`) 处对应的源码行，返回是否找到。
    pub(in crate::app) fn latex_source_at(&mut self, x: f32, y: f32) -> bool {
        let Some((id, _, _)) = self.latex_pdf() else {
            return false;
        };
        let Some(synctex) = (self.preview.latex.builds.get(&id)).and_then(|b| b.synctex.clone())
        else {
            return false;
        };
        let Some((page, fx, fy)) = self.preview.pdf.page_at(x, y) else {
            return false;
        };
        let Some(Ok(pages)) = self.preview.pdf.pages.clone() else {
            return false;
        };
        let Some(size) = pages.get(page) else {
            return false;
        };
        let (h, v) = (fx * size.width as f32, fy * size.height as f32);
        let Some((file, line)) = synctex.inverse(page, h, v) else {
            return false;
        };
        self.show_source_line(&file, line);
        true
    }

    /// 在源码中显示文件 `file` 的第 `line` 行（从 1 起）。
    fn show_source_line(&mut self, file: &Path, line: usize) {
        let line = line.saturating_sub(1);
        let here = self.editor.active().and_then(|d| d.path.as_deref()) == Some(file);
        let go = move |app: &mut App| {
            let Some(doc) = app.editor.active_mut() else {
                return;
            };
            let text = doc.text();
            let line = line.min(text.line_count().saturating_sub(1));
            let pos = text.line_col_to_pos(line, 0);
            app.focus = crate::app::Focus::Editor;
            app.scroll_to_line(line, 1.0 / 3.0, true);
            app.select(pos, pos);
            app.request_redraw();
        };
        if here {
            go(self);
        } else {
            self.open_path_then(file, go);
        }
    }
}

/// 各个产物目录的锁，同一目录的编译依次进行。
static BUILDING: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = LazyLock::new(Mutex::default);

/// 找主文件并编译；找不到主文件或被取消时为 `None`。
#[allow(
    clippy::disallowed_methods,
    reason = "编译只在本机进行，见 `latex::compiles_here`"
)]
fn compile(
    shell: &winc_editor::shell::Shell,
    tool: &Toolchain,
    path: &Path,
    text: &str,
    (folder, cache): (Option<&Path>, &Path),
    cancel: &Cancel,
    progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Option<(Job, Compiled)> {
    let root = latex::root_of(path, text, &[])
        .or_else(|| latex::root_of(path, text, &candidates(path, folder)))?;
    let root_text = match root == path {
        true => text.to_owned(),
        false => std::fs::read_to_string(&root).ok()?,
    };
    let dir = root.parent().unwrap_or(Path::new(""));
    let rc = [".latexmkrc", "latexmkrc"]
        .iter()
        .find_map(|name| std::fs::read_to_string(dir.join(name)).ok());
    let out = latex::out_dir(cache, &root);
    let turn = {
        let mut building = BUILDING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(building.entry(out.clone()).or_default())
    };
    let _turn = turn
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let job = latex::job(
        tool,
        &root,
        latex::engine_of(&root_text),
        &out,
        rc.as_deref(),
    );
    let compiled = latex::compile(shell, &job, cancel, progress)?;
    Some((job, compiled))
}

/// 返回 `path` 所在目录和往上几层里的 `.tex` 文件和原文，不出打开的目录 `folder`。
#[allow(
    clippy::disallowed_methods,
    reason = "编译只在本机进行，见 `latex::compiles_here`"
)]
fn candidates(path: &Path, folder: Option<&Path>) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut dir = path.parent();
    for _ in 0..ROOT_SEARCH_DEPTH {
        let Some(here) = dir else {
            break;
        };
        for entry in std::fs::read_dir(here).into_iter().flatten().flatten() {
            let file = entry.path();
            let tex = file
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("tex"));
            let small = entry
                .metadata()
                .is_ok_and(|m| m.len() as usize <= MAX_CANDIDATE_BYTES);
            if tex
                && small
                && file != path
                && let Ok(text) = std::fs::read_to_string(&file)
            {
                out.push((file, text));
            }
        }
        if folder.is_some_and(|f| here == f) {
            break;
        }
        dir = here.parent();
    }
    out
}

/// 读编译写出的 `SyncTeX` 对照。
#[allow(
    clippy::disallowed_methods,
    reason = "编译只在本机进行，见 `latex::compiles_here`"
)]
fn read_synctex(job: &Job) -> Option<Synctex> {
    Synctex::read(&std::fs::read(&job.synctex).ok()?, &job.dir)
}

/// 返回错误所在的文件名和行号。
fn problem_place(problem: &Problem) -> String {
    let name = (problem.file.as_deref())
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned());
    match (name, problem.line) {
        (Some(name), Some(line)) => format!("{name}:{line}"),
        (Some(name), None) => name,
        (None, Some(line)) => line.to_string(),
        (None, None) => String::new(),
    }
}
