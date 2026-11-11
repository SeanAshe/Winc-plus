//! 文本对象、查找、标记与滚动命令的对照用例。

use super::{Buffer, Builder, CODE, Group, PROSE, TINY, WORDS, typed};

/// 引号与转义。
const QUOTES: Buffer = Buffer {
    text: &[
        "say \"hello world\" and 'x' or `cmd arg` end",
        "a \"one \\\" two\" b \"\" c 'it''s' d",
        "  'unclosed and \"two\"  \"three\"",
        "",
        "plain line",
    ],
    cursors: &[
        (0, 0),
        (0, 4),
        (0, 7),
        (0, 16),
        (0, 18),
        (0, 23),
        (0, 30),
        (0, 41),
        (1, 2),
        (1, 9),
        (1, 18),
        (1, 28),
        (2, 3),
        (2, 20),
        (4, 2),
    ],
};

/// 标签，包括嵌套、属性、自闭合与跨行。
const TAGS: Buffer = Buffer {
    text: &[
        "<div class=\"a\">",
        "  <p>one <b>two</b> three</p>",
        "  <br/>",
        "  <span>x</span><em></em>",
        "</div>",
        "tail <a href='x'>link</a> end <i>",
        "<ul>",
        "item",
        "</ul>",
    ],
    cursors: &[
        (0, 0),
        (0, 6),
        (1, 0),
        (1, 3),
        (1, 6),
        (1, 12),
        (1, 15),
        (1, 22),
        (2, 3),
        (3, 8),
        (3, 18),
        (4, 2),
        (5, 1),
        (5, 18),
        (5, 30),
        (7, 1),
    ],
};

/// 超过一屏的文本，用于滚动。
const LONG: Buffer = Buffer {
    text: &[
        "  line01 x",
        "  line02 x",
        "  line03 x",
        "  line04 x",
        "  line05 x",
        "  line06 x",
        "  line07 x",
        "  line08 x",
        "  line09 x",
        "  line10 x",
        "  line11 x",
        "  line12 x",
        "  line13 x",
        "  line14 x",
        "  line15 x",
        "  line16 x",
        "  line17 x",
        "  line18 x",
        "  line19 x",
        "  line20 x",
        "  line21 x",
        "  line22 x",
        "  line23 x",
        "  line24 x",
        "  line25 x",
        "  line26 x",
        "  line27 x",
        "  line28 x",
        "  line29 x",
        "  line30 x",
        "  line31 x",
        "  line32 x",
        "  line33 x",
        "  line34 x",
        "  line35 x",
        "  line36 x",
        "  line37 x",
        "  line38 x",
        "  line39 x",
        "  line40 x",
        "  line41 x",
        "  line42 x",
        "  line43 x",
        "  line44 x",
        "  line45 x",
        "  line46 x",
        "  line47 x",
        "  line48 x",
        "  line49 x",
        "  line50 x",
        "  line51 x",
        "  line52 x",
        "  line53 x",
        "  line54 x",
        "  line55 x",
        "  line56 x",
        "  line57 x",
        "  line58 x",
        "  line59 x",
        "  line60 x",
    ],
    cursors: &[(0, 0), (4, 3), (29, 0), (40, 9), (57, 5), (59, 0)],
};

/// 查找用例使用的模式。
const PATTERNS: &[&str] = &[
    "a",
    "one",
    "x",
    r"\<a",
    r"e\>",
    r"\<\w\+\>",
    r"o\+",
    "[a-c]",
    "[^a-z ]",
    "[]x]",
    r"\d",
    r"\s\+",
    r"\S\+$",
    r"\w\+",
    r"\a\d",
    r"\u\l\+",
    r"^\s*",
    "^",
    "$",
    ".",
    "a.b",
    r"\.",
    "a*",
    "*",
    r"\(a\|e\)r",
    r"\vo+",
    r"\v(a|e)r",
    r"\vx|y",
    r"\v<t\w*>",
    r"\Vfn main()",
    r"\V.",
    r"\Ma.b",
    r"\cFIRST",
    r"\CFirst",
    r"e\{2}",
    r"o\{-1,}",
    r"l\{1,2}",
    r"x\=)",
    r"n\zse",
    r"\w\ze)",
    r"(\@<=x",
    r"x)\@=",
    r"\(foo\)\@!\<\w",
    r"\n\n",
    r"e\n",
    r"\_s\+\w",
    "(x)",
    "[[:digit:]]",
    r"[[:upper:]]\w",
    r"\%(ab\)\=c",
    "中",
    r"\t",
    r"a\|b\|c",
    r"\<\(\w\)\w*\1",
    "zzz",
];

/// 文本对象：等待动作时作为范围，可视模式下选中与延伸。
pub(super) fn objects(full: bool) -> Group {
    let mut b = Builder::new("t", full);
    let mut add = |buffers: &[&Buffer], objects: &[&str]| {
        for buffer in buffers {
            for object in objects {
                let object = typed(object);
                b.each(buffer, &format!("d{object}"));
                b.some(buffer, 2, &format!("y{object}"));
                b.some(buffer, 3, &format!("c{object}XY<Esc>"));
                b.some(buffer, 2, &format!("v{object}d"));
                b.some(buffer, 3, &format!("d2{object}"));
                b.some(buffer, 3, &format!("v{object}{object}y"));
                b.some(buffer, 4, &format!("vl{object}y"));
                b.some(buffer, 4, &format!("vlo{object}y"));
                b.some(buffer, 4, &format!("V{object}d"));
                b.some(buffer, 5, &format!("v3{object}d"));
                b.some(buffer, 5, &format!("d{object}."));
            }
        }
    };
    add(
        &[&WORDS, &CODE, &PROSE],
        &["iw", "aw", "iW", "aW", "is", "as", "ip", "ap"],
    );
    add(&[&CODE, &QUOTES], &["i(", "a(", "i[", "a[", "i{", "a{"]);
    add(&[&CODE, &QUOTES], &["i\"", "a\"", "i'", "a'", "i`", "a`"]);
    add(&[&TAGS], &["it", "at", "i<", "a<"]);
    // 同一个对象的其他写法。
    for buffer in [&CODE, &QUOTES, &TAGS] {
        for object in [
            "i)", "a)", "ib", "ab", "i]", "a]", "i}", "a}", "iB", "aB", "i>", "a>",
        ] {
            let object = typed(object);
            b.some(buffer, 2, &format!("d{object}"));
            b.some(buffer, 4, &format!("v{object}y"));
        }
    }
    for buffer in &TINY {
        for object in ["iw", "aw", "ip", "ap", "is", "as", "i(", "i\"", "it"] {
            let object = typed(object);
            b.each(buffer, &format!("d{object}"));
            b.each(buffer, &format!("v{object}y"));
        }
    }
    for buffer in [&WORDS, &CODE, &PROSE] {
        for keys in [
            "vipipd",
            "vapapd",
            "Vipd",
            "Vapd",
            "Vjipd",
            "vjipd",
            "Vkipd",
            "vkapy",
            "vjapapy",
            "vawawd",
            "vawawawy",
            "vhiwiwy",
            "vhawy",
            "vbawawy",
            "visisy",
            "vasasy",
            "vhisy",
            "vhasy",
            "v2awd",
            "v2iwd",
            "y2ap",
            "d3aw",
            "d3iw",
            "d2as",
            "d2is",
            "ciwQ<Esc>w.",
            "dawdaw",
            "yawP",
            "gUiw",
            "g~aw",
            ">ip",
            "<lt>ap",
            "viwp",
            "yiwwviwp",
            "viwo<Esc>x",
        ] {
            b.some(buffer, 2, keys);
        }
    }
    b.finish(
        "objects",
        "文本对象：作为操作符的范围，以及在可视模式下选中与延伸。",
    )
}

/// 查找：模式语法、偏移、`n` `N`、光标处的词、匹配的文本对象。
pub(super) fn searches(full: bool) -> Group {
    let mut b = Builder::new("s", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for (index, pattern) in PATTERNS.iter().enumerate() {
            let pattern = typed(pattern);
            b.some(buffer, 2, &format!("/{pattern}<CR>"));
            b.some(buffer, 4, &format!("?{pattern}<CR>"));
            b.some(buffer, 4, &format!("/{pattern}<CR>n"));
            b.some(buffer, 4, &format!("d/{pattern}<CR>"));
            // 其余的组合只用前面几个模式。
            if index >= 12 {
                continue;
            }
            b.some(buffer, 4, &format!("/{pattern}<CR>N"));
            b.some(buffer, 4, &format!("?{pattern}<CR>nn"));
            b.some(buffer, 4, &format!("2/{pattern}<CR>"));
            b.some(buffer, 5, &format!("y?{pattern}<CR>"));
            b.some(buffer, 5, &format!("v/{pattern}<CR>d"));
            b.some(buffer, 5, &format!("/{pattern}<CR>dgn"));
            b.some(buffer, 5, &format!("/{pattern}<CR>gnd"));
            b.some(buffer, 5, &format!("/{pattern}<CR>ggdgn"));
            b.some(buffer, 5, &format!("/{pattern}<CR>gNy"));
        }
        for keys in [
            "/a/e<CR>",
            "/one/e<CR>",
            "/one/e+1<CR>",
            "/one/e-1<CR>",
            "/one/s+1<CR>",
            "/one/s-1<CR>",
            "/one/b+2<CR>",
            "/a/+1<CR>",
            "/a/-1<CR>",
            "/a/1<CR>",
            "?a?e<CR>",
            "?a?+1<CR>",
            "/a/e<CR>n",
            "/a/e<CR>N",
            "/a/e+1<CR>n",
            "/a/s-1<CR>n",
            "/a/+1<CR>n",
            "d/a/e<CR>",
            "d/a/+1<CR>",
            "y/a/e<CR>",
            "/a<CR>/<CR>",
            "/a<CR>//e<CR>",
            "/a/e<CR>/<CR>",
            "/a<CR>?<CR>",
            "/a<Esc>n",
            "/a<CR>3n",
            "/a<CR>99n",
            "/a<CR>dn",
            "/a<CR>dN",
            "/a<CR>cnQ<Esc>",
            "/a<CR>yn",
            "/a<CR>d2n",
            "n",
            "N",
            "/<CR>",
            "dgn",
            "*",
            "#",
            "g*",
            "g#",
            "2*",
            "2#",
            "*n",
            "*N",
            "#n",
            "#N",
            "**",
            "*#",
            "g*n",
            "d*",
            "y#",
            "*dgn",
            "*cgnQ<Esc>",
            "*cgnQ<Esc>.",
            "*cgnQ<Esc>..",
            "*dgn.",
            "*gnd",
            "*gNd",
            "*vgny",
            "*vgngny",
            "*2dgn",
            "*ggdgn",
            "*GdgN",
            "*Ncgn<Esc>",
            "/a<CR>cgnXY<Esc>n.",
            r"/a\/b<CR>",
            r"?a\?b<CR>",
            r"/\/<CR>",
            "/a<BS>e<CR>",
        ] {
            b.some(buffer, 2, keys);
        }
    }
    b.finish(
        "searches",
        "查找：模式语法的转换、偏移、重复、光标处的词与匹配的文本对象。",
    )
}

/// 标记、跳转历史与修改列表。
pub(super) fn marks(full: bool) -> Group {
    let mut b = Builder::new("k", full);
    for buffer in [&WORDS, &CODE, &PROSE] {
        for keys in [
            "majj`a",
            "majj'a",
            "ma3l`a",
            "ma$`a",
            "maG`a",
            "mbgg'b",
            "maGd'a",
            "majjd`a",
            "mawwy`a",
            "maggy'a",
            "mAG'A",
            "mAgg`A",
            "'a",
            "`z",
            "d'a",
            "G``",
            "G''",
            "Ggg``",
            "Ggg''",
            "G````",
            "G''''",
            "jjG``kk``",
            "G``d``",
            "Gy''",
            "}``",
            ")''",
            "%``",
            "L``",
            "3G5G``",
            "jj``",
            "m'G``",
            "m`G''",
            "maGdd`a",
            "maggdd'a",
            "majdd`a",
            "maggOx<Esc>G`a",
            "yw$`[",
            "yw0`]",
            "y$0`]",
            "yj`]",
            "yyG`[",
            "yjG']",
            "ywP$`[",
            "ywP0`]",
            "yyp`[",
            "yyP`]",
            "vjy`<lt>",
            "vjy`>",
            "vj<Esc>gg'<lt>",
            "vj<Esc>gg'>",
            "Vjy`>",
            "Vj<Esc>`<lt>",
            "vlohy`<lt>",
            "m<lt>jjm>gv",
            "iab<Esc>gg`^",
            "Aab<Esc>gg`^",
            "ia<Esc>G`.",
            "xG`.",
            "xjxG`.",
            "ddG'.",
            "rQG`.",
            "iab<Esc>gg`[",
            "iab<Esc>gg`]",
            "xjg;",
            "xjxg;",
            "xjxg;g;",
            "xjxg;g;g,",
            "xjxG2g;",
            "xjxGg;g,",
            "g;",
            "g,",
            "xg,",
            "ia<Esc>Gg;",
            "xGxggg;g;",
            "G<C-o>",
            "Ggg<C-o>",
            "Ggg<C-o><C-o>",
            "G<C-o><Tab>",
            "Ggg<C-o><C-o><Tab>",
            "}}<C-o>",
            "G<C-o><C-o>",
            "3G5G<C-o>",
            "3G5G<C-o><C-o>",
            "3G5G2<C-o>",
            "3G5G<C-o><C-o><Tab><Tab>",
            "G<C-o>``",
            "<C-o>",
            "<Tab>",
            "*<C-o>",
            "Gx<C-o>",
        ] {
            b.some(buffer, 2, keys);
        }
    }
    b.finish("marks", "标记、跳转历史与修改列表。")
}

/// 滚动与折叠命令。
pub(super) fn views(full: bool) -> Group {
    let mut b = Builder::new("w", full);
    for keys in [
        "<C-d>",
        "<C-u>",
        "2<C-d>",
        "2<C-u>",
        "<C-d><C-d>",
        "<C-d><C-u>",
        "<C-u><C-d>",
        "5<C-d><C-d>",
        "30<C-d>",
        "<C-f>",
        "2<C-f>",
        "<C-f><C-f>",
        "<PageDown>",
        "<S-Down>",
        "<C-e>",
        "3<C-e>",
        "<C-y>",
        "2<C-y>",
        "zz",
        "zt",
        "zb",
        "z.",
        "z<CR>",
        "z-",
        "5zt",
        "5z.",
        "70z<CR>",
        "zh",
        "zl",
        "z<Left>",
        "z<Right>",
        "zH",
        "zL",
        "zs",
        "ze",
        "zo",
        "zc",
        "za",
        "zO",
        "zC",
        "zA",
        "zR",
        "zM",
        "zv",
        "zj",
        "zk",
        "v<C-d>y",
        "V<C-f>y",
        "$<C-d>",
        "$<C-e>",
    ] {
        b.each(&LONG, keys);
    }
    // 第一行上向上翻页没有可滚动的内容；其余位置上无界面执行的结果不反映窗口的滚动。
    for keys in ["<C-b>", "<PageUp>", "<S-Up>"] {
        b.add(LONG.text, (0, 0), keys);
    }
    b.finish("views", "滚动与折叠命令：核对光标的位置。")
}
