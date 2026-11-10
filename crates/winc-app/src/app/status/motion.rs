//! 状态栏右侧各项的过渡动画。
//!
//! 一项出现时先让出宽度，再淡入文字；消失时先淡出文字，再收起宽度。内容宽度变化时宽度缓动到
//! 新值，旧文字先淡出，新文字随后淡入。左侧各项的位置随这些宽度连续变化。

use std::time::Duration;

use super::*;
use crate::app::animation::{Clock, reveal, stage};

/// 一次过渡的时长（经验值）。
///
/// 宽度变化与文字淡入淡出分段进行，时长长于标签滑动（150 ms），各段均有足够的帧数。
pub(crate) const STATUS_SLIDE: Duration = Duration::from_millis(240);

/// 消失时文字淡出所占的比例（经验值）；淡出结束后宽度才开始收起。
const FADE_OUT: f32 = 0.3;

/// 视为宽度不变的最大差值（逻辑像素）。
///
/// 差值小于该值时直接采用新宽度，不启动过渡。
const STILL: f32 = 0.5;

/// 状态栏右侧的一项，按从左到右的顺序声明；各组的项按该顺序排列。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusKey {
    /// 当前的仓库或工作树（有多个位置时）。
    Worktree,
    /// Git 分支名，以及领先、落后的提交数。
    Branch,
    /// SVG 中未能绘制的特性。
    PictureMissing,
    /// 图片尺寸（像素）。
    PictureSize,
    /// 图片文件大小。
    PictureFile,
    /// 图片格式。
    PictureFormat,
    /// 光标位置、选中的字符数或多光标的选区个数。
    Position,
    /// 行尾。
    LineEnding,
    /// 编码。
    Encoding,
    /// 缩进。
    Indent,
    /// 语言。
    Language,
    /// 新版本提示。
    Update,
}

/// 一项在本帧应显示的内容。
#[derive(Debug, Clone)]
pub struct StatusEntry {
    /// 项的标识。
    pub key: StatusKey,
    /// 显示的文字。
    pub text: String,
    /// 点击时打开的菜单；不可点击时为 `None`。
    pub item: Option<StatusItem>,
    /// 内容的实际宽度（逻辑像素），含分支图标和下拉箭头。
    pub content: f32,
    /// 该项占用的宽度（逻辑像素）。
    ///
    /// 数字按最宽的数字计宽，光标移动时数字变化不引起宽度变化。
    pub width: f32,
}

/// 一项在本帧的绘制参数，由 [`StatusMotion::sync`] 返回。
#[derive(Debug, Clone)]
pub struct StatusRow {
    /// 项的标识。
    pub key: StatusKey,
    /// 当前文字及其内容宽度（逻辑像素）。
    pub text: (String, f32),
    /// 淡出中的旧文字、其内容宽度（逻辑像素）与不透明度；没有时为 `None`。
    pub previous: Option<(String, f32, f32)>,
    /// 当前文字的不透明度，0 到 1。
    pub alpha: f32,
    /// 占用的宽度（逻辑像素）。
    pub width: f32,
    /// 该项在整排中所占的比例，0 到 1：出现、消失途中小于 1，项与左侧的间距按它缩放。
    pub share: f32,
    /// 点击时打开的菜单；出现、消失途中为 `None`。
    pub item: Option<StatusItem>,
}

/// 状态栏右侧各项的过渡状态，按 [`StatusKey`] 排列。
#[derive(Debug, Default)]
pub struct StatusMotion {
    rows: Vec<Row>,
    /// 测试中没有窗口时仍播放过渡，用于截取过渡途中的画面。
    #[cfg(test)]
    pub(crate) animate_without_window: bool,
}

/// 一项的过渡状态。
#[derive(Debug)]
struct Row {
    key: StatusKey,
    item: Option<StatusItem>,
    text: String,
    /// 当前文字的内容宽度（逻辑像素）。
    content: f32,
    width: Glide,
    /// 淡出中的旧文字；没有时为 `None`。
    previous: Option<Fading>,
    presence: Presence,
}

/// 宽度变化后淡出中的旧文字。
#[derive(Debug)]
struct Fading {
    text: String,
    /// 旧文字的内容宽度（逻辑像素）。
    content: f32,
    /// 过渡的时钟；当前文字按同一时钟在后段淡入。
    clock: Clock,
    /// 开始淡出时旧文字的不透明度，0 到 1。
    ///
    /// 上一次过渡途中又换了文字时，从当时的不透明度开始淡出，不先跳回不透明。
    from: f32,
}

impl Fading {
    /// 旧文字当前的不透明度，0 到 1。
    fn alpha(&self) -> f32 {
        self.from * (1.0 - stage(self.clock.linear(), 0.0, FADE_OUT))
    }

    /// 当前文字当前的不透明度，0 到 1。
    fn incoming(&self) -> f32 {
        reveal(self.clock.progress())
    }
}

/// 一项是否在出现或消失途中。
#[derive(Debug, Clone, Copy)]
enum Presence {
    /// 已完全显示。
    Shown,
    /// 出现途中。
    Entering(Clock),
    /// 消失途中；结束后移除该项。
    Leaving(Clock),
}

/// 从 `from` 缓动到 `to` 的宽度（逻辑像素）；`clock` 为 `None` 时宽度为 `to`。
#[derive(Debug, Clone, Copy)]
struct Glide {
    from: f32,
    to: f32,
    clock: Option<Clock>,
}

impl Glide {
    fn current(&self) -> f32 {
        self.clock
            .map_or(self.to, |clock| clock.between(self.from, self.to))
    }

    /// 从当前宽度过渡到 `to`；`clock` 为 `None` 时直接采用 `to`。
    fn retarget(&mut self, to: f32, clock: Option<Clock>) {
        self.from = self.current();
        self.to = to;
        self.clock = clock;
    }
}

impl StatusMotion {
    /// 用本帧的 `entries` 更新 `group` 中各项的状态，返回该组从左到右的绘制参数。
    ///
    /// `group` 列出该组可能出现的全部项；其中不在 `entries` 里的项开始消失。`animate` 为假时
    /// （见 [`App::animate_status`]）各项直接显示为终态。
    pub fn sync(
        &mut self,
        group: &[StatusKey],
        entries: Vec<StatusEntry>,
        animate: bool,
    ) -> Vec<StatusRow> {
        let mut clock = || animate.then(|| Clock::new(STATUS_SLIDE));
        for row in self.rows.iter_mut().filter(|r| group.contains(&r.key)) {
            let listed = entries.iter().any(|e| e.key == row.key);
            if !listed && !matches!(row.presence, Presence::Leaving(_)) {
                let started = clock();
                row.width.retarget(0.0, started);
                row.presence = started.map_or(Presence::Shown, Presence::Leaving);
                if started.is_none() {
                    row.width.to = 0.0;
                }
            }
        }
        // 没有时钟时消失的项立即移除。
        self.rows.retain(|r| {
            !(group.contains(&r.key) && r.width.to == 0.0 && matches!(r.presence, Presence::Shown))
        });

        for entry in entries {
            match self.rows.iter_mut().find(|r| r.key == entry.key) {
                Some(row) => row.update(entry, &mut clock),
                None => {
                    let started = clock();
                    let width = Glide {
                        from: 0.0,
                        to: entry.width,
                        clock: started,
                    };
                    self.rows.push(Row {
                        key: entry.key,
                        item: entry.item,
                        text: entry.text,
                        content: entry.content,
                        width,
                        previous: None,
                        presence: started.map_or(Presence::Shown, Presence::Entering),
                    });
                }
            }
        }
        self.rows.sort_by_key(|r| r.key);

        self.rows
            .iter()
            .filter(|r| group.contains(&r.key))
            .map(Row::view)
            .collect()
    }

    /// 是否有项在过渡途中。
    pub fn moving(&self) -> bool {
        self.rows.iter().any(|r| {
            r.width.clock.is_some()
                || r.previous.is_some()
                || !matches!(r.presence, Presence::Shown)
        })
    }

    /// 结束已到终点的过渡，并移除已消失的项。
    pub fn settle(&mut self) {
        self.rows
            .retain(|r| !matches!(r.presence, Presence::Leaving(c) if c.done()));
        for row in &mut self.rows {
            if row.width.clock.is_some_and(|c| c.done()) {
                row.width.clock = None;
            }
            if row.previous.as_ref().is_some_and(|f| f.clock.done()) {
                row.previous = None;
            }
            if matches!(row.presence, Presence::Entering(c) if c.done()) {
                row.presence = Presence::Shown;
            }
        }
    }
}

impl Row {
    /// 用本帧的 `entry` 更新已有的项。
    ///
    /// 宽度变化时启动宽度过渡，旧文字淡出、新文字淡入；宽度不变时直接换上新文字，光标移动时
    /// 数字变化不引起淡入淡出。消失途中再次出现时，从当前宽度恢复。
    fn update(&mut self, entry: StatusEntry, clock: &mut impl FnMut() -> Option<Clock>) {
        if let Presence::Leaving(c) = self.presence {
            let started = clock();
            self.width.from *= 1.0 - stage(c.linear(), FADE_OUT, 1.0);
            self.width.clock = None;
            self.width.retarget(entry.width, started);
            self.presence = started.map_or(Presence::Shown, Presence::Entering);
        } else if (entry.width - self.width.to).abs() >= STILL {
            let started = clock();
            self.width.retarget(entry.width, started);
            // 上一次过渡途中又换了文字：旧文字仍比当前文字清晰时继续淡出旧文字，只换掉当前
            // 文字；否则从当前文字此刻的不透明度开始淡出当前文字。
            let fading = self.previous.as_ref();
            let keep = fading.is_some_and(|f| f.alpha() > f.incoming());
            if entry.text != self.text && !keep {
                let from = fading.map_or(1.0, Fading::incoming);
                let old = std::mem::take(&mut self.text);
                self.previous = started.map(|clock| Fading {
                    text: old,
                    content: self.content,
                    clock,
                    from,
                });
            }
        } else {
            self.width.to = entry.width;
        }
        self.text = entry.text;
        self.content = entry.content;
        self.item = entry.item;
    }

    fn view(&self) -> StatusRow {
        let (share, width, alpha) = match self.presence {
            Presence::Shown => (1.0, self.width.current(), 1.0),
            Presence::Entering(c) => (c.progress(), self.width.current(), reveal(c.progress())),
            Presence::Leaving(c) => {
                let t = c.linear();
                let share = 1.0 - stage(t, FADE_OUT, 1.0);
                let alpha = 1.0 - stage(t, 0.0, FADE_OUT);
                (share, self.width.from * share, alpha)
            }
        };
        // 旧文字在前段淡出，新文字在后段淡入，两段文字不同时清晰可见。
        StatusRow {
            key: self.key,
            text: (self.text.clone(), self.content),
            previous: self
                .previous
                .as_ref()
                .map(|f| (f.text.clone(), f.content, alpha * f.alpha())),
            alpha: alpha * self.previous.as_ref().map_or(1.0, Fading::incoming),
            width,
            share,
            item: self
                .item
                .filter(|_| matches!(self.presence, Presence::Shown)),
        }
    }
}

impl App {
    /// 状态栏本帧是否播放过渡；没有窗口、动态效果已关闭时为假。
    pub(crate) fn animate_status(&mut self) -> bool {
        #[cfg(test)]
        if self.status.motion.animate_without_window {
            return true;
        }
        self.clock(STATUS_SLIDE).is_some()
    }
}

#[cfg(test)]
mod tests;
