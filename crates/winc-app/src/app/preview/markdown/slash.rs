//! Markdown 中输入 `/` 呼出的命令列表（见 `paragraph::slash`）：插入标题、列表、引用、代码块、
//! 表格、提示块、图表等，写入的都是 Markdown 原文。

use winc_preview::markdown::paragraph::Kind;

use super::suggest::Hit;
use super::writing::line_text;
use super::*;
use crate::keymap::Action;

/// 命令为空时排在最前的最近使用的命令个数。
const RECENT: usize = 3;

/// 光标位置的标记，写入前从块中移除。
const CARET: char = '\u{1}';

/// 一条命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slash {
    Heading(u8),
    Bullet,
    Ordered,
    Task,
    Quote,
    Callout(Alert),
    Details,
    Code,
    Math,
    InlineMath,
    Diagram(Chart),
    Table,
    Divider,
    Image,
    Link,
    Contents,
    Date,
}

/// GFM 提示块的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Alert {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

/// mermaid 图的模板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chart {
    Flow,
    Sequence,
    Gantt,
}

/// 列表中的命令，按显示顺序排列。
const ALL: [Slash; 25] = [
    Slash::Heading(1),
    Slash::Heading(2),
    Slash::Heading(3),
    Slash::Bullet,
    Slash::Ordered,
    Slash::Task,
    Slash::Quote,
    Slash::Code,
    Slash::Table,
    Slash::Image,
    Slash::Link,
    Slash::Divider,
    Slash::Callout(Alert::Note),
    Slash::Callout(Alert::Tip),
    Slash::Callout(Alert::Important),
    Slash::Callout(Alert::Warning),
    Slash::Callout(Alert::Caution),
    Slash::Details,
    Slash::Math,
    Slash::InlineMath,
    Slash::Diagram(Chart::Flow),
    Slash::Diagram(Chart::Sequence),
    Slash::Diagram(Chart::Gantt),
    Slash::Contents,
    Slash::Date,
];

impl Slash {
    /// 返回列表中显示的名字。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Slash::Heading(1) => t!("preview-slash-heading-1"),
            Slash::Heading(2) => t!("preview-slash-heading-2"),
            Slash::Heading(_) => t!("preview-slash-heading-3"),
            Slash::Bullet => t!("preview-slash-bullet"),
            Slash::Ordered => t!("preview-slash-ordered"),
            Slash::Task => t!("preview-slash-task"),
            Slash::Quote => t!("preview-slash-quote"),
            Slash::Callout(Alert::Note) => t!("preview-slash-callout-note"),
            Slash::Callout(Alert::Tip) => t!("preview-slash-callout-tip"),
            Slash::Callout(Alert::Important) => t!("preview-slash-callout-important"),
            Slash::Callout(Alert::Warning) => t!("preview-slash-callout-warning"),
            Slash::Callout(Alert::Caution) => t!("preview-slash-callout-caution"),
            Slash::Details => t!("preview-slash-details"),
            Slash::Code => t!("preview-slash-code"),
            Slash::Math => t!("preview-slash-math"),
            Slash::InlineMath => t!("preview-slash-inline-math"),
            Slash::Diagram(Chart::Flow) => t!("preview-slash-flowchart"),
            Slash::Diagram(Chart::Sequence) => t!("preview-slash-sequence"),
            Slash::Diagram(Chart::Gantt) => t!("preview-slash-gantt"),
            Slash::Table => t!("preview-slash-table"),
            Slash::Divider => t!("preview-slash-divider"),
            Slash::Image => t!("preview-slash-image"),
            Slash::Link => t!("preview-slash-link"),
            Slash::Contents => t!("preview-slash-contents"),
            Slash::Date => t!("preview-slash-date"),
        }
    }

    /// 返回命令所属的类：文字块、插入、提示块与折叠块、公式与图表、目录与日期。列表中不同类的
    /// 命令之间有分隔线。
    pub(crate) fn group(self) -> u8 {
        match self {
            Slash::Heading(_) | Slash::Bullet | Slash::Ordered | Slash::Task | Slash::Quote => 0,
            Slash::Code | Slash::Table | Slash::Image | Slash::Link | Slash::Divider => 1,
            Slash::Callout(_) | Slash::Details => 2,
            Slash::Math | Slash::InlineMath | Slash::Diagram(_) => 3,
            Slash::Contents | Slash::Date => 4,
        }
    }

    /// 返回列表中的图标。
    pub(crate) fn glyph(self) -> crate::icons::WritingGlyph {
        use crate::icons::WritingGlyph as G;
        match self {
            Slash::Heading(n) => G::Heading(n),
            Slash::Bullet => G::Bullet,
            Slash::Ordered => G::Ordered,
            Slash::Task => G::Task,
            Slash::Quote => G::Quote,
            Slash::Callout(_) => G::Callout,
            Slash::Details => G::Details,
            Slash::Code => G::CodeBlock,
            Slash::Math => G::Math,
            Slash::InlineMath => G::InlineMath,
            Slash::Diagram(_) => G::Diagram,
            Slash::Table => G::Table,
            Slash::Divider => G::Divider,
            Slash::Image => G::Image,
            Slash::Link => G::Link,
            Slash::Contents => G::Contents,
            Slash::Date => G::Date,
        }
    }

    /// 返回筛选用的别名（英文、拼音和拼音首字母），以空格分隔。
    fn words(self) -> &'static str {
        match self {
            Slash::Heading(1) => "h1 heading title biaoti bt",
            Slash::Heading(2) => "h2 heading title biaoti bt",
            Slash::Heading(_) => "h3 heading title biaoti bt",
            Slash::Bullet => "ul bullet list liebiao lb",
            Slash::Ordered => "ol ordered numbered list liebiao lb",
            Slash::Task => "todo task checkbox renwu rw",
            Slash::Quote => "quote blockquote yinyong yy",
            Slash::Callout(Alert::Note) => "callout note alert tishi shuoming ts",
            Slash::Callout(Alert::Tip) => "callout tip alert tishi jiqiao ts",
            Slash::Callout(Alert::Important) => "callout important alert tishi zhongyao ts",
            Slash::Callout(Alert::Warning) => "callout warning alert tishi jinggao ts",
            Slash::Callout(Alert::Caution) => "callout caution danger alert tishi zhuyi ts",
            Slash::Details => "details toggle fold zhedie zd",
            Slash::Code => "code block daima dm",
            Slash::Math => "math latex formula block gongshi gs",
            Slash::InlineMath => "inline math latex formula hangnei gongshi gs",
            Slash::Diagram(Chart::Flow) => "mermaid flowchart diagram chart liuchengtu tu",
            Slash::Diagram(Chart::Sequence) => "mermaid sequence diagram chart shixutu tu",
            Slash::Diagram(Chart::Gantt) => "mermaid gantt schedule diagram chart gantetu tu",
            Slash::Table => "table biaoge bg",
            Slash::Divider => "hr divider line rule fengexian fgx",
            Slash::Image => "image picture img tupian tp",
            Slash::Link => "link url lianjie lj",
            Slash::Contents => "toc contents outline mulu ml",
            Slash::Date => "date today time riqi rq",
        }
    }

    /// 返回对应的写作命令，列表中显示其快捷键。
    pub(crate) fn action(self) -> Option<Action> {
        Some(match self {
            Slash::Heading(1) => Action::MarkdownHeading1,
            Slash::Heading(2) => Action::MarkdownHeading2,
            Slash::Heading(_) => Action::MarkdownHeading3,
            Slash::Bullet => Action::MarkdownBulletList,
            Slash::Ordered => Action::MarkdownOrderedList,
            Slash::Quote => Action::MarkdownQuote,
            Slash::Code => Action::MarkdownCodeBlock,
            Slash::Math => Action::MarkdownMathBlock,
            Slash::Table => Action::MarkdownTable,
            Slash::Image => Action::MarkdownImage,
            Slash::Link => Action::MarkdownLink,
            _ => return None,
        })
    }

    /// 返回整行改成这一种时的记号和对应的段落种类；不是这类命令时为 `None`。
    fn line(self) -> Option<(String, Kind)> {
        Some(match self {
            Slash::Heading(n) => (format!("{} ", "#".repeat(n.into())), Kind::Heading(n)),
            Slash::Bullet => ("- ".into(), Kind::Bullet),
            Slash::Ordered => ("1. ".into(), Kind::Ordered),
            Slash::Quote => ("> ".into(), Kind::Quote),
            Slash::Task => ("- [ ] ".into(), Kind::Task),
            _ => return None,
        })
    }

    /// 返回插入的块，其中 [`CARET`] 是插入后光标的位置；不是这类命令时为 `None`。
    fn block(self) -> Option<String> {
        let alert = |kind: &str| format!("> [!{kind}]\n> \u{1}");
        let chart = |body: &str| format!("```mermaid\n{body}\u{1}\n```");
        Some(match self {
            Slash::Callout(Alert::Note) => alert("NOTE"),
            Slash::Callout(Alert::Tip) => alert("TIP"),
            Slash::Callout(Alert::Important) => alert("IMPORTANT"),
            Slash::Callout(Alert::Warning) => alert("WARNING"),
            Slash::Callout(Alert::Caution) => alert("CAUTION"),
            Slash::Details => "<details>\n<summary>\u{1}</summary>\n\n</details>".into(),
            Slash::Diagram(Chart::Flow) => chart("flowchart TD\n    A --> B"),
            Slash::Diagram(Chart::Sequence) => chart("sequenceDiagram\n    A->>B: request"),
            Slash::Diagram(Chart::Gantt) => {
                let task = format!(
                    "gantt\n    dateFormat YYYY-MM-DD\n    Task :{}, 3d",
                    today()
                );
                chart(&task)
            }
            Slash::Divider => "---\n\n\u{1}".into(),
            Slash::Contents => "[TOC]\n\n\u{1}".into(),
            _ => return None,
        })
    }

    /// 返回在光标处插入的文字，其中 [`CARET`] 是插入后光标的位置；不是这类命令时为 `None`。
    fn inline(self) -> Option<String> {
        Some(match self {
            Slash::InlineMath => "$\u{1}$".into(),
            Slash::Date => format!("{}\u{1}", today()),
            _ => return None,
        })
    }
}

/// 返回今天的本地日期，写成 `2026-11-19`。
fn today() -> String {
    let (y, m, d, ..) = crate::system::local_time();
    format!("{y:04}-{m:02}-{d:02}")
}

impl App {
    /// 返回与 `query` 相符的命令：名字含有 `query`（不区分大小写），或名字、别名中有以 `query`
    /// 开头的词；英文、拼音的 `query` 只按词首匹配。开头相符的排在前面；`query` 为空时最近
    /// 使用的命令排在最前。
    pub(super) fn command_hits(&self, query: &str) -> Vec<Hit> {
        let query = query.to_lowercase();
        let starts = |c: &Slash| {
            let label = c.label().to_lowercase();
            let words = label.split(' ').chain(c.words().split(' '));
            label.starts_with(&query) || words.clone().any(|w| w.starts_with(&query))
        };
        let matches = |c: &Slash| {
            starts(c) || (!query.is_ascii() && c.label().to_lowercase().contains(&query))
        };
        let mut hits: Vec<Slash> = ALL.into_iter().filter(matches).collect();
        if query.is_empty() {
            let recent = &self.preview.markdown.suggest.recent;
            let rank = |c: &Slash| recent.iter().position(|r| r == c).unwrap_or(RECENT);
            hits.sort_by_key(rank);
        } else {
            hits.sort_by_key(|c| !starts(c));
        }
        hits.into_iter().map(Hit::Command).collect()
    }

    /// 执行命令 `command`：先删去已输入的 `/命令名`（从字符位置 `start` 到光标）。
    pub(super) fn run_slash(&mut self, command: Slash, start: usize) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let caret = doc.selection.head;
        doc.selection = winc_core::Selection::new(start, caret);
        self.run(winc_editor::Command::Insert(String::new()));
        let Some((line, _, _)) = self.selected_lines() else {
            return;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return;
        };
        let content = line_text(rope, line);
        let recent = &mut self.preview.markdown.suggest.recent;
        recent.retain(|c| *c != command);
        recent.insert(0, command);
        recent.truncate(RECENT);
        if let Some((mark, kind)) = command.line() {
            // 空行写入记号；有文字的行按段落命令改写。
            match content.trim().is_empty() {
                true => self.replace_lines(line, line, &mark, mark.len()..mark.len()),
                false => self.markdown_paragraph(kind),
            }
            return;
        }
        if let Some(text) = command.inline() {
            let at = text.find(CARET).unwrap_or(text.len());
            let caret = self.editor.active().map_or(0, |d| d.selection.head);
            self.run(winc_editor::Command::Insert(text.replace(CARET, "")));
            if let Some(doc) = self.editor.active_mut() {
                let pos = caret + text[..at].chars().count();
                doc.selection = winc_core::Selection::point(pos);
            }
            return;
        }
        if let Some(block) = command.block() {
            let at = block.find(CARET).unwrap_or(block.len());
            let block = block.replace(CARET, "");
            match content.trim().is_empty() {
                true => self.replace_lines(line, line, &block, at..at),
                false => self.insert_below(line, &block, at),
            }
            return;
        }
        match command {
            Slash::Code => self.markdown_fenced("```"),
            Slash::Math => self.markdown_fenced("$$"),
            Slash::Table => self.markdown_table(),
            Slash::Image => self.markdown_link(true),
            Slash::Link => self.markdown_link(false),
            _ => {}
        }
    }
}
