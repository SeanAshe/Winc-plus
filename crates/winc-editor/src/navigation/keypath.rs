//! 计算配置文件中光标所在的键路径（JSON、YAML、TOML），显示在状态栏上，也可以复制。
//!
//! 不解析整个文件，只读光标之前的文本，因此编辑到一半或格式有误的文件也能给出路径：JSON 从文件
//! 开头扫到光标处，取仍未闭合的对象和数组；YAML 从光标行向上按缩进找上级；TOML 找最近的表头。
//! JSON 的扫描可以在上次停下的地方继续（[`KeyPathScan`]），每隔 [`CHECKPOINT`] 个字符记下状态。

use ropey::Rope;
use winc_core::Lines;
use winc_core::text::Line;

/// 键路径中的一段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// 对象的键或表的字段名。
    Key(String),
    /// 数组元素的序号（从 0 起）。
    Index(usize),
}

/// 计算键路径的文件大小上限（字节）。
///
/// JSON 要从文件开头扫描到光标处（[`KeyPathScan`] 只是不必每次重扫），文件超过 4 MiB 时调用方
/// 不应再调用 [`key_path`]。
pub const KEY_PATH_MAX: usize = 4 << 20;

/// 把键路径格式化为一行文本，如 `a.b[3]["c.d"]`。
///
/// 键不是普通名字时写成 `["…"]`，其中的 `"` 和 `\` 用反斜杠转义。普通名字指以字母、`_`、`$`
/// 开头，其后只含字母、数字、`_`、`-`、`$` 的键。
pub fn format(path: &[Segment]) -> String {
    let mut out = String::new();
    for segment in path {
        match segment {
            Segment::Index(i) => out.push_str(&format!("[{i}]")),
            Segment::Key(key) if is_plain(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Segment::Key(key) => {
                out.push_str("[\"");
                for c in key.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        c => out.push(c),
                    }
                }
                out.push_str("\"]");
            }
        }
    }
    out
}

fn is_plain(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '$')
}

/// 返回字符位置 `pos`（`char` 偏移，从 0 起）处的键路径。
///
/// `language` 不是 `json`、`yaml`、`toml` 时返回空路径；光标不在任何键内时同样为空。本函数
/// 不检查文件大小，调用方应在文件超过 [`KEY_PATH_MAX`] 字节时跳过。
pub fn key_path(text: &impl Lines, language: &str, pos: usize) -> Vec<Segment> {
    match language {
        "json" => json(prefix(text, pos)),
        "yaml" => yaml(text, pos),
        "toml" => toml(text, pos),
        _ => Vec::new(),
    }
}

/// 依次返回文件开头到 `pos`（字符偏移）之前的字符，行与行之间是 `\n`。
///
/// 逐个读出字符而不拼成字符串，避免把光标之前的内容（最多 [`KEY_PATH_MAX`] 字节）整个复制。
fn prefix(text: &impl Lines, pos: usize) -> impl Iterator<Item = char> + '_ {
    let last = text.line_of(pos);
    (0..=last).flat_map(move |line| {
        let (take, newline) = if line < last {
            (usize::MAX, Some('\n'))
        } else {
            (pos - text.line_start(line), None)
        };
        line_chars(text.line_content(line))
            .take(take)
            .chain(newline)
    })
}

/// 依次返回一行中的字符：借用 rope 的行直接读出，不复制；大文件从磁盘读出的行才先转成字符。
fn line_chars(line: Line<'_>) -> impl Iterator<Item = char> + '_ {
    let (slice, owned) = match line {
        Line::Slice(slice) => (Some(slice), None),
        Line::Owned(text) => (None, Some(text.chars().collect::<Vec<_>>())),
    };
    slice
        .into_iter()
        .flat_map(|slice| slice.chars())
        .chain(owned.into_iter().flatten())
}

/// 返回 `before` 中仍未闭合的对象和数组构成的键路径，兼容 JSONC 的注释和多余的逗号。
fn json(before: impl Iterator<Item = char>) -> Vec<Segment> {
    let mut scan = Json::default();
    before.for_each(|c| scan.feed(c));
    scan.path()
}

/// 两次扫描记录点之间相隔的字符数（`char`）。
///
/// 记录点越密，光标往回移动时需要重扫的字符越少，但保存的状态越多；16 Ki 字符是两者的折中，
/// 取自调参。
pub const CHECKPOINT: usize = 16 << 10;

/// 可以增量扫描的 JSON 键路径：记着上次扫过的文字和沿途的扫描状态。
///
/// 每个文档各有一个。文字变了只丢弃改动位置之后的记录，找改动位置靠比较新旧文字的公共前缀；
/// 光标往回移动时从最近的记录继续扫，最多扫 [`CHECKPOINT`] 个字符。
#[derive(Default)]
pub struct KeyPathScan {
    /// 上次扫的文字。
    text: Rope,
    /// 第 `i` 项是扫完前 `(i + 1) * CHECKPOINT` 个字符时的状态。
    marks: Vec<Json>,
}

impl KeyPathScan {
    /// 返回 JSON 文本 `text` 中字符位置 `pos`（`char` 偏移，从 0 起）处的键路径。
    ///
    /// 结果与 `key_path(text, "json", pos)` 相同，差别是复用上次扫描的状态。
    pub fn json(&mut self, text: &Rope, pos: usize) -> Vec<Segment> {
        if self.text != *text {
            let same = common_prefix(&self.text, text);
            self.marks.truncate(same / CHECKPOINT);
            self.text = text.clone();
        }
        let pos = pos.min(text.len_chars());
        let resume = (pos / CHECKPOINT).min(self.marks.len());
        let mut scan = match resume {
            0 => Json::default(),
            n => self.marks[n - 1].clone(),
        };
        let mut at = resume * CHECKPOINT;
        for c in text.chars_at(at).take(pos - at) {
            scan.feed(c);
            at += 1;
            if at.is_multiple_of(CHECKPOINT) && at / CHECKPOINT > self.marks.len() {
                self.marks.push(scan.clone());
            }
        }
        scan.path()
    }
}

/// 返回 `a` 和 `b` 开头相同的字符数；按块逐段比较字节。
fn common_prefix(a: &Rope, b: &Rope) -> usize {
    let (mut chunks_a, mut chunks_b) = (a.chunks(), b.chunks());
    let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
    let mut same = 0;
    loop {
        if x.is_empty() {
            match chunks_a.next() {
                Some(c) => x = c.as_bytes(),
                None => break,
            }
        }
        if y.is_empty() {
            match chunks_b.next() {
                Some(c) => y = c.as_bytes(),
                None => break,
            }
        }
        let n = x.len().min(y.len());
        if x[..n] != y[..n] {
            same += x.iter().zip(y).take_while(|(p, q)| p == q).count();
            break;
        }
        same += n;
        (x, y) = (&x[n..], &y[n..]);
    }
    // 不同的字节落在一个多字节字符中间时，这个字符不算相同。
    a.byte_to_char(same)
}

/// JSON 扫描中未闭合的对象或数组。
#[derive(Clone)]
enum Open {
    /// 对象：最近读到的键，以及是否已读到该键之后的冒号（读到冒号才算位于该键的值中）。
    Object(Option<String>, bool),
    /// 数组：当前元素之前的元素个数（按逗号计数），即当前元素的序号。
    Array(usize),
}

/// JSON 扫描读到的位置在什么里面。
#[derive(Clone, Default)]
enum Mode {
    #[default]
    Normal,
    /// 刚读到一个 `/`，看下一个字符是不是注释。
    Slash,
    /// 字符串里：读到的内容，和上一个字符是不是转义用的 `\`。
    Str(String, bool),
    /// `//` 注释里。
    Line,
    /// 块注释里：上一个字符是不是 `*`。
    Block(bool),
}

/// 逐个字符扫描 JSON 的状态，可以复制后在副本上继续扫描。
#[derive(Clone, Default)]
struct Json {
    stack: Vec<Open>,
    mode: Mode,
}

impl Json {
    fn feed(&mut self, c: char) {
        match &mut self.mode {
            Mode::Str(s, escaped @ true) => {
                s.push(match c {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                });
                *escaped = false;
            }
            Mode::Str(_, escaped) if c == '\\' => *escaped = true,
            Mode::Str(s, _) if c == '"' => {
                let s = std::mem::take(s);
                self.mode = Mode::Normal;
                self.key(s);
            }
            Mode::Str(s, _) => s.push(c),
            Mode::Line => {
                if c == '\n' {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Block(star) => {
                if *star && c == '/' {
                    self.mode = Mode::Normal;
                } else {
                    *star = c == '*';
                }
            }
            Mode::Slash => match c {
                '/' => self.mode = Mode::Line,
                '*' => self.mode = Mode::Block(false),
                _ => {
                    self.mode = Mode::Normal;
                    self.normal(c);
                }
            },
            Mode::Normal => self.normal(c),
        }
    }

    /// 读完一个字符串：对象内、冒号之前的字符串是键。
    fn key(&mut self, s: String) {
        if let Some(Open::Object(key, colon)) = self.stack.last_mut()
            && !*colon
        {
            *key = Some(s);
        }
    }

    fn normal(&mut self, c: char) {
        match c {
            '"' => self.mode = Mode::Str(String::new(), false),
            '/' => self.mode = Mode::Slash,
            '{' => self.stack.push(Open::Object(None, false)),
            '[' => self.stack.push(Open::Array(0)),
            '}' | ']' => {
                self.stack.pop();
            }
            ':' => {
                if let Some(Open::Object(_, colon)) = self.stack.last_mut() {
                    *colon = true;
                }
            }
            ',' => match self.stack.last_mut() {
                Some(Open::Object(key, colon)) => {
                    *key = None;
                    *colon = false;
                }
                Some(Open::Array(n)) => *n += 1,
                None => {}
            },
            _ => {}
        }
    }

    /// 读到这里时仍未闭合的对象和数组构成的路径。
    ///
    /// 光标位于尚未写完的键内时，该键同样计入路径。
    fn path(&self) -> Vec<Segment> {
        let typing = match &self.mode {
            Mode::Str(s, _) => Some(s),
            _ => None,
        };
        let last = self.stack.len().saturating_sub(1);
        (self.stack.iter().enumerate())
            .filter_map(|(i, open)| match open {
                Open::Object(_, false) if i == last && typing.is_some() => {
                    typing.cloned().map(Segment::Key)
                }
                Open::Object(key, _) => key.clone().map(Segment::Key),
                Open::Array(n) => Some(Segment::Index(*n)),
            })
            .collect()
    }
}

/// 一行 YAML 的解析结果：缩进、列表项标记和键。
struct YamlLine {
    /// 行首空格数（列）。
    indent: usize,
    /// `- ` 之后的内容从第几列开始（从 0 起，含缩进）；不是列表项时为 `None`。
    item: Option<usize>,
    /// 该行（或 `- ` 之后）为 `key:` 形式时的键，以及键的起始列（从 0 起）。
    key: Option<(String, usize)>,
}

fn yaml_line(line: &str) -> Option<YamlLine> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    if rest.is_empty() || rest.starts_with('#') {
        return None;
    }
    let (item, body, at) = match rest.strip_prefix('-') {
        Some(after) if after.is_empty() || after.starts_with(' ') => {
            let pad = after.len() - after.trim_start_matches(' ').len();
            (
                Some(indent + 1 + pad),
                after.trim_start_matches(' '),
                indent + 1 + pad,
            )
        }
        _ => (None, rest, indent),
    };
    Some(YamlLine {
        indent,
        item,
        key: yaml_key(body).map(|k| (k, at)),
    })
}

/// 返回 `key: …`、`"key": …`、`'key': …` 形式中的键；不是这些形式时返回 `None`。
fn yaml_key(body: &str) -> Option<String> {
    let (key, rest) = if let Some(quoted) = body.strip_prefix('"') {
        let end = quoted.find('"')?;
        (quoted[..end].to_owned(), &quoted[end + 1..])
    } else if let Some(quoted) = body.strip_prefix('\'') {
        let end = quoted.find('\'')?;
        (quoted[..end].to_owned(), &quoted[end + 1..])
    } else {
        let end = body
            .find(": ")
            .or_else(|| body.strip_suffix(':').map(str::len))?;
        let key = body[..end].trim_end();
        if key.is_empty() || key.starts_with(['{', '[', '#', '&', '*', '!', '|', '>']) {
            return None;
        }
        (key.to_owned(), &body[end..])
    };
    (rest.starts_with(':')).then_some(key)
}

/// 返回字符位置 `pos` 处的 YAML 键路径：从光标所在行向上，按缩进逐层查找上级的键和列表项。
fn yaml(text: &impl Lines, pos: usize) -> Vec<Segment> {
    let line_text = |line: usize| text.line_content(line).slice().to_string();
    let mut path = Vec::new();

    // 光标所在行；该行为空行或注释时，向上取第一个有内容的行。
    let mut line = text.line_of(pos);
    let current = loop {
        if let Some(parsed) = yaml_line(&line_text(line)) {
            break parsed;
        }
        if line == 0 {
            return path;
        }
        line -= 1;
    };
    if let Some((key, _)) = current.key {
        path.push(Segment::Key(key));
    }
    // 上级的缩进必须小于 `limit`。`in_item` 为 `true` 时刚找到的是缩进为 `limit` 的列表项，
    // 此时缩进与之相同的键也是上级（如 `list:` 下紧挨着写 `- a`，`-` 与 `list` 同列）。
    let mut limit = current.indent;
    let mut in_item = current.item.is_some();
    if in_item {
        path.push(Segment::Index(yaml_index(text, line, current.indent)));
    }

    while line > 0 && (limit > 0 || in_item) {
        line -= 1;
        let Some(parsed) = yaml_line(&line_text(line)) else {
            continue;
        };
        match parsed.item {
            Some(content) if parsed.indent < limit => {
                // 例如 `- name: x` 下方的 `run: y`：与 `name` 同列的键属于同一个列表项，是同级而非
                // 上级，不计入路径；缩进比 `name` 更深的内容才位于 `name` 之下。
                if let Some((key, _)) = parsed.key
                    && content < limit
                {
                    path.push(Segment::Key(key));
                }
                path.push(Segment::Index(yaml_index(text, line, parsed.indent)));
                limit = parsed.indent;
                in_item = true;
            }
            None => {
                if let Some((key, at)) = parsed.key
                    && (at < limit || (in_item && at == limit))
                {
                    path.push(Segment::Key(key));
                    limit = at;
                    in_item = false;
                }
            }
            Some(_) => {}
        }
    }
    path.reverse();
    path
}

/// 返回第 `line` 行的列表项（`-` 位于第 `indent` 列）在同一列表中的序号（从 0 起）。
fn yaml_index(text: &impl Lines, line: usize, indent: usize) -> usize {
    let mut index = 0;
    for above in (0..line).rev() {
        let Some(parsed) = yaml_line(&text.line_content(above).slice().to_string()) else {
            continue;
        };
        if parsed.indent < indent || (parsed.indent == indent && parsed.item.is_none()) {
            break;
        }
        if parsed.indent == indent && parsed.item.is_some() {
            index += 1;
        }
    }
    index
}

/// 返回字符位置 `pos` 处的 TOML 键路径：光标之前最近的 `[表]` 或 `[[表数组]]`（后者附带
/// 序号），加上光标所在行的键（键可以带点）。
fn toml(text: &impl Lines, pos: usize) -> Vec<Segment> {
    let caret_line = text.line_of(pos);
    let line_text = |line: usize| text.line_content(line).slice().to_string();
    let mut path = Vec::new();

    let mut header = None;
    for line in (0..=caret_line).rev() {
        let content = line_text(line);
        let trimmed = content.trim();
        if let Some(name) = trimmed
            .strip_prefix("[[")
            .and_then(|r| r.split("]]").next())
        {
            // 序号等于它之前同名表数组的个数（从 0 起）。
            let name = name.trim().to_owned();
            let count = (0..line)
                .filter(|&l| {
                    let t = line_text(l);
                    t.trim()
                        .strip_prefix("[[")
                        .and_then(|r| r.split("]]").next())
                        .is_some_and(|n| n.trim() == name)
                })
                .count();
            header = Some((name, Some(count)));
            break;
        }
        if let Some(name) = trimmed.strip_prefix('[').and_then(|r| r.split(']').next()) {
            header = Some((name.trim().to_owned(), None));
            break;
        }
    }
    if let Some((name, index)) = header {
        path.extend(toml_keys(&name));
        if let Some(index) = index {
            path.push(Segment::Index(index));
        }
    }

    let content = line_text(caret_line);
    let trimmed = content.trim_start();
    if !trimmed.starts_with(['[', '#'])
        && let Some((key, _)) = trimmed.split_once('=')
    {
        path.extend(toml_keys(key.trim()));
    }
    path
}

/// 把带点的键 `a.b."c.d"` 拆成多段；引号内的点不拆分。
fn toml_keys(dotted: &str) -> Vec<Segment> {
    let mut keys = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for c in dotted.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '.') => {
                keys.push(Segment::Key(current.trim().to_owned()));
                current.clear();
            }
            (_, c) => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        keys.push(Segment::Key(current.trim().to_owned()));
    }
    keys
}

#[cfg(test)]
mod tests;
