//! 命令行与宏的对照用例。

use super::{Buffer, Builder, CODE, Group, WORDS, typed};

/// 重复的词、数与缩进，用于替换、排序与逐行执行。
const TABLE: Buffer = Buffer {
    text: &[
        "b2 alpha beta",
        "a10 gamma  alpha",
        "",
        "c1 Alpha beta beta",
        "  a2 delta",
        "b2 alpha beta",
        "-3 x.y(z) 0x1f",
        "zeta",
    ],
    cursors: &[
        (0, 0),
        (0, 4),
        (1, 6),
        (2, 0),
        (3, 9),
        (4, 3),
        (6, 5),
        (7, 2),
    ],
};

/// 返回输入命令行 `text` 并回车的按键记法。
fn ex(text: &str) -> String {
    format!(":{}<CR>", typed(text))
}

/// 按行工作的命令与范围。
const LINES: &[&str] = &[
    "d", "2,3d", "$d", ".,+1d", "d 2", "d x 2", "2d x", "%d", ",$d", "2;+1d", "$-1,$d", "3,1d",
    "y", "1,$-1y", "y a 2", "2,3y b", "pu", "2pu", "0pu", "pu!", "$pu", "2,3m0", "m$", "m+1",
    "m-2", "1,2m3", "m.", "2,4m$", "2,3t.", "t0", "t$", "1,2co$", "j", "1,3j", "j!", "j 3", "2j",
    ">", "1,2>>", "<", "> 2", "2,3<", "3", "$", "+1", "-1", "0", "2,4", "/a/d", "?a?d", "/a/,/e/d",
    "/a/;+1d", "/zzz/d", "'x", "2ka", "ka|3|'ad", "2d|1d", "3mark b", "p", "noh",
];

/// 替换。
const SUBSTITUTES: &[&str] = &[
    "s/a/X/",
    "s/a/X/g",
    "%s/a/X/g",
    r"%s/\(a\)\(l\)/\2\1/g",
    "s/a/&&/g",
    r"s/\w\+/\u&/g",
    r"s/\w\+/\U&\E!/",
    r"%s/\<\(\w\)\(\w*\)/\u\1\L\2/g",
    r"%s/a\|e/[&]/g",
    "s/A/X/gi",
    "s/a/X/gI",
    "%s/$/;/",
    "%s/^/> /",
    r"%s/\n//",
    r"s/\n//g",
    r"1,2s/ /\r/g",
    r"s/ /\r\r/",
    "s/x*/-/g",
    "s/a*/-/g",
    "s/a/b/|s/b/c/",
    "s#a#X#",
    r"s/a/\//",
    r"s/a/\&\\/",
    "s/a/X/ 2",
    "s/a/X/g 3",
    "%s/a//gn",
    "s/zzz/X/e",
    "s/zzz/X/",
    "%s/alpha/~/",
    "%s/a/X/e|%s/X/~Y/",
    "%s/b\\zse/E/g",
    r"%s/\s\+$//",
    "%s/beta/&/g",
    r"%s/\v(\w+) (\w+)/\2 \1/",
    "2,4s/a/X/",
    "s/a\\tb/x/",
    "%s/e/\\t/",
    "%s/中/zh/",
];

/// 逐行执行与排序。
const GLOBALS: &[&str] = &[
    "g/a/d",
    "v/a/d",
    "g!/a/d",
    "g/a/s/e/E/g",
    "g/a/normal Ax",
    "g/^/m0",
    "g/a/t.",
    "g/a/>",
    "g/a/j",
    "1,3g/a/d",
    "g/zzz/d",
    "g/a/d x",
    "g/^$/d",
    "g/a/normal dd",
    "g/a/m$",
    "g/b/s/a/X/|s/e/Y/",
    "v/./normal ix",
    "g/a/",
    "normal dw",
    "%normal Ax",
    "2,3normal I- ",
    "normal ihello",
    "%normal dd",
    "normal 2x",
    "2,$normal .",
    "%normal 0f lD",
    "sort",
    "sort!",
    "sort n",
    "sort i",
    "sort u",
    "sort iu",
    "2,5sort",
    r"sort /\w\+ /",
    r"sort /\d\+/ r",
    "sort x",
    "sort! n",
];

/// 命令行：范围、按行工作的命令、替换、逐行执行、排序，以及与按键命令的配合。
pub(super) fn commands(full: bool) -> Group {
    let mut b = Builder::new("x", full);
    for buffer in [&TABLE, &CODE, &WORDS] {
        for text in LINES {
            b.some(buffer, 2, &ex(text));
        }
    }
    for buffer in [&TABLE, &WORDS] {
        for text in SUBSTITUTES {
            b.some(buffer, 2, &ex(text));
        }
        for text in GLOBALS {
            b.some(buffer, 2, &ex(text));
        }
    }
    let s = ex("s/a/X/");
    let sg = ex("s/a/X/g");
    for buffer in [&TABLE, &CODE] {
        for keys in [
            format!("{}\"xp", ex("d x 2")),
            format!("{}P", ex("1,2y")),
            format!("{}\"ap", ex("y a")),
            format!("yiw{}", ex("pu")),
            format!("yy{}", ex("$pu")),
            format!("\"qyiw{}", ex("pu q")),
            format!("Vj{}", typed("d\r")),
            format!("Vjj{}", typed(">\r")),
            format!("vj{}", typed("s/a/X/g\r")),
            format!("3{}", typed(":d\r")),
            format!("2{}", typed(":j\r")),
            format!("majj{}", ex("'a,.d")),
            format!("{}G'a", ex("2ka")),
            format!("{}gg`b", ex("3mark b")),
            format!("{s}j&"),
            format!("{sg}j&"),
            format!("{sg}j{}", ex("&&")),
            format!("{sg}ggg&"),
            format!("{s}j{}", ex("s")),
            format!("{s}j{}", ex("&g")),
            format!("{s}{}", ex("s/e/~Y/")),
            format!("/a<CR>{}", ex("s//X/")),
            format!("{sg}u"),
            format!("{sg}u<C-r>"),
            format!("{sg}n"),
            format!("{sg}ggn"),
            format!("{}u", ex("2,3d")),
            format!("{}u", ex("g/a/d")),
            format!("{}u", ex("%normal Ax")),
            format!("{}u<C-r>", ex("g/a/normal Ax")),
            format!("x{}", ex("u")),
            format!("xu{}", ex("red")),
            format!("{s}j@:"),
            format!("{}@:@@", ex("d")),
            format!("{}2@:", ex("d")),
            format!("{}.", ex("d")),
            format!("x{}", ex("2,$normal .")),
            format!("{}``", ex("3")),
            format!("{}''", ex("$")),
            format!("{}p", ex("2,3d")),
            format!("{}\"1p\"2p", ex("g/a/d")),
            format!("{}j{}", ex("s/b/X/"), ex("&&")),
            format!("dd{}", ex("g/a/pu")),
            "ga".to_owned(),
            format!("{}ga", ex("2")),
        ] {
            b.some(buffer, 2, &keys);
        }
    }
    b.finish(
        "commands",
        "命令行：范围、按行工作的命令、替换、逐行执行与排序。",
    )
}

/// 宏：记录、回放、次数，以及寄存器中的文本作为按键执行。
pub(super) fn macros(full: bool) -> Group {
    let mut b = Builder::new("q", full);
    for buffer in [&TABLE, &WORDS, &CODE] {
        for keys in [
            "qaxjq@a",
            "qaxjq2@a",
            "qaxjq@a@@",
            "qaA!<Esc>jq@a@@",
            "qadwq3@a",
            "qaiab<Esc>q@a",
            "qqxq@q",
            "qaxqqAlq@a",
            "qaxq\"ap",
            "qaIx<Esc>jq2@a",
            "qa~q@a@@",
            "qaq",
            "qaxq2@a.",
            "qaxlq@au",
            "qaxlq3@au",
            "qaA<CR>x<Esc>q@a",
            "qa0d$q@a",
            "qayiwPq@a",
            "qarXlq5@a",
            "qa>>jq@a",
            "qa<C-a>q@a",
            "qaix<BS>y<Esc>q@a",
            "qaA<Tab>x<Esc>jq@a",
            "qbvlyq@b\"0p",
            "qaqqbq@a@b",
            "\"ayl@a",
            "\"ayy@a",
            "@a",
            "@@",
            "@:",
            "qa2q",
            "qaxq@Ax",
        ] {
            b.some(buffer, 2, keys);
        }
        for keys in [
            format!("qa{}jq@a", ex("s/a/X/")),
            "qa/a<CR>xq@a".to_owned(),
            format!("qa{}q@a", ex("d")),
            format!("qa{}q2@a", ex("normal Ax")),
            format!("qax{}q@a", ex("2")),
            "qa/zzz<CR>xq@a".to_owned(),
            format!("{}@a", ex("normal qaxq")),
            format!("qaxq{}", ex("%normal @a")),
            format!("qaxq{}", ex("g/a/normal @a")),
        ] {
            b.some(buffer, 2, &keys);
        }
    }
    b.finish("macros", "宏的记录与回放：结果文本、光标与寄存器。")
}
