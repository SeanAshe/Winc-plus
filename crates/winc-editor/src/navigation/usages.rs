//! 查找用法：在全字匹配的搜索结果中，只保留真正引用该名字的位置。
//!
//! 文本搜索不懂语法，这里按词法分析和定义写法剔除不是用法的命中：
//!
//! - 与当前文件的语言无关的文件（文档、数据文件、其他语言的源码）中的命中；
//! - 名字两端紧挨着名字字符（`my-class` 中的 `my`、`valid?` 中的 `valid`）的命中；
//! - 注释中的命中，以及代码语言中字符串里的命中（样式表的类名在 HTML 中写在属性值里，不剔除）；
//! - 其他同名定义（另一个文件中的同名 `struct Item0`）。

use std::path::Path;

use ropey::Rope;
use winc_syntax::{Highlighter, Language, Markup, Token, TokenKind};

use super::definition::{Finder, is_text};
use super::symbols::{self, SymbolKind};
use crate::editing::scopes::FoldRange;

/// 返回 `text` 中各位置（行号，列号，均从 0 起，列是字符偏移）所在 token 的类别；位置不在任何
/// token 中时为 `None`。
///
/// 按 `language` 从文本开头推算各行的行首状态，跨行的块注释和字符串同样识别；`positions` 不必
/// 有序。
pub(crate) fn token_kinds(
    text: &Rope,
    language: &'static Language,
    positions: &[(usize, usize)],
) -> Vec<Option<TokenKind>> {
    let mut order: Vec<usize> = (0..positions.len()).collect();
    order.sort_by_key(|&i| positions[i]);
    let mut kinds = vec![None; positions.len()];
    let mut highlighter = Highlighter::new(language);
    let mut tokens: Vec<Token> = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    let mut lexed = None;

    for i in order {
        let (line, col) = positions[i];
        if line >= text.len_lines() {
            continue;
        }
        if lexed != Some(line) {
            let Some(state) = highlighter.state_at(text, line, usize::MAX) else {
                continue;
            };
            chars.clear();
            chars.extend(text.line(line).chars().filter(|&c| c != '\n' && c != '\r'));
            winc_syntax::highlight_line(language, &chars, state, &mut tokens);
            lexed = Some(line);
        }
        kinds[i] = tokens
            .iter()
            .find(|t| (t.start..t.end).contains(&col))
            .map(|t| t.kind);
    }
    kinds
}

/// 判断搜索命中是否是某个名字的用法。
///
/// 剔除规则见模块说明，命中还需要与查找的名字完全匹配。
pub struct Filter {
    /// 被查找的名字。
    name: Vec<char>,
    /// 发起查找的文件的语言。
    origin: &'static Language,
    /// 名字是否不区分大小写（见 [`Filter::ignores_case`]）。
    ignore_case: bool,
    /// 识别其他同名定义的查找器。
    finder: Finder,
}

impl Filter {
    /// 创建查找 `name` 的用法的判断器；`origin` 是发起查找的文件的语言。
    ///
    /// `name` 不是 `origin` 中的名字（见 [`Language::is_name`]）时返回 `None`。
    pub fn new(name: &str, origin: &'static Language) -> Option<Filter> {
        if !origin.is_name(name) {
            return None;
        }
        let ignore_case = Self::ignores_case(name, origin);
        Some(Filter {
            name: name.chars().collect(),
            origin,
            ignore_case,
            finder: Finder::new(name, false, ignore_case)?,
        })
    }

    /// 名字 `name` 在语言 `language` 中是否不区分大小写：语言的关键字不区分大小写（SQL、PHP、
    /// PowerShell、批处理、CMake），且名字不带前缀符号。
    ///
    /// PHP、PowerShell 的变量（`$total`）区分大小写，函数名和类名不区分。
    pub fn ignores_case(name: &str, language: &Language) -> bool {
        language.ignore_case && !name.starts_with(|c| language.name_sigils.contains(&c))
    }

    /// 返回名字是否不区分大小写。
    pub fn ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// 返回被查找的名字。
    pub fn name(&self) -> String {
        self.name.iter().collect()
    }

    /// 返回被查找的名字的长度（字符）。
    pub fn name_len(&self) -> usize {
        self.name.len()
    }

    /// 返回 `chars` 的第 `col` 列起是否恰好是这个名字（两端不紧挨名字字符）。
    fn is_name_at(&self, chars: &[char], col: usize) -> bool {
        let end = col + self.name.len();
        let Some(word) = chars.get(col..end) else {
            return false;
        };
        let same = if self.ignore_case {
            word.iter()
                .zip(&self.name)
                .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
        } else {
            word == self.name.as_slice()
        };
        let origin = self.origin;
        // 名字本身不带前缀符号时，`$total` 中的 `total` 是另一个名字（PHP 的变量与函数）。
        let joined_before = col > 0 && {
            let c = chars[col - 1];
            origin.is_name_char(c) || origin.name_sigils.contains(&c)
        };
        let joined_after = chars
            .get(end)
            .is_some_and(|&c| origin.is_name_char(c) || origin.name_suffixes.contains(&c));
        same && !joined_before && !joined_after
    }

    /// 返回 `text` 中这个名字出现的全部位置（行号，列号，均从 0 起），用于只在当前文件内查找的
    /// 局部变量；尚未剔除注释、字符串和定义，交给 [`Filter::usages`] 判断。
    pub fn occurrences(&self, text: &Rope) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        let mut chars: Vec<char> = Vec::new();
        let first = self.name[0];
        for (line, slice) in text.lines().enumerate() {
            chars.clear();
            chars.extend(slice.chars().filter(|&c| c != '\n' && c != '\r'));
            for col in 0..chars.len() {
                let candidate = if self.ignore_case {
                    chars[col].to_lowercase().eq(first.to_lowercase())
                } else {
                    chars[col] == first
                };
                if candidate && self.is_name_at(&chars, col) {
                    found.push((line, col));
                }
            }
        }
        found
    }

    /// 判断语言为 `language`、内容为 `text` 的文件中各处命中（行号，列号）是否是用法，返回与
    /// `matches` 一一对应的结果。
    ///
    /// 剔除规则见模块说明；项目中其他文件的语言用 [`language_of`] 识别。
    pub fn usages(
        &self,
        language: &'static Language,
        text: &Rope,
        matches: &[(usize, usize)],
    ) -> Vec<bool> {
        if !self.origin.is_related(language) {
            return vec![false; matches.len()];
        }
        // 样式表的类名、id 写在 HTML 的属性值里，这类语言中字符串里的名字同样是用法。
        let strings_count = self.origin.stylesheet || self.origin.markup != Markup::None;
        let kinds = token_kinds(text, language, matches);
        let mut line_text = String::new();
        matches
            .iter()
            .zip(kinds)
            .map(|(&(line, col), kind)| {
                if line >= text.len_lines() {
                    return false;
                }
                line_text.clear();
                line_text.extend(text.line(line).chunks());
                let chars: Vec<char> = line_text.trim_end_matches(['\n', '\r']).chars().collect();
                let in_text = match kind {
                    Some(TokenKind::Comment) => true,
                    Some(TokenKind::String) => !strings_count,
                    _ => false,
                };
                let defined = self.finder.definition_at(text, line, &line_text);
                let elsewhere_defined = defined.is_some_and(|d| d.col == col);
                self.is_name_at(&chars, col) && !in_text && !elsewhere_defined
            })
            .collect()
    }

    /// 判断各处定义的命中（行号，整个匹配起始的列号）是否算数，返回与 `matches` 一一对应的结果。
    ///
    /// 注释和字符串中的不算；项目搜索只逐行匹配正则，这里再按 [`Finder::definition_at`] 核对。
    pub fn definitions(
        &self,
        language: &'static Language,
        text: &Rope,
        matches: &[(usize, usize)],
    ) -> Vec<bool> {
        let mut line_text = String::new();
        token_kinds(text, language, matches)
            .into_iter()
            .zip(matches)
            .map(|(kind, &(line, _))| {
                if kind.is_some_and(is_text) {
                    return false;
                }
                if line >= text.len_lines() {
                    return true;
                }
                line_text.clear();
                line_text.extend(text.line(line).chunks());
                self.finder.definition_at(text, line, &line_text).is_some()
            })
            .collect()
    }

    /// `path` 文件的语言是否与发起查找的文件相关（见 [`Language::is_related`]）。
    pub fn is_related(&self, path: &Path) -> bool {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.origin.is_related(winc_syntax::detect(&name))
    }
}

/// 返回局部变量的作用域：在包住定义所在行 `line` 的块（`blocks`，按缩进划分，见
/// [`crate::scopes::fold_ranges`]）中，开头是函数或方法（见 [`symbols::symbols`]）的最内层
/// 那一块的首末行（行号从 0 起，含两端）；没有这样的块时返回 `None`，作用域是整个文件。
///
/// 取函数而不是最内层的块：Python、JS 的 `var` 在 `if`、`for` 中定义的变量在块外同样可见，按
/// 函数划分不会漏掉这些用法；块级作用域的语言中，同一函数里不同块内的同名变量仍会合在一起。
pub fn function_scope(
    text: &Rope,
    language: &Language,
    blocks: &[FoldRange],
    line: usize,
) -> Option<(usize, usize)> {
    let functions: Vec<usize> = symbols::symbols(text, language)
        .into_iter()
        .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
        .map(|s| s.line)
        .collect();
    blocks
        .iter()
        .filter(|b| b.start < line && line <= b.end && functions.contains(&b.start))
        .max_by_key(|b| b.start)
        .map(|b| (b.start, b.end))
}

/// 按文件名和首行（`#!`）识别内容为 `text` 的 `path` 文件的语言。
pub fn language_of(path: &Path, text: &Rope) -> &'static Language {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let first: String = text
        .lines()
        .next()
        .map(|l| l.chars().take(256).collect())
        .unwrap_or_default();
    winc_syntax::detect_with_first_line(&name, &first)
}

#[cfg(test)]
mod tests;
