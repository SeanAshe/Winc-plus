//! 编辑区绘制：行号、各类高亮、选区、语法着色和光标，只绘制可见行。
//!
//! 此外还负责图片与 SVG 的预览、无法显示文本时的说明页，以及没有打开文件时的欢迎页。

mod area;
mod caret;
mod glyphs;
pub(super) mod hint;
mod minimap;
mod picture;
mod placeholder;
pub(super) mod places;
mod ruler;
mod sticky;

use area::draw_text_area;
pub(super) use caret::*;
pub(super) use glyphs::*;
pub(crate) use minimap::MinimapRows;
pub(in crate::view) use picture::*;
use placeholder::*;
pub(crate) use ruler::*;
use sticky::*;
pub(in crate::view) use sticky::{draw_pinned_rows, draw_pinned_shadow, pinned_height};

use std::fmt::Write as _;
use std::ops::{ControlFlow, Range};
use std::sync::Arc;

use ropey::RopeSlice;
use winc_core::text::{self, Columns, MAX_LINE_BYTES, MAX_LINE_CHARS};
use winc_core::{Indent, Lines, Selection, movement};
use winc_editor::PictureState;
use winc_editor::Unsupported;
use winc_i18n::t;
use winc_preview::image::Source;
use winc_syntax::{LineState, TokenKind};

use super::*;
use crate::app::{BraceKey, Focus, MARK_W, PlaceholderAction, fold_column, mark_x, ruler_rows};
use crate::layout::{GIT_LANE, MARK_LANE, Ruler, editor_content_height};
use crate::system;

/// 每帧最多向后推算高亮状态的行数（行）。
///
/// 取自调参：跳到长文件靠后的位置时推算分多帧完成，该行数与粘滞滚动的推算合计约 16 ms
/// （约一次屏幕刷新），期间界面保持响应。
pub(super) const HIGHLIGHT_BUDGET: usize = 4_000;

/// 查找匹配括号时，向前或向后最多扫描的字符数（字符）。
///
/// 超出该范围仍未找到时视为没有配对的括号，限定光标移动后单次查找的耗时。
const BRACE_SCAN_LIMIT: usize = 20_000;

/// 绘制编辑区：当前标签是文本时绘制文本区（见 `area`）。
///
/// 当前标签是图片、SVG、Markdown 的预览或无法显示的文件时，改为绘制相应的预览或说明；
/// 没有标签时绘制开始页或欢迎页。
pub(super) fn draw_editor(app: &mut App, frame: &mut Frame<'_>) {
    // Markdown、PDF 预览绘制时重新填充；预览不显示时若仍保留，滚轮和点击事件会被预览接收。
    app.markdown.area = Rect::default();
    app.pdf.area = Rect::default();
    app.pdf.strip = Rect::default();

    let Some(index) = app.editor.active else {
        // 没有标签：释放缩放后的预览缓存，避免继续占用内存。
        app.picture_cache = None;
        // 没有打开目录时显示开始页（打开、新建、最近目录）；打开了目录但没有标签时显示
        // 欢迎页（常用操作及其快捷键）。
        if app.editor.workspace.is_none() {
            workspace::start::draw(app, frame);
        } else {
            draw_welcome(app, frame);
        }
        return;
    };

    match app.editor.documents[index].picture.clone() {
        Some(PictureState::Ready(picture)) => {
            let id = app.editor.documents[index].id;
            let area = app.layout.editor;
            draw_picture(app, frame, id, &picture, area);
            return;
        }
        // 仍在后台解码时留空，解码完成后重新绘制。
        Some(PictureState::Loading) => return,
        Some(PictureState::Failed(reason)) => {
            app.picture_cache = None;
            let message = t!("editor-preview-failed", reason = reason);
            draw_placeholder(
                app,
                frame,
                message,
                &[(PlaceholderAction::ShowSource, t!("editor-show-source"))],
            );
            return;
        }
        None => {}
    }
    // docx：显示转换成 Markdown 后排版的阅读视图；读取或转换失败时说明原因。
    if app.editor.documents[index].unsupported == Some(Unsupported::Docx) {
        match app.markdown.failed.clone() {
            Some(reason) => {
                use PlaceholderAction::*;
                let message = t!("editor-preview-failed", reason = reason);
                let actions = [
                    (Retry, t!("editor-retry")),
                    (OpenWithDefaultApp, t!("editor-open-with-default-app")),
                ];
                draw_placeholder(app, frame, message, &actions);
            }
            None => {
                let area = app.layout.editor;
                super::preview::markdown::draw(app, frame, area);
            }
        }
        return;
    }
    // PDF：逐页显示；打开失败时说明原因。
    if app.editor.documents[index].unsupported == Some(Unsupported::Pdf) {
        match app.pdf.pages.clone() {
            Some(Err(reason)) => {
                use PlaceholderAction::*;
                let message = t!("editor-preview-failed", reason = reason);
                let actions = [
                    (Retry, t!("editor-retry")),
                    (OpenWithDefaultApp, t!("editor-open-with-default-app")),
                ];
                draw_placeholder(app, frame, message, &actions);
            }
            _ => {
                let area = app.layout.editor;
                super::preview::pdf::draw(app, frame, area);
            }
        }
        return;
    }
    let doc = &app.editor.documents[index];
    if let Some(view) = doc.table.as_deref().filter(|_| !doc.shows_text()) {
        let workbook = doc.unsupported.is_some();
        match &view.state {
            winc_editor::TableState::Ready(_) => {
                let area = app.layout.editor;
                super::preview::table::draw(app, frame, index, area);
            }
            // 仍在后台读取时留空，读取完成后重新绘制。
            winc_editor::TableState::Loading => {}
            winc_editor::TableState::Failed(reason) => {
                use PlaceholderAction::*;
                let message = t!("editor-preview-failed", reason = reason.clone());
                let actions: &[(PlaceholderAction, &str)] = if workbook {
                    &[
                        (Retry, t!("editor-retry")),
                        (OpenWithDefaultApp, t!("editor-open-with-default-app")),
                    ]
                } else {
                    &[(ShowSource, t!("editor-show-source"))]
                };
                draw_placeholder(app, frame, message, actions);
            }
        }
        return;
    }
    let doc = &app.editor.documents[index];
    if doc.preview_kind() == Some(winc_editor::PreviewKind::Markdown)
        && doc.preview_mode() == winc_editor::PreviewMode::Preview
    {
        let area = app.layout.editor;
        super::preview::markdown::draw(app, frame, area);
        return;
    }
    // 分栏预览时缓存中保存的是右半部分的图片（`draw_split`），不清除。
    if app.layout.split.w <= 0.0 {
        app.picture_cache = None;
    }
    if app.editor.documents[index].diff.is_some() {
        diff::draw_diff(app, frame, index);
        return;
    }
    if app.editor.documents[index].log.is_some() {
        super::git::log::draw_log(app, frame, index);
        return;
    }
    // 文件仍在读取（仅读取耗时较长时执行到此处）：提示正在读取的文件名。
    if app.editor.documents[index].is_pending() {
        let name = app.editor.documents[index].name().into_owned();
        draw_placeholder(app, frame, t!("editor-reading", name = name), &[]);
        return;
    }
    if let Some(reason) = app.editor.documents[index].unsupported.clone() {
        use PlaceholderAction::*;
        let actions: &[(PlaceholderAction, &str)] = match reason {
            Unsupported::Binary(_) => &[
                (OpenAsText, t!("editor-open-anyway")),
                (OpenWithDefaultApp, t!("editor-open-with-default-app")),
                (Reveal, system::reveal_label()),
            ],
            Unsupported::TooLarge(_) => &[
                (OpenWithDefaultApp, t!("editor-open-with-default-app")),
                (Reveal, system::reveal_label()),
            ],
            Unsupported::Unreadable(_)
            | Unsupported::Image
            | Unsupported::Workbook
            | Unsupported::Docx
            | Unsupported::Pdf => &[
                (Retry, t!("editor-retry")),
                (Reveal, system::reveal_label()),
            ],
        };
        draw_placeholder(app, frame, reason.to_string(), actions);
        return;
    }

    draw_text_area(app, frame, index);
}

#[cfg(test)]
mod tests;

/// 长行的着色结果，跨帧复用。
///
/// 不换行时每个可见行都要为前 `MAX_LINE_CHARS` 个字符着色，一屏数十条超长行逐帧重新分析
/// 会占去大部分帧时间。只缓存不短于 [`LONG_TOKENS_MIN`] 的行，最多 [`LONG_TOKENS_LINES`] 行，
/// 按最近使用淘汰。
#[derive(Default)]
pub(crate) struct LineTokens {
    pub(super) entries: Vec<(TokensKey, Vec<winc_syntax::Token>, winc_syntax::LineState)>,
}

/// [`LineTokens`] 的键：（文档，版本，行号，着色的字符数，行首状态，语言的标识）。
pub(super) type TokensKey = (
    winc_editor::DocumentId,
    u64,
    usize,
    usize,
    winc_syntax::LineState,
    &'static str,
);

/// 着色结果需要缓存的行的最短长度（字符）：更短的行重新分析的开销可以忽略。
pub(super) const LONG_TOKENS_MIN: usize = 1_000;

/// [`LineTokens`] 最多缓存的行数（行）：足以容纳编辑区和缩略图同时显示的长行。
pub(super) const LONG_TOKENS_LINES: usize = 512;
