//! 提交的分支和标签徽章、作者头像。
//!
//! 徽章由提交的引用逐个生成（当前分支、本地分支、远程分支、标签），再按可用宽度排成一行；
//! 排不下的收成「+N」。头像按邮箱哈希取调色板里的颜色，同一个作者的颜色固定。

use super::*;
use crate::icons::RefMark;

/// 一个引用徽章。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Badge {
    /// 徽章上的文字。
    pub text: String,
    /// 底色。
    pub fill: Color,
    /// 文字和图标的颜色。
    pub ink: Color,
    /// 文字前面的图标，表明引用的种类；收起的「+N」没有图标。
    pub mark: Option<RefMark>,
}

impl Badge {
    /// 返回徽章的宽度（逻辑像素）：文字宽 `text` 加两侧的留白，有图标时再加图标的位置。
    fn width(&self, text: f32) -> f32 {
        text + self.pad()
    }

    /// 返回徽章中文字以外的宽度（逻辑像素）。
    pub(super) fn pad(&self) -> f32 {
        BADGE_PAD + self.mark.map_or(0.0, |_| BADGE_MARK)
    }
}

/// 徽章中文字两侧留白的总和（逻辑像素），两侧各一半。
const BADGE_PAD: f32 = 12.0;
/// 徽章中图标占的宽度（逻辑像素）：图标宽 9，与文字相隔 3。
const BADGE_MARK: f32 = 12.0;

/// 返回一个提交的各个引用徽章。
///
/// 当前分支铺强调色；和远程分支在同一个提交上（已经同步）的本地分支只画一个，后面写
/// 远程名；远程分支、标签各一种淡色。每个徽章前面有表明种类的图标。
pub(super) fn badges(p: &Palette, commit: &Commit) -> Vec<Badge> {
    let local: Vec<&str> = commit
        .refs
        .iter()
        .filter_map(|r| match r {
            Ref::Head(n) | Ref::Branch(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    let synced = |name: &str| {
        commit.refs.iter().find_map(|r| match r {
            Ref::Remote(remote) => remote
                .split_once('/')
                .filter(|(_, branch)| *branch == name)
                .map(|(origin, _)| origin.to_owned()),
            _ => None,
        })
    };
    let soft = |c: Color| c.alpha(c.a * 0.16);
    let mut out = Vec::new();
    for r in &commit.refs {
        let (text, fill, ink, mark) = match r {
            Ref::Detached => (
                "HEAD".to_owned(),
                p.accent_fill,
                p.text_on_accent,
                RefMark::Commit,
            ),
            Ref::Head(name) | Ref::Branch(name) => {
                let text = match synced(name) {
                    Some(origin) => format!("{name} · {origin}"),
                    None => name.clone(),
                };
                if matches!(r, Ref::Head(_)) {
                    (text, p.accent_fill, p.text_on_accent, RefMark::Branch)
                } else {
                    (text, soft(p.accent), p.accent, RefMark::Branch)
                }
            }
            Ref::Remote(name) => {
                let branch = name.split_once('/').map_or(name.as_str(), |(_, b)| b);
                if local.contains(&branch) {
                    continue;
                }
                (
                    name.clone(),
                    soft(p.text_muted),
                    p.text_muted,
                    RefMark::Remote,
                )
            }
            Ref::Tag(name) => (name.clone(), soft(p.warning), p.warning, RefMark::Tag),
        };
        let mark = Some(mark);
        out.push(Badge {
            text,
            fill,
            ink,
            mark,
        });
    }
    out
}

/// 把一行引用徽章排进宽度 `room`（逻辑像素），返回排好的徽章。
///
/// 每个徽章最宽 [`BADGE_MAX`]，放不下的从末尾截短；余下的连截短到 [`BADGE_MIN`] 都放不下
/// 时，收成一个淡色的「+N」。第一个徽章总要排上。
pub(super) fn fit_badges(
    app: &App,
    frame: &mut Frame<'_>,
    badges: &[Badge],
    room: f32,
) -> Vec<Badge> {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let more_w = |frame: &mut Frame<'_>, n: usize| {
        frame.measure(FontKind::Ui, size, &format!("+{n}")) + BADGE_PAD
    };
    let mut out = Vec::new();
    let mut used = 0.0;
    for (i, badge) in badges.iter().enumerate() {
        let left = badges.len() - i - 1;
        // 后面还有徽章时给「+N」留出地方。
        let reserve = if left > 0 {
            more_w(frame, left) + BADGE_GAP
        } else {
            0.0
        };
        let fits = (room - used - reserve).min(BADGE_MAX);
        let full = badge.width(frame.measure(FontKind::Ui, size, &badge.text));
        if i > 0 && full.min(BADGE_MAX) > fits && fits < BADGE_MIN {
            let soft = p.text_muted.alpha(p.text_muted.a * 0.16);
            out.push(Badge {
                text: format!("+{}", badges.len() - i),
                fill: soft,
                ink: p.text_muted,
                mark: None,
            });
            return out;
        }
        let fits = if i == 0 {
            BADGE_MAX.min(room).max(BADGE_MIN)
        } else {
            fits
        };
        let room = (fits - badge.pad()).max(0.0);
        let text = truncate_end(frame, &badge.text, size, room).into_owned();
        used += badge.width(frame.measure(FontKind::Ui, size, &text)) + BADGE_GAP;
        out.push(Badge {
            text,
            ..badge.clone()
        });
    }
    out
}

/// [`fit_badges`] 排好的一行徽章的总宽（逻辑像素），不含最后一个后面的间隔。
pub(super) fn badges_width(app: &App, frame: &mut Frame<'_>, badges: &[Badge]) -> f32 {
    let size = app.fonts.ui_small;
    let total: f32 = badges
        .iter()
        .map(|b| b.width(frame.measure(FontKind::Ui, size, &b.text)) + BADGE_GAP)
        .sum();
    (total - BADGE_GAP).max(0.0)
}

/// 从 `x` 起画 [`fit_badges`] 排好的一行徽章。
pub(super) fn draw_badges(
    app: &App,
    frame: &mut Frame<'_>,
    (x, row): (f32, Rect),
    badges: &[Badge],
    clip: Rect,
) {
    let mut x = x;
    for badge in badges {
        x = draw_badge(app, frame, (x, row), badge, clip) + BADGE_GAP;
    }
}

/// 画一个圆角小徽章，高为行高减 6 逻辑像素、最多 18 逻辑像素（经验值）；返回右边缘的 x
/// 坐标。
pub(super) fn draw_badge(
    app: &App,
    frame: &mut Frame<'_>,
    (x, row): (f32, Rect),
    badge: &Badge,
    clip: Rect,
) -> f32 {
    let w = badge.width(frame.measure(FontKind::Ui, app.fonts.ui_small, &badge.text));
    let h = (row.h - 6.0).min(18.0);
    let rect = Rect::new(x, (row.y + (row.h - h) / 2.0).round(), w, h);
    frame.rect_clipped(rect, badge.fill, 4.0, clip);
    let mut text_x = x + BADGE_PAD / 2.0;
    if let Some(mark) = badge.mark {
        let center = (text_x + 4.5, rect.y + rect.h / 2.0);
        icons::ref_mark(frame, mark, center, badge.ink, clip);
        text_x += BADGE_MARK;
    }
    ui_text_small(frame, app, text_x, rect, &badge.text, badge.ink, clip);
    rect.right()
}

/// 作者的小头像：一个按邮箱取色的圆，里面是名字的第一个字。
///
/// 邮箱按 djb2 哈希（初值 5381、乘 33 再异或）取调色板里的颜色，同一个作者的颜色固定。
pub(super) fn draw_avatar(
    app: &App,
    frame: &mut Frame<'_>,
    commit: &Commit,
    (cx, cy): (f32, f32),
    clip: Rect,
) {
    let p = app.palette;
    let hash = commit
        .email
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    let (fill, ink) = p.badges[hash as usize % p.badges.len()];
    // 头像的半径（逻辑像素，经验值）。
    let r = 7.0;
    frame.rect_clipped(Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), fill, r, clip);
    let initial: String = commit
        .author
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    let size = app.fonts.ui_small * 0.8;
    let w = frame.measure(FontKind::Ui, size, &initial);
    let rect = Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r);
    caps_label(frame, app, size, (cx - w / 2.0, rect), &initial, ink, clip);
}
