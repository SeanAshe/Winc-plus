//! 功能对话框（新建工作树、新建分支）的几何：窗口正中一张卡片，左边一窄列右对齐的名称，右边是
//! 输入框；选项是勾选框，与输入框左对齐；卡片底部右对齐「取消」「创建」两个按钮。
//!
//! 外框沿用设置浮层的尺寸：[`SETTINGS_HEADER`] 高的标题行和关闭按钮，左右、底下留
//! [`SETTINGS_PAD`]；框 [`DIALOG_FIELD_H`] 高（与提交信息框、侧栏搜索框相同），按钮
//! [`SETTINGS_CONTROL_H`] 高（与「提交」按钮相同）。名称列取 [`DIALOG_LABEL_W`]。
//!
//! `git`、`ssh` 的输入请求使用 [`DialogLayout::question`]：说明文字占整行，其下是只读信息和输入框。

use super::*;

/// 说明文字的行高（逻辑像素）。
pub const QUESTION_LINE_H: f32 = 18.0;
/// 询问对话框的宽度（逻辑像素）：等宽小字号的 SHA-256 指纹（50 个字符）可以在一行内完整显示。
const QUESTION_W: f32 = 520.0;
/// 询问对话框的名称列宽度（逻辑像素）：容纳英文名称中最长的 `Fingerprint`。
const QUESTION_LABEL_W: f32 = 76.0;

/// 交互式变基对话框的宽度（逻辑像素）：提交说明在一行内可读。
const REBASE_W: f32 = 640.0;
/// 交互式变基列表一行的高度（逻辑像素）。
pub const REBASE_ROW_H: f32 = 28.0;
/// 交互式变基列表中处理方式一列的宽度（逻辑像素）：容纳最长的文字和下拉箭头。
pub const REBASE_PILL_W: f32 = 84.0;
/// 交互式变基列表最多同时显示的行数（行）：更多的提交滚动查看，对话框高度保持在窗口之内。
const REBASE_ROWS: usize = 10;
/// 交互式变基信息框的高度（逻辑像素）：容纳约四行文字。
const REBASE_MESSAGE_H: f32 = 84.0;
/// 交互式变基上移、下移按钮的边长（逻辑像素）。
const REBASE_MOVE: f32 = 28.0;

/// 对话框里可以点的部件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogPart {
    /// 右上角的关闭按钮。
    Close,
    /// 「取消」。
    Cancel,
    /// 主按钮（「创建」）。
    Confirm,
    /// 分支名的输入框；询问对话框中是第一个输入框。
    Branch,
    /// 询问对话框的第二个输入框：登录的密码、远程仓库的地址。
    Second,
    /// 连接对话框的第三个输入框：密钥文件。
    Third,
    /// 交互式变基的列表（键盘焦点）。
    RebaseList,
    /// 交互式变基列表的第几行。
    RebaseRow(usize),
    /// 交互式变基列表第几行的处理方式。
    RebaseAction(usize),
    /// 把选中的行上移。
    RebaseUp,
    /// 把选中的行下移。
    RebaseDown,
    /// 「基于」的分支菜单。
    Base,
    /// 输入框右端的选择按钮：「位置」的选目录，连接对话框中密钥文件的选文件。
    Change,
    /// 「复制被忽略的文件」（勾选框连同名称都能点）。
    Copy,
    /// 「在新窗口中打开」。
    NewWindow,
    /// 新建分支的「创建后签出」。
    Checkout,
}

/// 新建工作树对话框的整套几何；绘制、悬停和点击都从这里取位置。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DialogLayout {
    /// 卡片。
    pub card: Rect,
    /// 交互式变基的提交列表；其他对话框为空矩形。
    pub list: Rect,
    /// 询问对话框的说明文字；没有说明时为空矩形。
    pub message: Rect,
    /// 询问对话框的只读信息行。
    pub values: Vec<Rect>,
    /// 询问对话框的第二个输入框；其他对话框为空矩形。
    pub second: Rect,
    /// 连接对话框的第三个输入框；其他对话框为空矩形。
    pub third: Rect,
    /// 连接对话框第三个输入框右端选文件的按钮。
    pub choose: Option<Rect>,
    /// 顶部的标题行。
    pub header: Rect,
    /// 关闭按钮。
    pub close: Rect,
    /// 每行左边的名称（字在里面右对齐）：分支、基于，有「位置」一行时还有位置。
    pub labels: Vec<Rect>,
    /// 分支名的输入框。
    pub branch: Rect,
    /// 「基于」的分支菜单。
    pub base: Rect,
    /// 「位置」：只读的目录，和它右端选目录的按钮（新建工作树才有）。
    pub location: Option<(Rect, Rect)>,
    /// 勾选项，一项一行。
    pub options: Vec<(DialogPart, Rect)>,
    /// 「取消」。
    pub cancel: Rect,
    /// 主按钮。
    pub confirm: Rect,
}

impl DialogLayout {
    /// 计算对话框的几何：`window` 是窗口矩形，`location` 表示有没有「位置」一行，`options` 是
    /// 下面的勾选项。
    ///
    /// 卡片宽 [`DIALOG_W`]，窗口太窄时缩到窗口以内；高度按内容算，水平居中。竖直方向偏上：
    /// 顶边距窗口上边缘为剩余空间高度的 42%，使视觉重心落在窗口中部。
    pub fn compute(window: Rect, location: bool, options: &[DialogPart]) -> Self {
        let w = DIALOG_W.min(window.w - SETTINGS_MARGIN * 2.0).max(0.0);
        let inner = (w - SETTINGS_PAD * 2.0).max(0.0);
        // 先按卡片左上角在原点排好，最后整体平移到窗口里。
        let x = SETTINGS_PAD;
        let cx = x + DIALOG_LABEL_W + DIALOG_LABEL_GAP;
        let cw = (inner - DIALOG_LABEL_W - DIALOG_LABEL_GAP).max(0.0);
        let header = Rect::new(0.0, 0.0, w, SETTINGS_HEADER);
        let close = Rect::new(
            w - 12.0 - BUTTON,
            (SETTINGS_HEADER - BUTTON) / 2.0,
            BUTTON,
            BUTTON,
        );

        // 一行：左边名称，右边一个和提交信息框一样高的框，行与行隔 `DIALOG_GAP`。
        let mut y = header.bottom() + 4.0;
        let field = |y: &mut f32| {
            let label = Rect::new(x, *y, DIALOG_LABEL_W, DIALOG_FIELD_H);
            let control = Rect::new(cx, *y, cw, DIALOG_FIELD_H);
            *y += DIALOG_FIELD_H + DIALOG_GAP;
            (label, control)
        };
        let (branch_label, branch) = field(&mut y);
        let (base_label, base) = field(&mut y);
        let mut labels = vec![branch_label, base_label];
        let location = location.then(|| {
            let (label, location) = field(&mut y);
            labels.push(label);
            // 选目录的按钮在框里面的右端，上下各留 2。
            let change = Rect::new(
                location.right() - 2.0 - DIALOG_CHANGE_W,
                location.y + 2.0,
                DIALOG_CHANGE_W,
                location.h - 4.0,
            );
            (location, change)
        });

        // 勾选项排在框那一列，勾选框的左边和框的左边对齐。
        y += 2.0;
        let options: Vec<_> = (options.iter())
            .map(|&part| {
                let row = Rect::new(cx, y, cw, DIALOG_FIELD_H);
                y += row.h;
                (part, row)
            })
            .collect();

        y += 16.0;
        let confirm = Rect::new(
            x + inner - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let cancel = Rect::new(
            confirm.x - 8.0 - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let h = confirm.bottom() + SETTINGS_PAD;

        let room = (window.h - h).max(0.0);
        let (dx, dy) = (
            (window.x + (window.w - w) / 2.0).round(),
            (window.y + room * 0.42).round(),
        );
        let at = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);

        DialogLayout {
            card: at(Rect::new(0.0, 0.0, w, h)),
            list: Rect::default(),
            message: Rect::default(),
            values: Vec::new(),
            second: Rect::default(),
            third: Rect::default(),
            choose: None,
            header: at(header),
            close: at(close),
            labels: labels.into_iter().map(at).collect(),
            branch: at(branch),
            base: at(base),
            location: location.map(|(l, c)| (at(l), at(c))),
            options: options.into_iter().map(|(p, r)| (p, at(r))).collect(),
            cancel: at(cancel),
            confirm: at(confirm),
        }
    }

    /// 计算询问对话框的几何：`lines` 是说明文字的行数，`values` 是只读信息的行数，`inputs` 是输入框
    /// 的个数（最多 3 个）。
    ///
    /// 窗口太窄时卡片缩到窗口以内，位置规则与 [`DialogLayout::compute`] 相同。标签在 `labels` 中
    /// 按只读信息、输入框的顺序排列。
    pub fn question(window: Rect, lines: usize, values: usize, inputs: usize) -> Self {
        let w = QUESTION_W.min(window.w - SETTINGS_MARGIN * 2.0).max(0.0);
        let inner = (w - SETTINGS_PAD * 2.0).max(0.0);
        let x = SETTINGS_PAD;
        let cx = x + QUESTION_LABEL_W + DIALOG_LABEL_GAP;
        let cw = (inner - QUESTION_LABEL_W - DIALOG_LABEL_GAP).max(0.0);
        let header = Rect::new(0.0, 0.0, w, SETTINGS_HEADER);
        let close = Rect::new(
            w - 12.0 - BUTTON,
            (SETTINGS_HEADER - BUTTON) / 2.0,
            BUTTON,
            BUTTON,
        );
        let mut y = header.bottom() + 4.0;
        let message = Rect::new(x, y, inner, lines as f32 * QUESTION_LINE_H);
        if lines > 0 {
            y = message.bottom() + DIALOG_GAP + 2.0;
        }
        let mut labels = Vec::new();
        let mut row = |y: &mut f32| {
            labels.push(Rect::new(x, *y, QUESTION_LABEL_W, DIALOG_FIELD_H));
            let control = Rect::new(cx, *y, cw, DIALOG_FIELD_H);
            *y += DIALOG_FIELD_H + DIALOG_GAP;
            control
        };
        let rows: Vec<Rect> = (0..values).map(|_| row(&mut y)).collect();
        let fields: Vec<Rect> = (0..inputs).map(|_| row(&mut y)).collect();
        y += 6.0;
        let confirm = Rect::new(
            x + inner - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let cancel = Rect::new(
            confirm.x - 8.0 - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let h = confirm.bottom() + SETTINGS_PAD;
        let room = (window.h - h).max(0.0);
        let (dx, dy) = (
            (window.x + (window.w - w) / 2.0).round(),
            (window.y + room * 0.42).round(),
        );
        let at = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        let field = |i: usize| fields.get(i).map_or(Rect::default(), |&r| at(r));
        DialogLayout {
            card: at(Rect::new(0.0, 0.0, w, h)),
            message: at(message),
            values: rows.iter().map(|&r| at(r)).collect(),
            second: field(1),
            third: field(2),
            header: at(header),
            close: at(close),
            labels: labels.into_iter().map(at).collect(),
            branch: field(0),
            cancel: at(cancel),
            confirm: at(confirm),
            ..DialogLayout::default()
        }
    }

    /// 计算交互式变基对话框的几何：`rows` 是提交数，`warning` 表示有已推送的提交。
    ///
    /// `values` 依次是上移、下移按钮，`branch` 是信息框，`message` 是已推送提交的提醒。
    pub fn rebase(window: Rect, rows: usize, warning: bool) -> Self {
        let w = REBASE_W.min(window.w - SETTINGS_MARGIN * 2.0).max(0.0);
        let inner = (w - SETTINGS_PAD * 2.0).max(0.0);
        let x = SETTINGS_PAD;
        let header = Rect::new(0.0, 0.0, w, SETTINGS_HEADER);
        let close = Rect::new(
            w - 12.0 - BUTTON,
            (SETTINGS_HEADER - BUTTON) / 2.0,
            BUTTON,
            BUTTON,
        );
        let mut y = header.bottom() + 4.0;
        let message = Rect::new(x, y, inner, if warning { QUESTION_LINE_H } else { 0.0 });
        if warning {
            y = message.bottom() + DIALOG_GAP;
        }
        let list_h = rows.min(REBASE_ROWS) as f32 * REBASE_ROW_H;
        let list = Rect::new(x, y, inner, list_h);
        y = list.bottom() + DIALOG_GAP;
        let field = Rect::new(x, y, inner, REBASE_MESSAGE_H);
        y = field.bottom() + DIALOG_GAP + 6.0;
        let confirm = Rect::new(
            x + inner - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let cancel = Rect::new(
            confirm.x - 8.0 - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let move_y = y + (SETTINGS_CONTROL_H - REBASE_MOVE) / 2.0;
        let up = Rect::new(x, move_y, REBASE_MOVE, REBASE_MOVE);
        let down = Rect::new(up.right() + 4.0, move_y, REBASE_MOVE, REBASE_MOVE);
        let h = confirm.bottom() + SETTINGS_PAD;
        let room = (window.h - h).max(0.0);
        let (dx, dy) = (
            (window.x + (window.w - w) / 2.0).round(),
            (window.y + room * 0.42).round(),
        );
        let at = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        DialogLayout {
            card: at(Rect::new(0.0, 0.0, w, h)),
            list: at(list),
            message: at(message),
            values: vec![at(up), at(down)],
            header: at(header),
            close: at(close),
            branch: at(field),
            cancel: at(cancel),
            confirm: at(confirm),
            ..DialogLayout::default()
        }
    }

    /// 计算克隆仓库对话框的几何：地址和位置两行，版式与询问对话框相同。
    pub fn clone_form(window: Rect) -> Self {
        let mut layout = Self::question(window, 0, 0, 2);
        let rect = std::mem::take(&mut layout.second);
        let change = Rect::new(
            rect.right() - 2.0 - DIALOG_CHANGE_W,
            rect.y + 2.0,
            DIALOG_CHANGE_W,
            rect.h - 4.0,
        );
        layout.location = Some((rect, change));
        layout
    }

    /// 计算连接远端主机对话框的几何：主机、端口和密钥文件三行，版式与克隆仓库相同；密钥
    /// 文件一行的右端有选文件的按钮，位置同「位置」一行的目录按钮。
    pub fn connect_form(window: Rect) -> Self {
        let mut layout = Self::question(window, 0, 0, 3);
        let rect = layout.third;
        layout.choose = Some(Rect::new(
            rect.right() - 2.0 - DIALOG_CHANGE_W,
            rect.y + 2.0,
            DIALOG_CHANGE_W,
            rect.h - 4.0,
        ));
        layout
    }

    /// 输入框右端的选择按钮（[`DialogPart::Change`]）：「位置」的目录按钮，或密钥文件的选文件
    /// 按钮；没有时为 `None`。
    pub fn change(&self) -> Option<Rect> {
        self.location.map(|(_, change)| change).or(self.choose)
    }

    /// 返回 `(x, y)` 处可点击的部件；落在部件之外时返回 `None`。
    pub fn part_at(&self, x: f32, y: f32) -> Option<DialogPart> {
        let change = self.change().map(|change| (change, DialogPart::Change));
        let parts = [
            (self.close, DialogPart::Close),
            (self.cancel, DialogPart::Cancel),
            (self.confirm, DialogPart::Confirm),
            (self.branch, DialogPart::Branch),
            (self.second, DialogPart::Second),
            (self.third, DialogPart::Third),
            (self.base, DialogPart::Base),
        ];
        // 选择按钮叠在输入框里面，先匹配这一项。
        (change.into_iter().chain(parts))
            .chain(self.options.iter().map(|&(part, row)| (row, part)))
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, part)| part)
    }
}
