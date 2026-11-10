//! 文件树各行名字的过渡：合并显示的目录展开、折叠时，以及放不下的名字随鼠标展开、收回时。
//!
//! 宽度缓动到新值；相同的前缀保持不动，其余部分旧的先淡出，新的随后淡入，节奏与状态栏相同。
//! 提交视图的改动列表用同一套（键是文件和目录，见 `CommitName`），两处的名字展开、收回一致。

use std::borrow::Borrow;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::path::PathBuf;

use crate::app::animation::{Clock, reveal, stage};
use crate::app::status::motion::STATUS_SLIDE;

use super::*;

/// 旧内容淡出所占的比例（经验值），与状态栏相同。
const FADE_OUT: f32 = 0.3;

/// 视为宽度不变的最大差值（逻辑像素）。
const STILL: f32 = 0.5;

/// 文件树各行名字的过渡状态，按 `K` 区分各行。
///
/// 绘制时才知道每行的内容和宽度，因此可在只读 `App` 的绘制路径中更新。
pub struct TreeLabels<K = PathBuf> {
    rows: RefCell<HashMap<K, Label>>,
    /// 本帧是否播放过渡；没有窗口、动态效果关闭时为假。
    animate: bool,
    /// 绘制的帧号，用来清除不再显示的行。
    frame: u64,
    /// 测试中没有窗口时仍播放过渡，时钟停在刚开始的位置，结果不随实际时间变化。
    #[cfg(test)]
    pub(crate) animate_without_window: bool,
}

/// 一行名字的状态。
struct Label {
    parts: Vec<String>,
    width: f32,
    cause: LabelCause,
    motion: Option<Motion>,
    frame: u64,
}

/// 一次过渡的起点。
struct Motion {
    from: Vec<String>,
    from_width: f32,
    clock: Clock,
}

/// 决定一行名字该显示什么的用户状态：只有它变了才播放过渡。
///
/// 拖动侧栏宽度、横向滚动也会改变显示的内容和宽度，这些不是用户对这一行的操作，直接采用新值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelCause {
    /// 鼠标是否在这一行上。
    pub hovered: bool,
    /// 合并显示的目录级数；没有合并时为 0。
    pub levels: usize,
    /// 当前的一级在合并的目录中的下标。
    pub current: usize,
}

/// 一行名字在本帧的绘制参数。
#[derive(Debug, Clone, PartialEq)]
pub struct LabelView {
    /// 占用的宽度（逻辑像素）。
    pub width: f32,
    /// 淡出中的旧内容；没有时为空。
    pub old: Vec<String>,
    /// 新旧内容相同的前缀部分数，这些部分不做淡入淡出。
    pub common: usize,
    /// 旧内容的不透明度，0 到 1。
    pub old_alpha: f32,
    /// 新内容的不透明度，0 到 1。
    pub alpha: f32,
    /// 过渡的进度，0 到 1；没有过渡时为 1。
    pub progress: f32,
}

impl Label {
    fn view(&self) -> LabelView {
        let Some(motion) = &self.motion else {
            return LabelView {
                width: self.width,
                old: Vec::new(),
                common: self.parts.len(),
                old_alpha: 0.0,
                alpha: 1.0,
                progress: 1.0,
            };
        };
        let common = self
            .parts
            .iter()
            .zip(&motion.from)
            .take_while(|(a, b)| a == b)
            .count();
        let changed = motion.from != self.parts;
        LabelView {
            width: motion.clock.between(motion.from_width, self.width),
            old: if changed {
                motion.from.clone()
            } else {
                Vec::new()
            },
            common,
            old_alpha: 1.0 - stage(motion.clock.linear(), 0.0, FADE_OUT),
            alpha: if changed {
                reveal(motion.clock.progress())
            } else {
                1.0
            },
            progress: motion.clock.progress(),
        }
    }
}

impl<K> Default for TreeLabels<K> {
    fn default() -> Self {
        Self {
            rows: RefCell::default(),
            animate: false,
            frame: 0,
            #[cfg(test)]
            animate_without_window: false,
        }
    }
}

impl<K: Hash + Eq + Clone> TreeLabels<K> {
    /// 开始绘制一帧；`animate` 为假时各行直接显示为终态。
    pub fn begin_frame(&mut self, animate: bool) {
        self.frame += 1;
        self.animate = animate;
    }

    /// 结束绘制一帧：不再显示的行不再记录，之后再出现时不播放过渡。
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        self.rows.get_mut().retain(|_, row| row.frame == frame);
    }

    /// 记录 `key` 这一行本帧应显示的内容 `parts` 和宽度 `width`，返回本帧的绘制参数。
    ///
    /// 第一次出现的行直接显示；`cause` 变了时从当前宽度开始过渡，否则直接采用新值，进行中的
    /// 过渡继续朝新值进行。
    pub fn sync<Q>(
        &self,
        key: &Q,
        cause: LabelCause,
        parts: &[Cow<'_, str>],
        width: f32,
    ) -> LabelView
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ToOwned<Owned = K> + ?Sized,
    {
        let same = |row: &Label| {
            row.parts.len() == parts.len() && row.parts.iter().zip(parts).all(|(a, b)| a == b)
        };
        let mut rows = self.rows.borrow_mut();
        if !rows.contains_key(key) {
            rows.insert(
                key.to_owned(),
                Label {
                    parts: parts.iter().map(|p| p.clone().into_owned()).collect(),
                    width,
                    cause,
                    motion: None,
                    frame: self.frame,
                },
            );
        }
        let row = rows.get_mut(key).expect("刚刚插入");
        row.frame = self.frame;
        let caused = row.cause != cause;
        row.cause = cause;
        if !same(row) || (row.width - width).abs() >= STILL {
            let current = row.view().width;
            let from = std::mem::replace(
                &mut row.parts,
                parts.iter().map(|p| p.clone().into_owned()).collect(),
            );
            row.width = width;
            if caused {
                row.motion = self.animate.then(|| Motion {
                    from,
                    from_width: current,
                    clock: self.clock(),
                });
            }
        }
        row.view()
    }

    /// 返回 `key` 这一行本帧的绘制参数；没有记录时返回 `None`。
    pub fn peek<Q>(&self, key: &Q) -> Option<LabelView>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.rows.borrow().get(key).map(Label::view)
    }

    /// 返回正在过渡的各行及其绘制参数。
    pub fn moving_rows(&self) -> Vec<(K, LabelView)> {
        self.rows
            .borrow()
            .iter()
            .filter(|(_, row)| row.motion.is_some())
            .map(|(key, row)| (key.clone(), row.view()))
            .collect()
    }

    /// 是否有行在过渡途中。
    pub fn moving(&self) -> bool {
        self.rows.borrow().values().any(|row| row.motion.is_some())
    }

    /// 返回一次过渡的时钟。
    pub(crate) fn clock(&self) -> Clock {
        #[cfg(test)]
        if self.animate_without_window {
            return Clock::stopped(STATUS_SLIDE, std::time::Duration::from_millis(1));
        }
        Clock::new(STATUS_SLIDE)
    }

    /// 立即结束所有过渡，测试里用来进入下一步。
    #[cfg(test)]
    pub(crate) fn finish(&mut self) {
        for row in self.rows.get_mut().values_mut() {
            row.motion = None;
        }
    }

    /// 结束已到终点的过渡。
    pub fn settle(&mut self) {
        for row in self.rows.get_mut().values_mut() {
            if row.motion.as_ref().is_some_and(|m| m.clock.done()) {
                row.motion = None;
            }
        }
    }
}

impl App {
    /// 文件树名字本帧是否播放过渡；没有窗口、动态效果关闭时为假。
    pub(crate) fn animate_tree_labels(&mut self) -> bool {
        #[cfg(test)]
        if self.tree.labels.animate_without_window {
            return true;
        }
        self.clock(STATUS_SLIDE).is_some()
    }

    /// 改动列表的名字本帧是否播放过渡，条件同 [`App::animate_tree_labels`]。
    pub(crate) fn animate_commit_names(&mut self) -> bool {
        #[cfg(test)]
        if self.commit.names.animate_without_window {
            return true;
        }
        self.clock(STATUS_SLIDE).is_some()
    }
}

#[cfg(test)]
mod tests;
