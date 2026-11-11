//! 模式语法的转换：把查找用的模式转换成宿主的正则引擎接受的写法（[`Query`]）。
//!
//! 模式先按当前的转义级别（`\v`、`\m`、`\M`、`\V`）逐个取出记号，再转换成正则：分组、选择、
//! 量词、词的边界、字符类、环视、匹配的起点与终点。不能转换的写法返回其所在的字符偏移，不按
//! 字面查找。

use crate::host::Query;

/// 转义级别：决定哪些字符不加反斜杠就有特殊含义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Magic {
    /// `\V`：只有反斜杠有特殊含义。
    None,
    /// `\M`：另有 `^`、`$`。
    Off,
    /// `\m`：另有 `.`、`*`、`[`、`~`。
    On,
    /// `\v`：字母、数字、下划线之外的字符都有特殊含义。
    All,
}

/// 一个记号：字符，以及该字符是否有特殊含义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token {
    /// 字符。
    c: char,
    /// 是否有特殊含义。
    magic: bool,
    /// 记号在模式中占用的字符数。
    len: usize,
}

/// 加反斜杠之后含义与不加时相反的字符。
const META: &str = "%&()*+.123456789<=>?@ACDFHIKLMOPSUVWXZ[_acdfhiklmnopsuvwxz{|~";

/// 只在 `\v` 之后不加反斜杠就有特殊含义的字符。
const VERY: &str = "(){%+=?@!&|<>#\"',-:;`/";

/// 转换过程的状态。
struct Translator<'a> {
    /// 模式的全部字符。
    chars: &'a [char],
    /// 当前的位置。
    at: usize,
    /// 转义级别。
    magic: Magic,
    /// 已生成的正则。
    out: String,
    /// 最近一个原子在 `out` 中的起点；其后可以接量词与环视。
    atom: Option<usize>,
    /// 当前位置是否在模式、分组或分支的开头。
    at_start: bool,
    /// 各层分组在 `out` 中分支的起点；第 0 项是最外层。
    branches: Vec<usize>,
    /// 未闭合的各个分组的左括号在 `out` 中的位置。
    opens: Vec<usize>,
    /// 当前最外层的分支末尾需要补上的右括号个数（`\ze` 产生）。
    closers: usize,
    /// `\c`、`\C` 指定的大小写；未指定时为 `None`。
    case: Option<bool>,
    /// 最近一个字面字符在 `out` 中的终点。
    literal_end: Option<usize>,
}

/// 把 `pattern` 转换成查询。
///
/// `ignore_case` 与 `smart_case` 是忽略大小写的两个选项：后者为真且模式中有大写字母时区分
/// 大小写；模式中的 `\c`、`\C` 优先。
///
/// # Errors
///
/// 模式中有无法转换的写法时返回该写法在模式中的字符偏移。
pub fn translate(pattern: &str, ignore_case: bool, smart_case: bool) -> Result<Query, usize> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut translator = Translator {
        chars: &chars,
        at: 0,
        magic: Magic::On,
        out: String::new(),
        atom: None,
        at_start: true,
        branches: vec![0],
        opens: Vec::new(),
        closers: 0,
        case: None,
        literal_end: None,
    };
    translator.run()?;
    let ignore_case = match translator.case {
        Some(ignore) => ignore,
        None => ignore_case && !(smart_case && has_uppercase(&chars)),
    };
    Ok(Query {
        ends_literal: translator.literal_end == Some(translator.out.len()),
        regex: translator.out,
        ignore_case,
    })
}

/// 模式中是否有大写字母；反斜杠之后的字符不算。
fn has_uppercase(chars: &[char]) -> bool {
    let mut at = 0;
    while let Some(&c) = chars.get(at) {
        if c == '\\' {
            at += match chars.get(at + 1) {
                Some('_' | '%') => 3,
                _ => 2,
            };
        } else {
            if c.is_uppercase() {
                return true;
            }
            at += 1;
        }
    }
    false
}

/// 把字面字符 `c` 写入正则，必要时转义。
fn push_literal(out: &mut String, c: char) {
    if "\\.+*?()|[]{}^$#&-~".contains(c) {
        out.push('\\');
    }
    out.push(c);
}

/// 返回字母 `c` 表示的字符类的内容（方括号之内的写法）与是否取反；`c` 不是字符类时返回
/// `None`。
fn class_of(c: char) -> Option<(&'static str, bool)> {
    Some(match c {
        's' => (" \\t", false),
        'S' => (" \\t", true),
        'd' => ("0-9", false),
        'D' => ("0-9", true),
        'x' => ("0-9A-Fa-f", false),
        'X' => ("0-9A-Fa-f", true),
        'o' => ("0-7", false),
        'O' => ("0-7", true),
        'w' => ("0-9A-Za-z_", false),
        'W' => ("0-9A-Za-z_", true),
        'h' => ("A-Za-z_", false),
        'H' => ("A-Za-z_", true),
        'a' => ("A-Za-z", false),
        'A' => ("A-Za-z", true),
        'l' => ("a-z", false),
        'L' => ("a-z", true),
        'u' => ("A-Z", false),
        'U' => ("A-Z", true),
        'i' | 'k' => ("\\w", false),
        'I' | 'K' => ("\\p{L}_", false),
        'f' => ("\\w/.\\-+,#$%\\~=", false),
        'F' => ("\\p{L}_/.\\-+,#$%\\~=", false),
        'p' => ("\\p{Cc}", true),
        'P' => ("\\p{Cc}0-9", true),
        _ => return None,
    })
}

impl Translator<'_> {
    /// 返回位置 `at` 处的字符。
    fn get(&self, at: usize) -> Option<char> {
        self.chars.get(at).copied()
    }

    /// 取出当前位置的记号，不前进；已到末尾时返回 `None`。
    fn peek(&self) -> Option<Token> {
        self.token_at(self.at)
    }

    /// 取出位置 `at` 处的记号。
    fn token_at(&self, at: usize) -> Option<Token> {
        let c = self.get(at)?;
        if c != '\\' {
            return Some(Token {
                c,
                magic: self.plain_magic(c, at, false),
                len: 1,
            });
        }
        let Some(next) = self.get(at + 1) else {
            // 末尾单独的反斜杠按字面处理。
            return Some(Token {
                c: '\\',
                magic: false,
                len: 1,
            });
        };
        let magic = if META.contains(next) {
            !self.plain_magic(next, at + 1, true)
        } else if matches!(next, '^' | '$') {
            self.magic == Magic::None
        } else {
            false
        };
        let c = match next {
            'r' if !magic => '\r',
            't' if !magic => '\t',
            'e' if !magic => '\u{1b}',
            'b' if !magic => '\u{8}',
            other => other,
        };
        Some(Token { c, magic, len: 2 })
    }

    /// 不加反斜杠的字符 `c`（位于 `at`）是否有特殊含义；`escaped` 表示该字符跟在反斜杠之后。
    fn plain_magic(&self, c: char, at: usize, escaped: bool) -> bool {
        match c {
            '.' | '[' | '~' => self.magic >= Magic::On,
            '*' => self.magic >= Magic::On && (escaped || !self.at_start),
            '^' => {
                self.magic >= Magic::Off && !escaped && (self.at_start || self.magic == Magic::All)
            }
            '$' => self.magic >= Magic::Off && !escaped && self.dollar_ends(at + 1),
            c if VERY.contains(c) => self.magic == Magic::All,
            _ => false,
        }
    }

    /// 位置 `at` 起是否为模式、分组或分支的末尾：紧接其前的 `$` 才表示行尾。
    fn dollar_ends(&self, at: usize) -> bool {
        let mut at = at;
        let mut all = self.magic == Magic::All;
        // `$` 之后的大小写与转义级别的标记不算。
        while self.get(at) == Some('\\') {
            match self.get(at + 1) {
                Some('v') => all = true,
                Some('m' | 'M' | 'V') => all = false,
                Some('c' | 'C' | 'Z') => {}
                _ => break,
            }
            at += 2;
        }
        match (self.get(at), self.get(at + 1)) {
            (None, _) => true,
            (Some('\\'), Some('|' | '&' | ')' | 'n')) => true,
            (Some('|' | '&' | ')'), _) => all,
            _ => self.magic == Magic::All,
        }
    }

    /// 结束当前的原子：其后的 `^` 不再表示行首，`*` 恢复为量词。
    fn atom_from(&mut self, start: usize) {
        self.atom = Some(start);
        self.at_start = false;
    }

    /// 返回当前最外层分支的起点。
    fn branch(&self) -> usize {
        self.branches.last().copied().unwrap_or(0)
    }

    /// 结束一个分支：补上 `\ze` 留下的右括号。
    fn end_branch(&mut self) {
        if self.branches.len() == 1 {
            for _ in 0..self.closers {
                self.out.push(')');
            }
            self.closers = 0;
        }
    }

    /// 转换整个模式。
    fn run(&mut self) -> Result<(), usize> {
        while let Some(token) = self.peek() {
            let at = self.at;
            self.at += token.len;
            if token.magic {
                self.special(token.c, at)?;
            } else {
                let start = self.out.len();
                push_literal(&mut self.out, token.c);
                self.atom_from(start);
                self.literal_end = Some(self.out.len());
            }
        }
        if !self.opens.is_empty() {
            return Err(self.chars.len());
        }
        self.end_branch();
        Ok(())
    }

    /// 处理有特殊含义的记号 `c`；`at` 是该记号在模式中的位置。
    fn special(&mut self, c: char, at: usize) -> Result<(), usize> {
        let start = self.out.len();
        match c {
            'v' => self.magic = Magic::All,
            'm' => self.magic = Magic::On,
            'M' => self.magic = Magic::Off,
            'V' => self.magic = Magic::None,
            'c' => self.case = Some(true),
            'C' => self.case = Some(false),
            'Z' => {}
            '^' => {
                self.out.push('^');
                self.atom = None;
            }
            '$' => {
                self.out.push('$');
                self.atom = None;
                self.at_start = false;
            }
            '.' => {
                self.out.push('.');
                self.atom_from(start);
            }
            '<' => {
                self.out.push_str("\\b{start}");
                self.atom = None;
                self.at_start = false;
            }
            '>' => {
                self.out.push_str("\\b{end}");
                self.atom = None;
                self.at_start = false;
            }
            '|' => {
                self.end_branch();
                self.out.push('|');
                if let Some(branch) = self.branches.last_mut() {
                    *branch = self.out.len();
                }
                self.atom = None;
                self.at_start = true;
            }
            '(' => self.open("("),
            ')' => {
                let open = self.opens.pop().ok_or(at)?;
                self.branches.pop();
                self.out.push(')');
                self.atom_from(open);
            }
            '%' => self.percent(at)?,
            'z' => self.bounds(at)?,
            '_' => self.with_newline(at)?,
            '*' => self.multi("*", at)?,
            '+' => self.multi("+", at)?,
            '=' | '?' => self.multi("?", at)?,
            '{' => self.braces(at)?,
            '@' => self.lookaround(at)?,
            '[' => self.collection(at, false)?,
            'n' => {
                self.out.push_str("\\n");
                self.atom_from(start);
                self.at_start = true;
            }
            '1'..='9' => {
                self.out.push('\\');
                self.out.push(c);
                self.atom_from(start);
            }
            // 上一次替换的文本与分支的同时匹配不在转换的范围内。
            '~' | '&' => return Err(at),
            c => match class_of(c) {
                Some((set, negated)) => {
                    self.push_class(set, negated, false, matches!(c, 'l' | 'L' | 'u' | 'U'));
                    self.atom_from(start);
                }
                None => {
                    push_literal(&mut self.out, c);
                    self.atom_from(start);
                    self.literal_end = Some(self.out.len());
                }
            },
        }
        Ok(())
    }

    /// 开始一个分组，`prefix` 是左括号的写法。
    fn open(&mut self, prefix: &str) {
        self.opens.push(self.out.len());
        self.out.push_str(prefix);
        self.branches.push(self.out.len());
        self.atom = None;
        self.at_start = true;
    }

    /// 写入字符类：`set` 是方括号之内的写法，`newline` 为真时另外匹配换行，`cased` 为真时
    /// 不受忽略大小写的影响。
    fn push_class(&mut self, set: &str, negated: bool, newline: bool, cased: bool) {
        if cased {
            self.out.push_str("(?-i:");
        }
        self.out.push('[');
        if negated {
            self.out.push('^');
        }
        self.out.push_str(set);
        // 取反的字符类不匹配换行；带换行的形式匹配。
        if negated != newline {
            self.out.push_str("\\n");
            if negated {
                self.out.push_str("\\r");
            }
        }
        self.out.push(']');
        if cased {
            self.out.push(')');
        }
    }

    /// 处理 `\_` 开头的写法：其后的字符类或 `.` 另外匹配换行。
    fn with_newline(&mut self, at: usize) -> Result<(), usize> {
        let start = self.out.len();
        let c = self.get(self.at).ok_or(at)?;
        self.at += 1;
        match c {
            '.' => self.out.push_str("(?s:.)"),
            '^' => {
                self.out.push('^');
                self.atom = None;
                return Ok(());
            }
            '$' => {
                self.out.push('$');
                self.atom = None;
                self.at_start = false;
                return Ok(());
            }
            '[' => return self.collection(at, true),
            c => {
                let (set, negated) = class_of(c).ok_or(at)?;
                self.push_class(set, negated, true, matches!(c, 'l' | 'L' | 'u' | 'U'));
            }
        }
        self.atom_from(start);
        Ok(())
    }

    /// 处理 `\%` 开头的写法。
    fn percent(&mut self, at: usize) -> Result<(), usize> {
        let start = self.out.len();
        let c = self.get(self.at).ok_or(at)?;
        self.at += 1;
        match c {
            '(' => {
                self.open("(?:");
                return Ok(());
            }
            '^' => {
                self.out.push_str("\\A");
                self.atom = None;
                return Ok(());
            }
            '$' => {
                self.out.push_str("\\z");
                self.atom = None;
                self.at_start = false;
                return Ok(());
            }
            'd' | 'x' | 'u' | 'U' | 'o' => {
                let (radix, max) = match c {
                    'd' => (10, 10),
                    'o' => (8, 4),
                    'x' => (16, 2),
                    'u' => (16, 4),
                    _ => (16, 8),
                };
                let mut value = 0u32;
                let mut digits = 0;
                while digits < max
                    && let Some(digit) = self.get(self.at).and_then(|c| c.to_digit(radix))
                {
                    value = value.checked_mul(radix).ok_or(at)? + digit;
                    self.at += 1;
                    digits += 1;
                }
                let c = char::from_u32(value).filter(|_| digits > 0).ok_or(at)?;
                self.out.push_str(&format!("\\x{{{:X}}}", u32::from(c)));
            }
            // 光标、可视区域、行号、列号、标记等依赖编辑状态的写法无法转换。
            _ => return Err(at),
        }
        self.atom_from(start);
        Ok(())
    }

    /// 处理 `\zs` 与 `\ze`：只能出现在最外层。
    fn bounds(&mut self, at: usize) -> Result<(), usize> {
        let c = self.get(self.at).ok_or(at)?;
        self.at += 1;
        if self.branches.len() != 1 {
            return Err(at);
        }
        match c {
            's' => {
                let branch = self.branch();
                if self.out.len() > branch {
                    self.out.insert_str(branch, "(?<=");
                    self.out.push(')');
                }
            }
            'e' => {
                self.out.push_str("(?=");
                self.closers += 1;
            }
            _ => return Err(at),
        }
        self.atom = None;
        Ok(())
    }

    /// 给最近的原子加上量词 `suffix`；没有原子时返回错误。
    fn multi(&mut self, suffix: &str, at: usize) -> Result<(), usize> {
        if self.atom.take().is_none() {
            return Err(at);
        }
        self.out.push_str(suffix);
        Ok(())
    }

    /// 处理 `\{n,m}`、`\{-n,m}` 等次数的写法。
    fn braces(&mut self, at: usize) -> Result<(), usize> {
        let mut lazy = false;
        if self.get(self.at) == Some('-') {
            lazy = true;
            self.at += 1;
        }
        let number = |this: &mut Self| {
            let mut digits = String::new();
            while let Some(c) = this.get(this.at).filter(char::is_ascii_digit) {
                digits.push(c);
                this.at += 1;
            }
            digits
        };
        let low = number(self);
        let mut high = None;
        if self.get(self.at) == Some(',') {
            self.at += 1;
            high = Some(number(self));
        }
        // 结尾的 `}` 之前可以有反斜杠。
        if self.get(self.at) == Some('\\') {
            self.at += 1;
        }
        if self.get(self.at) != Some('}') {
            return Err(at);
        }
        self.at += 1;
        let quantifier = match (low.as_str(), high.as_deref()) {
            ("", None | Some("")) => "*".to_owned(),
            (low, None) => format!("{{{low}}}"),
            ("", Some(high)) => format!("{{0,{high}}}"),
            (low, Some(high)) => format!("{{{low},{high}}}"),
        };
        self.multi(&quantifier, at)?;
        if lazy {
            self.out.push('?');
        }
        Ok(())
    }

    /// 处理 `\@=`、`\@!`、`\@<=`、`\@<!`、`\@>`：把最近的原子包成环视或原子组。
    fn lookaround(&mut self, at: usize) -> Result<(), usize> {
        // 向前文环视可以带回看的字节数，转换时忽略。
        while self.get(self.at).is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        let prefix = match (self.get(self.at), self.get(self.at + 1)) {
            (Some('='), _) => "(?=",
            (Some('!'), _) => "(?!",
            (Some('>'), _) => "(?>",
            (Some('<'), Some('=')) => "(?<=",
            (Some('<'), Some('!')) => "(?<!",
            _ => return Err(at),
        };
        self.at += if prefix.len() == 4 { 2 } else { 1 };
        let start = self.atom.take().ok_or(at)?;
        self.out.insert_str(start, prefix);
        self.out.push(')');
        Ok(())
    }

    /// 处理 `[` 开头的字符集合；`newline` 为真时另外匹配换行。没有配对的 `]` 时 `[` 按字面
    /// 处理。
    fn collection(&mut self, at: usize, newline: bool) -> Result<(), usize> {
        let start = self.out.len();
        let mut pos = self.at;
        let negated = self.get(pos) == Some('^');
        if negated {
            pos += 1;
        }
        let mut set = String::new();
        let mut first = true;
        let push = |set: &mut String, c: char| {
            if "\\[]^-&~".contains(c) {
                set.push('\\');
            }
            set.push(c);
        };
        loop {
            let Some(c) = self.get(pos) else {
                // 没有结尾：`[` 是字面字符。
                push_literal(&mut self.out, '[');
                self.atom_from(start);
                return Ok(());
            };
            if c == ']' && !first {
                pos += 1;
                break;
            }
            first = false;
            // 一个成员：字符类、转义的字符，或字面字符。
            let member = if c == '[' && self.get(pos + 1) == Some(':') {
                let end = (pos + 2..self.chars.len())
                    .find(|&i| self.get(i) == Some(':') && self.get(i + 1) == Some(']'));
                match end {
                    Some(end) => {
                        let name: String = self.chars[pos + 2..end].iter().collect();
                        let known = [
                            "alpha", "digit", "alnum", "lower", "upper", "punct", "space",
                            "xdigit", "blank", "cntrl", "graph", "print",
                        ];
                        if !known.contains(&name.as_str()) {
                            return Err(at);
                        }
                        set.push_str(&format!("[:{name}:]"));
                        pos = end + 2;
                        continue;
                    }
                    None => {
                        pos += 1;
                        '['
                    }
                }
            } else if c == '\\' {
                let next = self.get(pos + 1);
                pos += 2;
                match next {
                    Some('e') => '\u{1b}',
                    Some('t') => '\t',
                    Some('r') => '\r',
                    Some('b') => '\u{8}',
                    Some('n') => '\n',
                    Some(c @ ('\\' | ']' | '^' | '-')) => c,
                    Some(c @ ('d' | 'o' | 'x' | 'u' | 'U')) => {
                        let (radix, max) = match c {
                            'd' => (10, 10),
                            'o' => (8, 4),
                            'x' => (16, 2),
                            'u' => (16, 4),
                            _ => (16, 8),
                        };
                        let mut value = 0u32;
                        let mut digits = 0;
                        while digits < max
                            && let Some(digit) = self.get(pos).and_then(|c| c.to_digit(radix))
                        {
                            value = value.checked_mul(radix).ok_or(at)? + digit;
                            pos += 1;
                            digits += 1;
                        }
                        if digits == 0 {
                            // 不是数值：反斜杠与字母各是一个成员。
                            push(&mut set, '\\');
                            c
                        } else {
                            char::from_u32(value).ok_or(at)?
                        }
                    }
                    Some(other) => {
                        // 其余字符之前的反斜杠是字面字符。
                        push(&mut set, '\\');
                        other
                    }
                    None => {
                        pos -= 1;
                        '\\'
                    }
                }
            } else {
                pos += 1;
                c
            };
            // 范围：成员之后是 `-` 与另一个不是 `]` 的字符。
            if self.get(pos) == Some('-')
                && let Some(high) = self.get(pos + 1).filter(|c| *c != ']' && *c != '\\')
            {
                if high < member {
                    return Err(at);
                }
                push(&mut set, member);
                set.push('-');
                push(&mut set, high);
                pos += 2;
            } else {
                push(&mut set, member);
            }
        }
        self.at = pos;
        if set.is_empty() {
            return Err(at);
        }
        self.push_class(&set, negated, newline, false);
        self.atom_from(start);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
