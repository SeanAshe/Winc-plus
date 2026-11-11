//! 对照用例的生成：把动作、操作符、编辑命令与一组有代表性的文本、光标位置组合成用例。
//!
//! 生成的结果是确定的：同一份代码每次生成相同的用例文件。用例的名字由所在的组与序号组成。

use winc_vim::fixture::{Case, File, Position};
use winc_vim::{Key, notation};

mod commands;
mod navigation;

use commands::{commands, macros};
use navigation::{marks, objects, searches, views};

/// 一份初始文本及其上的若干光标位置（行号，字符列）。
struct Buffer {
    /// 各行文本。
    text: &'static [&'static str],
    /// 光标位置，都在某个字符上（空行为第 0 列）。
    cursors: &'static [(usize, usize)],
}

/// 词、标点、宽字符与组合字符。
const WORDS: Buffer = Buffer {
    text: &[
        "foo bar.baz  qux_1 (x) end",
        "   lead, tail;  ",
        "",
        "中文 text，全角。next",
        "a-b a1b2 #$% e\u{301}x",
    ],
    cursors: &[
        (0, 0),
        (0, 5),
        (0, 7),
        (0, 11),
        (0, 20),
        (0, 25),
        (1, 0),
        (1, 3),
        (1, 15),
        (2, 0),
        (3, 1),
        (3, 9),
        (4, 0),
        (4, 13),
        (4, 15),
    ],
};

/// 括号、引号与第一列的花括号。
const CODE: Buffer = Buffer {
    text: &[
        "fn main() {",
        "    call(a, [b, c], \"s)\");",
        "    if (x) { y('(') }",
        "}",
        "",
        "{",
        "  inner } {",
        "}",
        "tail",
    ],
    cursors: &[
        (0, 0),
        (0, 7),
        (0, 10),
        (1, 4),
        (1, 8),
        (1, 13),
        (1, 21),
        (2, 7),
        (2, 16),
        (3, 0),
        (4, 0),
        (6, 4),
        (6, 10),
        (8, 2),
    ],
};

/// 句子、段落、制表符与只有空白的行。
const PROSE: Buffer = Buffer {
    text: &[
        "First one.  Second one! Third?  ",
        "Still (third.) Fourth.",
        "",
        "",
        "New para here. End",
        "\tTabbed\tline here",
        "   ",
        "last",
    ],
    cursors: &[
        (0, 0),
        (0, 9),
        (0, 14),
        (0, 31),
        (1, 8),
        (1, 21),
        (2, 0),
        (4, 4),
        (4, 17),
        (5, 0),
        (5, 3),
        (5, 7),
        (6, 1),
        (7, 3),
    ],
};

/// 只有一行、只有空行的文本。
const TINY: [Buffer; 3] = [
    Buffer {
        text: &["x"],
        cursors: &[(0, 0)],
    },
    Buffer {
        text: &[""],
        cursors: &[(0, 0)],
    },
    Buffer {
        text: &["ab cd", ""],
        cursors: &[(0, 4), (1, 0)],
    },
];

/// 各种写法的数。
const NUMBERS: Buffer = Buffer {
    text: &[
        "x 7 -3 007 0x0f 0XFF 0b101 y",
        "9 -1 abc08 1-2 99 0 -0x1 08",
        "no digits here",
        "18446744073709551615 0xfF 0x 0b2 -",
    ],
    cursors: &[
        (0, 0),
        (0, 2),
        (0, 4),
        (0, 5),
        (0, 8),
        (0, 12),
        (0, 14),
        (0, 17),
        (0, 23),
        (0, 27),
        (1, 0),
        (1, 2),
        (1, 6),
        (1, 11),
        (1, 12),
        (1, 15),
        (1, 18),
        (1, 20),
        (1, 25),
        (2, 3),
        (3, 0),
        (3, 21),
        (3, 26),
        (3, 29),
        (3, 33),
    ],
};

/// 长短不一的行，用于按块的选区。
const BLOCK: Buffer = Buffer {
    text: &[
        "abcdefgh",
        "ij",
        "klmnopqrst",
        "",
        "uvwxyz12",
        "  indented line",
        "last one here",
    ],
    cursors: &[(0, 0), (0, 2), (0, 6), (1, 1), (2, 4), (4, 3), (5, 2)],
};

/// 把文字 `text` 写成依次键入各字符的按键记法。
fn typed(text: &str) -> String {
    let keys: Vec<Key> = text.chars().map(Key::char).collect();
    notation::write(&keys)
}

/// 不带参数的动作。
const MOTIONS: &[&str] = &[
    "h", "l", "<BS>", "<Space>", "0", "^", "$", "g_", "g0", "g^", "g$", "gm", "|", "k", "j", "gk",
    "gj", "-", "+", "<CR>", "_", "gg", "G", "<C-End>", "%", "H", "M", "L", "w", "W", "b", "B", "e",
    "E", "ge", "gE", "(", ")", "{", "}", "[[", "]]", "[]", "][", "[(", "[{", "])", "]}",
];

/// 与上面的动作等价的功能键。
const KEY_MOTIONS: &[&str] = &[
    "<Left>",
    "<Right>",
    "<Up>",
    "<Down>",
    "<Home>",
    "<End>",
    "<S-Right>",
    "<C-Right>",
    "<S-Left>",
    "<C-Left>",
    "<C-Home>",
    "g<Home>",
    "g<End>",
    "g<Up>",
    "g<Down>",
    "<C-h>",
    "<C-j>",
    "<C-n>",
    "<C-p>",
];

/// 行内查找的按键序列。
const FINDS: &[&str] = &[
    "fa", "Fa", "ta", "Ta", "f<Space>", "f.", "t)", "F(", "2fa", "fa;", "fa,", "ta;", "Ta;", "ta,",
    "fa2;", "fz", "f中", "t。",
];

/// 已知结果与 `vim` 不同的生成用例：按键、文本的第一行与光标。这些用例带 `known` 标记，
/// 对照测试核对结果确实不同。
///
/// 这些命令没有改动文本，`vim` 仍为它们记录一个撤销步骤：`x` 在空行上、`3J` 在最后一行上。
/// 之后的撤销因此少撤回一步，或只把光标移回原处。最后两个是替换：`vim` 的 `\w\+` 在这里不匹配
/// 带组合字符的字母。
const KNOWN: &[(&str, &str, (usize, usize))] = &[
    (r":s/\w\+/\u&/g<CR>", "foo bar.baz  qux_1 (x) end", (4, 0)),
    (r":s/\w\+/\u&/g<CR>", "foo bar.baz  qux_1 (x) end", (4, 15)),
    ("xxu", "fn main() {", (3, 0)),
    ("xxx2u", "fn main() {", (3, 0)),
    ("3Ju", "foo bar.baz  qux_1 (x) end", (4, 13)),
    ("3Ju", "foo bar.baz  qux_1 (x) end", (4, 15)),
    ("3Ju", "fn main() {", (8, 2)),
    ("3Ju", "First one.  Second one! Third?  ", (7, 3)),
];

/// 生成器的输出：一个用例文件的主文件名与用例。
pub(super) struct Group {
    /// 主文件名。
    pub name: &'static str,
    /// 文件开头的说明。
    pub about: &'static str,
    /// 用例。
    pub file: File,
}

/// 默认生成的用例中，每个按键序列取用的光标位置相对全部组合的间隔。
///
/// 全部组合约 1.8 万个用例，录制文件约 3 MB；取四分之一后各按键序列仍然轮流覆盖全部光标位置。
const THIN: usize = 4;

/// 逐个添加用例并编号。
struct Builder {
    /// 名字的前缀。
    prefix: &'static str,
    /// 是否生成全部组合。
    full: bool,
    /// 已添加的用例。
    cases: Vec<Case>,
}

impl Builder {
    fn new(prefix: &'static str, full: bool) -> Self {
        Self {
            prefix,
            full,
            cases: Vec::new(),
        }
    }

    /// 添加一个用例：文本 `text`、光标 `cursor`、按键 `keys`。
    fn add(&mut self, text: &[&str], cursor: (usize, usize), keys: &str) {
        self.cases.push(Case {
            name: format!("{}{:04}", self.prefix, self.cases.len() + 1),
            text: text.iter().map(|line| (*line).to_owned()).collect(),
            cursor: Position {
                line: cursor.0,
                col: cursor.1,
            },
            keys: keys.to_owned(),
            ..Case::default()
        });
    }

    /// 在 `buffer` 的每个光标位置上各添加一个按键为 `keys` 的用例。
    fn each(&mut self, buffer: &Buffer, keys: &str) {
        self.some(buffer, 1, keys);
    }

    /// 在 `buffer` 的光标位置中每隔 `step` 个取一个，各添加一个用例。
    ///
    /// 不生成全部组合时间隔再乘以 [`THIN`]，起点随按键序列变化，使各序列取用不同的位置。
    fn some(&mut self, buffer: &Buffer, step: usize, keys: &str) {
        let (step, skip) = if self.full || buffer.cursors.len() <= 2 {
            (step, 0)
        } else {
            let step = step * THIN;
            let seed: usize = keys.bytes().map(usize::from).sum();
            (step, seed % step.min(buffer.cursors.len()))
        };
        for cursor in buffer.cursors.iter().skip(skip).step_by(step) {
            self.add(buffer.text, *cursor, keys);
        }
    }

    fn finish(mut self, name: &'static str, about: &'static str) -> Group {
        for case in &mut self.cases {
            let position = (case.cursor.line, case.cursor.col);
            let known = KNOWN.iter().any(|(keys, first, cursor)| {
                case.keys == *keys && case.text[0] == *first && position == *cursor
            });
            if known {
                case.tags.push("known".into());
            }
        }
        Group {
            name,
            about,
            file: File {
                vim: None,
                cases: self.cases,
            },
        }
    }
}

/// 返回生成的用例文件；`full` 为真时生成全部组合，否则取其中的一部分。
pub(super) fn generated(full: bool) -> Vec<Group> {
    vec![
        motions(full),
        operators(full),
        edits(full),
        inserts(full),
        visuals(full),
        undo(full),
        objects(full),
        searches(full),
        marks(full),
        views(full),
        typing(full),
        numbers(full),
        blocks(full),
        commands(full),
        macros(full),
    ]
}

/// 单独执行的动作：核对光标的位置。
fn motions(full: bool) -> Group {
    let mut b = Builder::new("m", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for motion in MOTIONS {
            b.each(buffer, motion);
            b.some(buffer, 2, &format!("2{motion}"));
        }
        for motion in KEY_MOTIONS {
            b.some(buffer, 3, motion);
        }
        for find in FINDS {
            b.some(buffer, 2, find);
        }
        // 上下移动保持期望列，`$` 之后停在行尾。
        for keys in [
            "jj", "jk", "$j", "$jj", "$kk", "3|j", "lljjkk", "gmj", "g_j", "$hj",
        ] {
            b.some(buffer, 2, keys);
        }
        for keys in [
            "3G",
            "1G",
            "99G",
            "3gg",
            "50%",
            "100%",
            "1%",
            "101%",
            "4|",
            "30|",
            "99|",
            "2H",
            "2L",
            "99H",
            "3w",
            "5w",
            "99w",
            "3b",
            "99b",
            "3e",
            "3ge",
            "99h",
            "99l",
            "3<Space>",
            "99<Space>",
            "3<BS>",
            "99<BS>",
            "3j",
            "99j",
            "3k",
            "99k",
            "3+",
            "3-",
            "3_",
            "2$",
            "3$",
            "99$",
            "3)",
            "3(",
            "3}",
            "3{",
            "2g_",
            "2%",
        ] {
            b.some(buffer, 3, keys);
        }
    }
    for buffer in &TINY {
        for motion in MOTIONS {
            b.each(buffer, motion);
        }
    }
    b.finish("motions", "单独执行的动作：核对光标的位置。")
}

/// 操作符与动作的组合，叠写形式，强制类型，同义写法。
fn operators(full: bool) -> Group {
    let mut b = Builder::new("o", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for motion in MOTIONS.iter().chain(KEY_MOTIONS) {
            b.some(buffer, 2, &format!("d{motion}"));
            b.some(buffer, 4, &format!("d2{motion}"));
            b.some(buffer, 3, &format!("y{motion}"));
            b.some(buffer, 4, &format!("c{motion}XY<Esc>"));
        }
        for find in FINDS {
            b.some(buffer, 3, &format!("d{find}"));
            b.some(buffer, 5, &format!("y{find}"));
        }
        for operator in [">", "<lt>", "g~", "gu", "gU", "g?"] {
            for motion in ["l", "w", "$", "j", "k", "G", "}", "2j", "e", "0", "gg"] {
                b.some(buffer, 4, &format!("{operator}{motion}"));
            }
        }
        for keys in [
            "dd",
            "2dd",
            "3dd",
            "99dd",
            "d2d",
            "2d2d",
            "cc",
            "2ccZ<Esc>",
            "yy",
            "2yy",
            "99yy",
            ">>",
            "2>>",
            "<lt><lt>",
            "2<lt><lt>",
            ">><lt><lt>",
            "g~~",
            "g~g~",
            "2g~~",
            "guu",
            "gugu",
            "gUU",
            "gUgU",
            "g??",
            "g?g?",
            "3g??",
        ] {
            b.some(buffer, 2, keys);
        }
        for keys in [
            "dvj",
            "dvk",
            "dVw",
            "dVl",
            "dvw",
            "dve",
            "dv$",
            "yvj",
            "dvd",
            "dv2j",
            "d2vj",
            "dvb",
            "dVgg",
            "dvG",
            "cvwQ<Esc>",
        ] {
            b.some(buffer, 3, keys);
        }
        for keys in [
            "x", "3x", "99x", "X", "2X", "99X", "D", "2D", "99D", "CQ<Esc>", "2CQ<Esc>", "sQ<Esc>",
            "3sQ<Esc>", "SQ<Esc>", "2SQ<Esc>", "Y", "2Y", "<Del>", "2<Del>",
        ] {
            b.some(buffer, 2, keys);
        }
        // 寄存器：具名、追加、黑洞，以及编号寄存器的后移。
        for keys in [
            "\"ayw",
            "\"ayy",
            "\"add",
            "\"adw",
            "\"ayw\"Ayw",
            "\"ayy\"Ayw",
            "\"ayw\"Ayy",
            "\"_dd",
            "\"_dw",
            "\"_x",
            "dddd",
            "dddddd",
            "dwdd",
            "dddw",
            "ywdd",
            "yydw",
            "xdd",
            "d}",
            "d)",
            "d%",
            "\"bd}",
            "yw\"cyy",
            "\"zx\"zx",
            "ddjdd\"1yy",
        ] {
            b.some(buffer, 3, keys);
        }
    }
    for buffer in &TINY {
        for keys in [
            "dd", "dw", "d$", "dl", "dh", "dj", "dk", "dG", "dgg", "x", "X", "D", "yy", "yw", "cc",
            "cwZ<Esc>", "ccZ<Esc>", "SZ<Esc>", ">>", "<lt><lt>", "g~~", "2dd", "d}", "d{",
        ] {
            b.each(buffer, keys);
        }
    }
    b.finish(
        "operators",
        "操作符与动作的组合、叠写形式、强制类型、同义写法与寄存器。",
    )
}

/// 替换、切换大小写、合并行、粘贴与点重复。
fn edits(full: bool) -> Group {
    let mut b = Builder::new("e", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for keys in [
            "rQ", "3rQ", "99rQ", "r<CR>", "2r<CR>", "r<Tab>", "r<Space>", "r中", "~", "3~", "99~",
            "J", "3J", "99J", "gJ", "3gJ", "JJ", "Jx", "2J",
        ] {
            b.some(buffer, 2, keys);
        }
        for keys in [
            "yyp",
            "yyP",
            "yy3p",
            "yy2P",
            "ywp",
            "ywP",
            "yw3p",
            "yw2P",
            "yjp",
            "yjP",
            "y$p",
            "y$P",
            "ddp",
            "ddP",
            "xp",
            "xP",
            "dwP",
            "dwp",
            "yy]p",
            "yj]p",
            "yw]p",
            "yygp",
            "yygP",
            "yy2gp",
            "ywgp",
            "ywgP",
            "yw2gp",
            "yjgp",
            "yjGgp",
            "yyGp",
            "yyGP",
            "yyggP",
            "y}p",
            "y}P",
            "y)p",
            "y}gp",
            "\"ayy\"ap",
            "\"ayw\"Ayw\"ap",
            "\"ayy\"Ayw\"ap",
            "\"ayw\"Ayy\"ap",
            "yyjdd\"0p",
            "ywdw\"0P",
            "dddd\"1p",
            "dddd\"2p",
            "dddd\"1p.",
            "dwdw\"-p",
            "ywjp",
            "y2wj$p",
            "yljP",
            "p",
            "P",
            "\"qp",
        ] {
            b.some(buffer, 3, keys);
        }
        for keys in [
            "x.",
            "3x.",
            "x2.",
            "3x..",
            "dw.",
            "dw..",
            "dw2.",
            "d2w.",
            "dd.",
            "dd2.",
            "2dd.",
            "rQ.",
            "3rQl.",
            "~.",
            "J.",
            ">>.",
            "g~w.",
            "guw.",
            "gUUj.",
            "yyp.",
            "yyP.",
            "ywp.",
            "ywp2.",
            "D.",
            "Dj.",
            "X.",
            "dl.",
            "d$j.",
            "dfaj.",
            "dta.",
            "\"adw.",
            "\"add.\"ap",
        ] {
            b.some(buffer, 3, keys);
        }
    }
    b.finish("edits", "替换字符、切换大小写、合并行、粘贴与点重复。")
}

/// 进入插入模式的命令、输入的重复与点重复。
fn inserts(full: bool) -> Group {
    let mut b = Builder::new("i", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for keys in [
            "iab<Esc>",
            "aab<Esc>",
            "Iab<Esc>",
            "Aab<Esc>",
            "gIab<Esc>",
            "oab<Esc>",
            "Oab<Esc>",
            "i<Esc>",
            "a<Esc>",
            "A<Esc>",
            "I<Esc>",
            "o<Esc>",
            "O<Esc>",
            "3iab<Esc>",
            "2aab<Esc>",
            "2Iab<Esc>",
            "2Aab<Esc>",
            "3oab<Esc>",
            "2Oab<Esc>",
            "ia<CR>b<Esc>",
            "2ia<CR>b<Esc>",
            "Aa<CR>b<Esc>",
            "<Insert>ab<Esc>",
        ] {
            b.some(buffer, 2, keys);
        }
        for keys in [
            "iab<Esc>.",
            "iab<Esc>j.",
            "aab<Esc>..",
            "Aab<Esc>j.",
            "Iab<Esc>j.",
            "oab<Esc>.",
            "Oab<Esc>.",
            "3iab<Esc>.",
            "iab<Esc>3.",
            "3iab<Esc>2.",
            "oab<Esc>2.",
            "2oab<Esc>.",
            "cwab<Esc>.",
            "cwab<Esc>w.",
            "cwab<Esc>ww.",
            "c2wab<Esc>w.",
            "ceab<Esc>w.",
            "c$ab<Esc>j.",
            "Cab<Esc>j.",
            "ccab<Esc>j.",
            "Sab<Esc>j.",
            "sab<Esc>l.",
            "3sab<Esc>l.",
            "cfaZ<Esc>;.",
            "ia<CR>b<Esc>j.",
            "iab<Esc>giXY<Esc>",
            "Aab<Esc>ggiQ<Esc>giR<Esc>",
            "iab<Esc>u",
            "iab<Esc>uu",
            "oab<Esc>u",
            "cwab<Esc>u",
            "3iab<Esc>u",
            "iab<Esc>u.",
        ] {
            b.some(buffer, 3, keys);
        }
    }
    for buffer in &TINY {
        for keys in [
            "iab<Esc>",
            "aab<Esc>",
            "Aab<Esc>",
            "oab<Esc>",
            "Oab<Esc>",
            "3iab<Esc>",
        ] {
            b.each(buffer, keys);
        }
    }
    b.finish("inserts", "进入插入模式的命令、输入的重复与点重复。")
}

/// 可视模式：选区的延伸、操作符、粘贴与点重复。
fn visuals(full: bool) -> Group {
    let mut b = Builder::new("v", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for motion in [
            "l", "h", "j", "k", "w", "b", "e", "$", "0", "^", "G", "gg", "}", "{", ")", "%", "2j",
            "3l", "fa", "g_", "<Space>", "<BS>", "ge", "W", "+", "-",
        ] {
            b.some(buffer, 3, &format!("v{motion}d"));
            b.some(buffer, 4, &format!("v{motion}y"));
            b.some(buffer, 4, &format!("V{motion}d"));
            b.some(buffer, 5, &format!("V{motion}y"));
        }
        for keys in [
            "vd",
            "vy",
            "Vd",
            "Vy",
            "vld",
            "vlly",
            "v$d",
            "v$y",
            "v$hd",
            "vj$d",
            "vllly",
            "vlcQ<Esc>",
            "vjcQ<Esc>",
            "VcQ<Esc>",
            "VjcQ<Esc>",
            "vlsQ<Esc>",
            "vlCQ<Esc>",
            "vlRQ<Esc>",
            "vjJ",
            "vjgJ",
            "VjJ",
            "V2jJ",
            "vJ",
            "VgJ",
            "v3lrQ",
            "VrQ",
            "vjrQ",
            "vlu",
            "vlU",
            "vl~",
            "Vu",
            "VU",
            "V~",
            "vjU",
            "vlg?",
            "vjD",
            "vlX",
            "vjY",
            "vlx",
            "vl<Del>",
            "v>",
            "Vj>",
            "Vj2>",
            "Vj<lt>",
            "vj3<lt>",
            "V>.",
            "vlod",
            "vjoy",
            "vjokd",
            "vlO<Esc>x",
            "vjOd",
            "vl<Esc>x",
            "vj<Esc>x",
            "v$<Esc>x",
            "Vj<Esc>x",
            "vlvx",
            "vlVd",
            "VjvD",
            "Vlvd",
            "vl<Esc>gvd",
            "Vj<Esc>jgvd",
            "vl<Esc>jgvy",
            "vj<Esc>ggvlgvd",
            "v\"ay",
            "vl\"ad",
            "Vj\"ay",
            "vl\"_d",
            "vly\"0P",
        ] {
            b.some(buffer, 2, keys);
        }
        for keys in [
            "ywjvlp", "ywjvlP", "yyjvlp", "yyjVp", "ywjVp", "ywjVjp", "yyvjp", "ywvep", "ywve2p",
            "yyjv$p", "ywjv$p", "dwvlp", "ddVp", "ywvlpp", "yyVpu",
        ] {
            b.some(buffer, 3, keys);
        }
        for keys in [
            "vld.",
            "vlldj.",
            "vjd.",
            "Vd.",
            "Vjd.",
            "vly.",
            "vlcQ<Esc>j.",
            "VjcQ<Esc>.",
            "vl~l.",
            "Vj>.",
            "vllrQj.",
            "vjJ.",
            "vlUw.",
        ] {
            b.some(buffer, 3, keys);
        }
    }
    b.finish("visuals", "可视模式：选区的延伸、操作符、粘贴与点重复。")
}

/// 撤销与重做之后的文本与光标。
fn undo(full: bool) -> Group {
    let mut b = Builder::new("u", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for keys in [
            "xu",
            "3xu",
            "xu<C-r>",
            "xxu",
            "xxuu",
            "xxx2u",
            "xxxu<C-r><C-r>",
            "xxuu<C-r>",
            "dwu",
            "dbu",
            "d$u",
            "d0u",
            "ddu",
            "2ddu",
            "dju",
            "dku",
            "dGu",
            "dggu",
            "d}u",
            "d{u",
            "ddu<C-r>",
            "dku<C-r>",
            "dwu<C-r>",
            "dbu<C-r>",
            "yypu",
            "yyPu",
            "ywpu",
            "ywPu",
            "yypu<C-r>",
            "yyPu<C-r>",
            "ywpu<C-r>",
            "Ju",
            "3Ju",
            "Ju<C-r>",
            "rQu",
            ">>u",
            "g~~u",
            "g~wu",
            "gUju",
            "Dju",
            "ddjddu",
            "ddjdduu",
            "u",
            "<C-r>",
            "xu<C-r>x",
            "Aab<Esc>u",
            "Iab<Esc>u",
            "Oab<Esc>u",
            "oab<Esc>u<C-r>",
            "iab<Esc>u<C-r>",
            "cwab<Esc>u<C-r>",
            "vjdu",
            "Vjdu",
            "vldu",
            "vjdu<C-r>",
            "x3u",
            "xxx3u<C-r>",
            "xxx3u2<C-r>",
        ] {
            b.some(buffer, 3, keys);
        }
    }
    b.finish("undo", "撤销与重做之后的文本与光标。")
}

/// 替换模式与插入模式的 `Ctrl` 键。
fn typing(full: bool) -> Group {
    let mut b = Builder::new("y", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for keys in [
            "RXY<Esc>",
            "3RXY<Esc>",
            "RXY<BS><Esc>",
            "RXY<BS><BS><BS>Z<Esc>",
            "RX<CR>Y<Esc>",
            "RXY<Esc>.",
            "RXY<Esc>j.",
            "RXY<Esc>3.",
            "R<Esc>",
            "Rabcdefghijklmnopqrstuvwxyz0123456789<Esc>",
            "RXY<Esc>u",
            "RXYZ<Esc>u<C-r>",
            "R中<Esc>",
            "RX<C-h>Y<Esc>",
            "RXY<C-c>",
            "2RX<CR><Esc>",
            "iab<Space>cd<C-w>X<Esc>",
            "Oab<Space>cd<C-u>X<Esc>",
            "Oab<Space>cd<C-w><C-w>X<Esc>",
            "iab<C-h>c<Esc>",
            "ia<C-t>b<Esc>",
            "ia<C-t><C-t>b<Esc>",
            "ia<C-t><C-d>b<Esc>",
            "ia<C-d>b<Esc>",
            "ywA<C-r>\"<Esc>",
            "ywA<C-r>0<C-r>0<Esc>",
            "\"aylA<C-r>a<C-r>a<Esc>",
            "A<C-r>z<Esc>",
            "A<C-r><Esc>x<Esc>",
            "ia<C-o>xb<Esc>",
            "A<C-o>0x<Esc>",
            "Aab<C-o>hX<Esc>",
            "A<C-o>$X<Esc>",
            "A<C-o>xX<Esc>",
            "ia<C-o>dwb<Esc>",
            "i<C-o>2lX<Esc>",
            "ia<C-o>jb<Esc>",
            "ia<C-o>$b<Esc>",
            "ia<C-o>0b<Esc>",
            "ia<C-o><Esc>b<Esc>",
            "ia<C-o>xb<Esc>.",
            "ia<C-o>xb<Esc>u",
            "ia<C-o>xb<Esc>uu",
            "RX<C-o>lY<Esc>",
            "i<C-e><C-e><Esc>",
            "i<C-y><C-y><Esc>",
            "A<C-e><Esc>",
            "A<C-y>x<Esc>",
            "iab<C-c>",
            "3iab<C-c>",
            "ia<C-c>.",
            "cwab<C-c>w.",
        ] {
            b.some(buffer, 2, keys);
        }
    }
    b.finish("typing", "替换模式与插入模式的 Ctrl 键。")
}

/// `Ctrl-A` 与 `Ctrl-X`。
fn numbers(full: bool) -> Group {
    let mut b = Builder::new("n", full);
    for keys in [
        "<C-a>",
        "<C-x>",
        "5<C-a>",
        "10<C-x>",
        "<C-a>.",
        "<C-a>3.",
        "<C-a>u",
        "100<C-a>",
        "<C-x><C-x>",
        "<C-a><C-a><C-x>",
        "999999999<C-a>",
    ] {
        b.each(&NUMBERS, keys);
    }
    for buffer in [&WORDS, &CODE] {
        for keys in ["<C-a>", "<C-x>", "3<C-a>"] {
            b.some(buffer, 1, keys);
        }
    }
    b.finish("numbers", "数的增减。")
}

/// 按块的可视模式，以及可视模式下的 `I` 与 `A`。
fn blocks(full: bool) -> Group {
    let mut b = Builder::new("b", full);
    for keys in [
        "<C-v>jld",
        "<C-v>jly",
        "<C-v>jjlld",
        "<C-v>jjjly",
        "<C-v>jj$d",
        "<C-v>jj$y",
        "<C-v>j$hd",
        "<C-v>ld",
        "<C-v>d",
        "<C-v>y",
        "<C-v>3ly",
        "<C-v>kld",
        "<C-v>jhd",
        "<C-v>jlcX<Esc>",
        "<C-v>jjcXY<Esc>",
        "<C-v>jj$cX<Esc>",
        "<C-v>jlsX<Esc>",
        "<C-v>jjIX<Esc>",
        "<C-v>jjlIXY<Esc>",
        "<C-v>jjlAX<Esc>",
        "<C-v>jjllAXY<Esc>",
        "<C-v>jj$AX<Esc>",
        "<C-v>jjI<Esc>",
        "<C-v>jlrQ",
        "<C-v>jj$rQ",
        "<C-v>jl~",
        "<C-v>jlU",
        "<C-v>jlu",
        "<C-v>jlg?",
        "<C-v>jl>",
        "<C-v>jl2>",
        "<C-v>jjl<lt>",
        "<C-v>jl>.",
        "<C-v>jlJ",
        "<C-v>jlgJ",
        "<C-v>jlD",
        "<C-v>jlX",
        "<C-v>jlY",
        "<C-v>jlCQ<Esc>",
        "<C-v>jlRQ<Esc>",
        "<C-v>jlod",
        "<C-v>jlOd",
        "<C-v>jlOhd",
        "<C-v>jlokd",
        "<C-v>jl<Esc>x",
        "<C-v>jl<C-v>x",
        "<C-v>jlyP",
        "<C-v>jlyp",
        "<C-v>jly$p",
        "<C-v>jlyGp",
        "<C-v>jlyG$p",
        "<C-v>jly3p",
        "<C-v>jlygP",
        "<C-v>jlyjjp",
        "<C-v>jldp",
        "<C-v>jld.",
        "<C-v>jldj.",
        "<C-v>jlIX<Esc>j.",
        "<C-v>jlAX<Esc>j.",
        "<C-v>jlcX<Esc>j.",
        "<C-v>jlrQj.",
        "<C-v>jl~j.",
        "<C-v>jldu",
        "<C-v>jlIX<Esc>u",
        "<C-v>jlcX<Esc>u",
        "<C-v>jl<Esc>gvd",
        "<C-v>jl<Esc>ggjgvy",
        "<C-v>jlvd",
        "<C-v>jlVd",
        "vjl<C-v>d",
        "Vj<C-v>d",
        "<C-q>jld",
        "<C-v>jl\"ay\"ap",
        "<C-v>jl\"ad\"aP",
        "<C-v>jly\"0p",
        "<C-v>jleyp",
        "<C-v>jwd",
        "<C-v>j$A<Esc>x",
        "<C-v>jliwd",
        "<C-v>jl<Esc>gg`<lt>",
        "<C-v>jl<Esc>`>",
        "d<C-v>j",
        "d<C-v>2j",
        "d<C-v>l",
        "d<C-v>w",
        "y<C-v>jp",
        "c<C-v>jX<Esc>",
        "d<C-v>j.",
        "vlIX<Esc>",
        "vlAX<Esc>",
        "vjIX<Esc>",
        "vjAX<Esc>",
        "VjIX<Esc>",
        "VjAX<Esc>",
        "v$AX<Esc>",
        "vAX<Esc>",
        "v$hAX<Esc>",
        "vjhAX<Esc>",
        "vlIX<Esc>j.",
        "vlAX<Esc>j.",
        "vhAX<Esc>",
        "vkAX<Esc>",
        "vkIX<Esc>",
        "vkhIX<Esc>",
        "vkk$IX<Esc>",
        "vhIX<Esc>j.",
        "vAX<Esc>j.",
        "v$AX<Esc>j0.",
        "vjAX<Esc>gg.",
        "vjIX<Esc>gg.",
        "VjAX<Esc>j.",
    ] {
        b.each(&BLOCK, keys);
    }
    for buffer in [&WORDS, &CODE] {
        for keys in [
            "<C-v>jld",
            "<C-v>jly",
            "<C-v>jj$y",
            "<C-v>jlIX<Esc>",
            "<C-v>jlAX<Esc>",
            "<C-v>jlcX<Esc>",
            "<C-v>jlyp",
        ] {
            b.some(buffer, 3, keys);
        }
    }
    b.finish(
        "blocks",
        "按块的可视模式：操作符、插入、粘贴与点重复，以及可视模式下的 I 与 A。",
    )
}
