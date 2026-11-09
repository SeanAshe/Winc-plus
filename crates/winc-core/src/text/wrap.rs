//! 自动换行：按显示宽度把一行断成若干段，即屏幕上的若干显示行。
//!
//! 断点优先选在空白或标点之后；中日韩文字之间的任意位置都可以断开；单词比整行还长时在行宽
//! 处硬断。行首禁则的标点（如 `，。）」`）不放到下一段开头，行尾禁则的开括号（如 `（「`）不放
//! 在上一段末尾。后续各段与第一段的缩进对齐；缩进超过行宽的一半时不再缩进。

use ropey::RopeSlice;

use crate::text::char_cells;

/// 一行断开后的各段起止：后续各段的起始位置，以及后续各段的缩进宽度。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Breaks {
    /// 第二段起各段的开头（行内的字符序号，从 0 起），严格递增。
    pub starts: Vec<usize>,
    /// 后续各段的缩进宽度（显示列）。
    pub indent: usize,
    /// 第一段的缩进宽度（显示列）：整行的结果为 0；只含一行中部若干段的结果（见
    /// [`breaks_from`]）为 `indent`。
    pub head: usize,
}

impl Breaks {
    /// 返回断开后的段数，即这一行占用的显示行数。
    pub fn rows(&self) -> usize {
        self.starts.len() + 1
    }

    /// 返回第 `col` 个字符所在的段号（从 0 起）。
    ///
    /// 分界处的字符属于后一段：光标位于分界处时绘制在下一行的开头。
    pub fn row_of(&self, col: usize) -> usize {
        self.starts.partition_point(|&s| s <= col)
    }

    /// 返回第 `row` 段的字符范围（行内字符序号，左闭右开）；`len` 是整行的字符数。
    ///
    /// # Panics
    ///
    /// `row` 不小于 [`Breaks::rows`] 时 panic。
    pub fn range(&self, row: usize, len: usize) -> std::ops::Range<usize> {
        let end = self.starts.get(row).copied().unwrap_or(len);
        self.start_of(row)..end
    }

    /// 返回第 `row` 段开头的字符序号（行内，从 0 起）；第一段为 0。
    ///
    /// # Panics
    ///
    /// `row` 不小于 [`Breaks::rows`] 时 panic。
    pub fn start_of(&self, row: usize) -> usize {
        if row == 0 { 0 } else { self.starts[row - 1] }
    }

    /// 返回第 `row` 段开头的缩进宽度（显示列）。
    pub fn indent_of(&self, row: usize) -> usize {
        if row == 0 { self.head } else { self.indent }
    }
}

/// 后续各段缩进上限的分母：缩进宽度超过行宽的 `1 / MAX_INDENT_SHARE` 时不再缩进。
///
/// 取 2，即缩进最多占行宽的一半。取自调参：缩进更宽时后续各段可用的宽度过窄，每段只能容纳
/// 少数字符，显示行数随之明显增加。
const MAX_INDENT_SHARE: usize = 2;

/// 按 `width` 个显示列把 `chars` 断开，返回分段结果。
///
/// `chars` 是该行的字符（不含换行符），`width` 小于 1 时按 1 处理，
/// `tab_width` 是制表位宽度（显示列）。
///
/// # Panics
///
/// `chars` 含制表符且 `tab_width` 为 0 时 panic。
pub fn breaks(chars: &[char], width: usize, tab_width: usize) -> Breaks {
    let width = width.max(1);
    // 整行放得下时无需分段，返回 `Breaks::default()`（缩进记为 0）。调用方通常已测量过行宽，
    // 此处仍测量到超出行宽为止，使结果不依赖调用方的判断；该测量的代价不超过一段的宽度。
    let mut total = 0;
    for &c in chars {
        total += char_cells(c, total, tab_width);
        if total > width {
            break;
        }
    }
    if total <= width {
        return Breaks::default();
    }

    let lead = chars.iter().take_while(|&&c| c == ' ' || c == '\t').count();
    let lead_cells = chars[..lead]
        .iter()
        .fold(0, |cell, &c| cell + char_cells(c, cell, tab_width));
    let indent = if lead_cells * MAX_INDENT_SHARE <= width {
        lead_cells
    } else {
        0
    };

    let mut out = Breaks {
        starts: Vec::new(),
        indent,
        head: 0,
    };
    cut_rows(chars, Some(lead), width, tab_width, &mut out);
    out
}

/// 按 `width` 个显示列把一行中部的字符 `chars` 断开，返回分段结果，位置相对 `chars` 的开头。
///
/// `chars` 从该行某个后续段的开头起，各段都按 `indent` 个显示列缩进。`chars` 在该行的某个断点
/// 或行尾结束时，结果与对整行调用 [`breaks`] 所得的对应部分相同：每段的断点只取决于该段开头
/// 之后的字符。长行因此可以只计算屏幕上可见的若干段，不必从行首断起。
///
/// # Panics
///
/// `chars` 含制表符且 `tab_width` 为 0 时 panic。
pub fn breaks_from(chars: &[char], width: usize, tab_width: usize, indent: usize) -> Breaks {
    let mut out = Breaks {
        starts: Vec::new(),
        indent,
        head: indent,
    };
    cut_rows(chars, None, width.max(1), tab_width, &mut out);
    out
}

/// 从 `chars` 的开头起逐段断开，断点追加到 `out.starts`。
///
/// `lead` 是行首缩进的字符数，表示 `chars` 从行首起；为 `None` 时 `chars` 从某个后续段的开头起。
fn cut_rows(chars: &[char], lead: Option<usize>, width: usize, tab_width: usize, out: &mut Breaks) {
    let mut start = 0;
    while start < chars.len() {
        // 后续各段先留出缩进。
        let (offset, lead) = match start {
            0 => (out.head, lead),
            _ => (out.indent, None),
        };
        let cut = match row_cut(&chars[start..], offset, lead, width, tab_width) {
            RowCut::Rest => break,
            RowCut::At(cut) => start + cut,
            // 断在这串空白之后：空白留在上一段末尾，超出部分不可见，下一段从非空白字符开始。
            RowCut::Blank(end) => {
                let rest = &chars[start + end..];
                start + end + rest.iter().take_while(|c| matches!(c, ' ' | '\t')).count()
            }
        };
        if cut >= chars.len() {
            break;
        }
        out.starts.push(cut);
        start = cut;
    }
}

/// 按 `width` 个显示列把一行文字 `text` 断开，对第二段起每一段开头的字符序号（行内，从 0 起）
/// 依次调用 `each`，返回后续各段的缩进宽度（显示列）。
///
/// 断点和缩进与 [`breaks`] 的结果相同，但不为整行建立字符表：每段只查看段首之后约一个行宽
/// 的内容，全是 ASCII 字符时按字节计算。用于只需段数和少数断点的场合，
/// 例如统计数 MB 长行的显示行数。
///
/// # Panics
///
/// `text` 含制表符且 `tab_width` 为 0 时 panic。
pub fn each_break(
    text: &str,
    width: usize,
    tab_width: usize,
    mut each: impl FnMut(usize),
) -> usize {
    let width = width.max(1);
    let bytes = text.as_bytes();
    // 整行放得下时没有断点，缩进记为 0，与 `breaks` 相同。
    let mut total = 0;
    for c in text.chars() {
        total += char_cells(c, total, tab_width);
        if total > width {
            break;
        }
    }
    if total <= width {
        return 0;
    }

    let blank = |b: &&u8| matches!(**b, b' ' | b'\t');
    let lead = bytes.iter().take_while(blank).count();
    let lead_cells = bytes[..lead].iter().fold(0, |cell, &b| {
        cell + char_cells(char::from(b), cell, tab_width)
    });
    let indent = if lead_cells * MAX_INDENT_SHARE <= width {
        lead_cells
    } else {
        0
    };

    // 当前段开头的字节偏移和字符序号。
    let (mut at, mut col) = (0, 0);
    let mut window: Vec<char> = Vec::new();
    while at < bytes.len() {
        let (offset, lead) = match at {
            0 => (0, Some(lead)),
            _ => (indent, None),
        };
        // 一段最多容纳 `width` 个有宽度的字符，查看 `width + 1` 个字符通常足以确定断点；
        // 段内含不占宽度的字符时可能不足，此时加倍后重试。
        let mut want = width + 1;
        let (chars, len, cut) = loop {
            let end = (at + want).min(bytes.len());
            let (cut, len, whole) = if bytes[at..end].is_ascii() {
                let cut = row_cut(&Ascii(&bytes[at..end]), offset, lead, width, tab_width);
                (cut, None, end == bytes.len())
            } else {
                window.clear();
                window.extend(text[at..].chars().take(want));
                let cut = row_cut(&window[..], offset, lead, width, tab_width);
                (cut, Some(&window), window.len() < want)
            };
            let chars = match cut {
                RowCut::Rest if whole => return indent,
                RowCut::Rest => {
                    want *= 2;
                    continue;
                }
                RowCut::At(chars) | RowCut::Blank(chars) => chars,
            };
            // ASCII 字符各占 1 字节。
            let len = len.map_or(chars, |w| w[..chars].iter().map(|c| c.len_utf8()).sum());
            break (chars, len, cut);
        };
        (at, col) = (at + len, col + chars);
        if matches!(cut, RowCut::Blank(_)) {
            let run = bytes[at..].iter().take_while(blank).count();
            (at, col) = (at + run, col + run);
        }
        if at >= bytes.len() {
            break;
        }
        each(col);
    }
    indent
}

/// 一段文字中按序号取用的字符，供 [`row_cut`] 使用。
trait RowChars {
    /// 字符数。
    fn len(&self) -> usize;
    /// 第 `i` 个字符。
    fn at(&self, i: usize) -> char;
    /// 第 `i` 个字符从显示列 `cell` 起占的显示列数。
    fn cells(&self, i: usize, cell: usize, tab_width: usize) -> usize {
        char_cells(self.at(i), cell, tab_width)
    }
}

impl RowChars for [char] {
    fn len(&self) -> usize {
        <[char]>::len(self)
    }

    fn at(&self, i: usize) -> char {
        self[i]
    }
}

/// 全是 ASCII 字符的一段字节，每个字节是一个字符。
struct Ascii<'a>(&'a [u8]);

impl RowChars for Ascii<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn at(&self, i: usize) -> char {
        char::from(self.0[i])
    }

    /// 与 [`char_cells`] 的结果相同（见测试 `ascii_cells_match_char_cells`）：可打印字符占
    /// 1 个显示列，控制字符不占宽度，不必查 Unicode 宽度表。
    fn cells(&self, i: usize, cell: usize, tab_width: usize) -> usize {
        match self.0[i] {
            b'\t' => tab_width - cell % tab_width,
            0x20..=0x7e => 1,
            _ => 0,
        }
    }
}

/// 一段的结束位置，见 [`row_cut`]。
enum RowCut {
    /// 给出的字符都属于这一段。
    Rest,
    /// 下一段的起始序号。
    At(usize),
    /// 该序号处的空白超出行宽：下一段从这串连续空白之后开始。
    Blank(usize),
}

/// 返回从 `chars` 开头起、起始显示列为 `offset` 的一段的结束位置。
///
/// `lead` 是行首缩进的字符数，表示该段是整行的第一段。结果只取决于该段的字符和其后第一个
/// 放不下的字符：`chars` 包含这些字符时，追加更多字符不改变结果。
fn row_cut<C: RowChars + ?Sized>(
    chars: &C,
    offset: usize,
    lead: Option<usize>,
    width: usize,
    tab_width: usize,
) -> RowCut {
    // 向后测量到放不下的字符为止，`fits` 记录放得下的最后位置；
    // 回退查找断点时从 `fits` 开始，无需重新计算宽度。
    let (mut cell, mut end, mut fits) = (offset, 0, 0);
    while end < chars.len() {
        let w = chars.cells(end, cell, tab_width);
        // 每段至少放一个字符，否则行宽小于该字符的宽度时无法前进。
        if cell + w > width && end > 0 {
            break;
        }
        cell += w;
        end += 1;
        if cell <= width {
            fits = end;
        }
    }
    if end >= chars.len() {
        return RowCut::Rest;
    }
    if matches!(chars.at(end), ' ' | '\t') {
        return RowCut::Blank(end);
    }
    // 从 `fits` 向前查找可断开的位置，找不到时在放不下的字符处硬断。`earliest` 是最靠前的
    // 断点：第一段不能断在缩进内，否则第一段只剩缩进，浪费一行。
    let earliest = lead.map_or(1, |lead| (lead + 1).min(end));
    let mut cut = fits.clamp(earliest, end);
    while cut > earliest && !can_break(chars.at(cut - 1), chars.at(cut)) {
        cut -= 1;
    }
    RowCut::At(if can_break(chars.at(cut - 1), chars.at(cut)) {
        cut
    } else {
        end
    })
}

/// 是否可以在相邻字符 `a` 和 `b` 之间断开。
fn can_break(a: char, b: char) -> bool {
    if no_line_start(b) || no_line_end(a) {
        return false;
    }
    if a == ' ' || a == '\t' {
        return b != ' ' && b != '\t';
    }
    if is_wide(a) || is_wide(b) {
        return true;
    }
    // 代码中的标点在其后断开（如 `a,`、`x-`、`a/`），且下一个字符不是 ASCII 标点。
    matches!(
        a,
        '-' | '/' | ',' | ';' | ')' | ']' | '}' | '>' | '|' | '&' | '?' | '!' | ':'
    ) && !b.is_ascii_punctuation()
}

/// 是否为中日韩文字或全角标点；这些字符的两侧都可以断开。
///
/// 按 Unicode 码点区间判断：谚文字母与音节、CJK 符号与标点、假名、汉字、全角形式等。
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// 是否为不能放在一行开头的字符（行首禁则），即不能断在该字符之前。
///
/// 依据中文排版的避头尾规则：收尾标点、后引号、后括号不能出现在行首；小假名同理。
fn no_line_start(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '）'
            | '】'
            | '」'
            | '』'
            | '》'
            | '〉'
            | '”'
            | '’'
            | '…'
            | '·'
            | '～'
            | '%'
            | '‰'
            | ')'
            | ']'
            | '}'
            | ','
            | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | 'ー'
            | 'ゝ'
            | 'ゞ'
            | '々'
    ) || ('ぁ'..='ゖ').contains(&c) && "ぁぃぅぇぉっゃゅょゎゕゖ".contains(c)
        || ('ァ'..='ヶ').contains(&c) && "ァィゥェォッャュョヮヵヶ".contains(c)
}

/// 是否为不能放在一行末尾的字符（行尾禁则），即不能断在该字符之后。
///
/// 依据中文排版的避头尾规则：开引号、开括号不能留在行末。
fn no_line_end(c: char) -> bool {
    matches!(
        c,
        '（' | '【' | '「' | '『' | '《' | '〈' | '“' | '‘' | '(' | '[' | '{' | '$' | '￥' | '#'
    )
}

/// 返回 `chars` 中第 `col` 个字符在其所在段内的起始显示列（含该段的缩进宽度）。
///
/// `chars` 与 `breaks` 对应同一段文字：整行及 [`breaks`] 的结果，或一行中部的若干段及
/// [`breaks_from`] 的结果。
pub fn cell_in_row(chars: RopeSlice<'_>, breaks: &Breaks, col: usize, tab_width: usize) -> usize {
    let row = breaks.row_of(col);
    let start = breaks.start_of(row);
    let mut cell = breaks.indent_of(row);
    // 从段首开始测量，不从行首逐个跳过前面各段的字符：长行靠后的各段每帧都需要测量。
    for c in chars.chars_at(start).take(col - start) {
        cell += char_cells(c, cell, tab_width);
    }
    cell
}

#[cfg(test)]
mod tests;
