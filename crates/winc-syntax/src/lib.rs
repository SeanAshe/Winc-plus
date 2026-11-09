//! 语法高亮：按 [`Language`] 的规则表逐行做词法分析，跨行结构的状态记录在行首的 [`LineState`] 中。
//!
//! [`Highlighter`] 按文档逐行缓存行首状态，编辑后从修改行开始失效，绘制时只推算可见区域。
//! 本 crate 不依赖绘制与主题：token 只标出类别，颜色由 `winc-render` 和 `winc-app` 决定。

mod lang;
mod lexer;

use std::ops::ControlFlow;

use winc_core::text::{Lines, MAX_LINE_BYTES, MAX_LINE_CHARS};

pub use lang::{
    BlockString, Heredocs, Interpolation, Language, Markup, RawStrings, Variables, detect,
    detect_with_first_line, find, languages,
};
pub use lexer::{LineState, Token, TokenKind, highlight_line};

/// 一个文档的高亮状态：每行行首的词法状态缓存。
///
/// 只保存已经推算到的行，其余行在需要时向后推算；编辑后从修改所在行开始失效。
#[derive(Debug, Clone)]
pub struct Highlighter {
    /// 文档使用的语言规则。
    language: &'static Language,
    /// `states[i]` 是第 `i` 行行首的状态（行号从 0 起）。
    ///
    /// 长度等于已推算的行数，且不小于 1。
    states: Vec<LineState>,
    /// 大文件按 [`Self::state_near`] 推算出的一段连续行首状态：`near[i]` 是第
    /// `near_from + i` 行行首的状态，`near[0]` 从 `Normal` 开始推算。
    ///
    /// 为空时没有可用的结果。
    near: Vec<LineState>,
    /// `near` 第一项对应的行号（从 0 起）。
    near_from: usize,
    /// 推算状态时复用的字符缓冲，避免每行分配。
    chars: Vec<char>,
}

impl Highlighter {
    /// 创建只含第 0 行行首状态（`Normal`）的高亮状态。
    pub fn new(language: &'static Language) -> Self {
        Self {
            language,
            states: vec![LineState::Normal],
            near: Vec::new(),
            near_from: 0,
            chars: Vec::new(),
        }
    }

    /// 返回创建时传入的语言规则。
    pub fn language(&self) -> &'static Language {
        self.language
    }

    /// 返回已推算出的第 `line` 行行首的状态，不做新的推算；尚未推算到该行时返回 `None`。
    pub fn known_state(&self, line: usize) -> Option<LineState> {
        let near = || self.near.get(line.checked_sub(self.near_from)?);
        self.states.get(line).or_else(near).copied()
    }

    /// 丢弃第 `line` 行之后各行行首的状态；在第 `line` 行发生编辑后调用。
    pub fn invalidate_from(&mut self, line: usize) {
        self.states.truncate(line + 1);
        match line.checked_sub(self.near_from) {
            Some(kept) => self.near.truncate(kept + 1),
            None => self.near.clear(),
        }
    }

    /// 返回第 `line` 行行首的状态（行号从 0 起）；本次推算后仍未到达该行时返回 `None`。
    ///
    /// 从已推算到的位置向后推算，每次最多推算 `budget` 行。返回 `None` 时，调用方先按 `Normal`
    /// 绘制，下一帧继续推算，使跳转到长文件末尾时不会阻塞单帧。`line` 超过末行时按末行计算。
    pub fn state_at(&mut self, text: &impl Lines, line: usize, budget: usize) -> Option<LineState> {
        let target = line.min(text.line_count() - 1);
        let start = self.states.len() - 1;
        let end = start + target.saturating_sub(start).min(budget);

        if end > start {
            let (language, states, chars) = (self.language, &mut self.states, &mut self.chars);

            text.visit_short_lines(start, MAX_LINE_BYTES, |index, _, content| {
                if index >= end {
                    return ControlFlow::Break(());
                }

                let state = states[states.len() - 1];
                states.push(content.map_or(state, |line| scan(language, chars, line, state)));
                ControlFlow::Continue(())
            });
        }

        self.states.get(target).copied()
    }

    /// 返回第 `line` 行行首的状态，从 `line` 之前 `lookback` 行处按 `Normal` 开始推算。
    ///
    /// 查看大文件末尾时无需从文件开头扫描；代价是跨度超过 `lookback` 行的块注释和多行
    /// 字符串可能着色错误。
    ///
    /// 推算出的一段状态保留在 `near` 中，`line` 落在其中时直接返回，否则从该段末尾继续推算。
    pub fn state_near(&mut self, text: &impl Lines, line: usize, lookback: usize) -> LineState {
        let end = self.near_from + self.near.len();
        if self.near.is_empty() || line < self.near_from || line > end + lookback {
            self.near_from = line.saturating_sub(lookback);
            self.near = vec![LineState::Normal];
        }
        let (language, chars, near) = (self.language, &mut self.chars, &mut self.near);
        let from = self.near_from;
        if line >= from + near.len() {
            text.visit_short_lines(
                from + near.len() - 1,
                MAX_LINE_BYTES,
                |index, _, content| {
                    if index >= line {
                        return ControlFlow::Break(());
                    }
                    let state = near[near.len() - 1];
                    near.push(content.map_or(state, |line| scan(language, chars, line, state)));
                    ControlFlow::Continue(())
                },
            );
        }
        near.get(line - from).copied().unwrap_or(LineState::Normal)
    }
}

/// 推算第 `line` 行之后下一行行首的状态。
///
/// `chars` 是复用的字符缓冲，只取行首的 `MAX_LINE_CHARS` 个字符。超过该长度的行只扫描开头，
/// 此时直接返回 `state`，避免截断位置落在字符串或块注释中间时后续整个文件都按字符串着色。
fn scan(language: &Language, chars: &mut Vec<char>, line: &str, state: LineState) -> LineState {
    chars.clear();
    let mut all = line.chars();
    chars.extend(all.by_ref().take(MAX_LINE_CHARS));
    if all.next().is_some() {
        return state;
    }
    lexer::scan_line(language, chars, state)
}

#[cfg(test)]
mod tests;
