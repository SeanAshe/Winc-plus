//! 命令行、宏与映射的手写测试：录制覆盖不到的行为（失败时中止、上限、逐处确认、交给宿主
//! 的命令、选项、映射的等待与超时）。

mod common;

use std::time::Duration;

use winc_core::Lines;
use winc_vim::Mode;
use winc_vim::ex::{Completing, complete};
use winc_vim::{AppCommand, Focusing, Host, HostOption, Key, Listing, Mapped, Mapping, Message};
use winc_vim::{Prompt, Shared, Vim, notation};

use common::{TestHost, feed};

/// 一次测试的状态：宿主、窗口的状态与共用的状态。
struct Session {
    host: TestHost,
    vim: Vim,
    shared: Shared,
}

impl Session {
    /// 创建文本为 `text`（末尾另加换行符）、光标在开头的状态。
    fn new(text: &str) -> Self {
        Self {
            host: TestHost::new(&format!("{text}\n")),
            vim: Vim::default(),
            shared: Shared::default(),
        }
    }

    /// 输入按键记法 `keys`。
    fn keys(&mut self, keys: &str) -> &mut Self {
        feed(&mut self.vim, &mut self.shared, &mut self.host, keys);
        self
    }

    /// 输入命令行 `line` 并回车。
    fn ex(&mut self, line: &str) -> &mut Self {
        self.keys(":");
        self.vim
            .prompt_submit(&mut self.shared, &mut self.host, line);
        self
    }

    /// 返回去掉末尾换行符的文本。
    fn text(&self) -> String {
        let text = self.host.text();
        text.strip_suffix('\n').unwrap_or(&text).to_owned()
    }

    /// 返回光标的行号与列号。
    fn caret(&self) -> (usize, usize) {
        self.host.rope.pos_to_line_col(self.vim.caret(&self.host))
    }

    /// 取出宿主收到的提示。
    fn messages(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.host.messages)
    }

    /// 返回寄存器 `name` 的文本。
    fn register(&self, name: char) -> String {
        let content = self.shared.registers.get(Some(name));
        content
            .map(|content| content.text.clone())
            .unwrap_or_default()
    }
}

#[test]
fn the_command_line_is_prefilled_from_the_selection_and_the_count() {
    let mut s = Session::new("one\ntwo\nthree\nfour");
    s.keys(":");
    assert_eq!(
        (s.vim.prompt(), s.vim.prompt_prefill()),
        (Some(Prompt::Command), "")
    );
    s.keys("<Esc>");
    assert_eq!(s.vim.prompt(), None);

    s.keys("3:");
    assert_eq!(s.vim.prompt_prefill(), ".,.+2");
    s.keys("d<CR>");
    assert_eq!((s.text().as_str(), s.caret()), ("four", (0, 0)));
    assert_eq!(s.messages(), [Message::LineDelta(-3)]);

    // 可视模式下输入命令行即离开可视模式，范围是选区的首末行。
    s.keys("uggVj:");
    assert_eq!(
        (s.vim.mode(), s.vim.prompt_prefill()),
        (Mode::Normal, "'<,'>")
    );
    s.keys("><CR>");
    assert_eq!(s.text(), "\tone\n\ttwo\nthree\nfour");
    assert_eq!(s.shared.command_history(), [".,.+2d", "'<,'>>"]);
    assert_eq!(s.register(':'), "'<,'>>");
}

#[test]
fn errors_are_reported_and_leave_the_text_alone() {
    let mut s = Session::new("one\ntwo\nthree");
    for (line, message) in [
        ("zzz", Message::BadCommand),
        ("9d", Message::BadRange),
        ("3,1d", Message::BadRange),
        ("'q", Message::NoMark('q')),
        ("d 0", Message::BadArgument),
        ("&&", Message::NoSubstitute),
        ("s/zzz/x/", Message::NotFound),
        ("g/zzz/d", Message::NotFound),
        ("set nosuch", Message::UnknownOption("nosuch".into())),
        ("m 2,3 x", Message::BadArgument),
    ] {
        s.ex(line);
        assert_eq!(s.messages(), [message], "{line}");
        assert_eq!(s.text(), "one\ntwo\nthree", "{line}");
    }
    // 两个地址相同的合并不执行，最后一行上的合并失败；光标都移到该行。
    s.keys("gg").ex("2,2j");
    assert_eq!((s.caret(), s.messages()), ((1, 0), vec![]));
    s.keys("gg").ex("$j");
    assert_eq!((s.text().as_str(), s.caret()), ("one\ntwo\nthree", (2, 0)));

    s.host.read_only = true;
    s.ex("d");
    assert_eq!(s.messages(), [Message::ReadOnly]);
    s.ex("1,2y");
    assert_eq!(
        (s.register('0').as_str(), s.messages()),
        ("one\ntwo", vec![])
    );
}

#[test]
fn a_failing_command_aborts_the_macro() {
    let mut s = Session::new("one\ntwo\nthree\nfour");
    // 在末尾执行自身的宏在某条命令失败时结束：最后一行上的 `j` 失败。
    s.keys("qaqqaxj@aqgg@a");
    assert_eq!(s.text(), "e\nwo\nhree\nour");
    assert_eq!(s.register('a'), "xj@a");
    // 整个回放是一个撤销单元。
    s.keys("u");
    assert_eq!(s.text(), "ne\ntwo\nthree\nfour");

    // 次数没有用完也中止；找不到匹配同样中止。
    let mut s = Session::new("one\ntwo\nthree");
    s.keys("qajxq9@a");
    assert_eq!(s.text(), "one\nwo\nhree");
    // 键入时失败的命令不影响之后的按键，回放时中止。
    s.keys("ggqb/zzz<CR>xq");
    assert_eq!(s.text(), "ne\nwo\nhree");
    s.host.messages.clear();
    s.keys("@b");
    assert_eq!(
        (s.text().as_str(), s.messages()),
        ("ne\nwo\nhree", vec![Message::NotFound])
    );
    // 没有定义的按键也中止。
    let mut s = Session::new("éx rest");
    s.keys("\"cy2l@c");
    assert_eq!(
        (s.register('c').as_str(), s.text().as_str()),
        ("éx", "éx rest")
    );
}

#[test]
fn recording_keeps_typed_keys_and_shows_in_the_state() {
    let mut s = Session::new("one two\nthree");
    assert_eq!(s.vim.recording(), None);
    s.keys("qa");
    assert_eq!(s.vim.recording(), Some('a'));
    // 没有字符的按键写成按键记法，其余的按键是字符本身。
    s.keys("A<Left>x<C-w>y<Esc>:");
    s.vim.prompt_submit(&mut s.shared, &mut s.host, "s/e/E/");
    s.keys("q");
    assert_eq!(s.vim.recording(), None);
    assert_eq!(s.register('a'), "A<Left>x\u{17}y\u{1b}:s/e/E/\r");
    assert_eq!(s.text(), "onE y\nthree");
    s.keys("j@a");
    assert_eq!(s.text(), "onE y\ny");
    // 大写的寄存器名追加；键入的 `<` 之后恰好构成按键记法时写成 `<lt>`，回放时仍是字符。
    s.keys("qAo<lt>Up><lt>b<Esc>q");
    assert!(s.register('a').ends_with("o<lt>Up><b\u{1b}"));
    let mut s = Session::new("one");
    s.keys("qao<lt>Up><lt>b<Esc>q@a");
    assert_eq!(s.register('a'), "o<lt>Up><b\u{1b}");
    assert_eq!(s.text(), "one\n<Up><b\n<Up><b");
    s.keys("qq").keys("<Esc>q");
    assert_eq!(s.register('q'), "\u{1b}");
}

#[test]
fn playback_stops_at_the_limits() {
    let mut s = Session::new("@a");
    s.vim.limit_playback(10_000, Duration::from_secs(60));
    // 寄存器的内容是执行自身：只有上限能结束回放。
    s.keys("\"ay$@a");
    assert_eq!(s.messages(), [Message::Interrupted]);
    assert_eq!(s.vim.mode(), Mode::Normal);
    // 之后的按键不受影响。
    s.keys("x");
    assert_eq!(s.text(), "a");

    // 时间的上限；已完成的部分是一个撤销单元。
    let mut s = Session::new("one\ntwo");
    s.vim.limit_playback(usize::MAX, Duration::ZERO);
    s.keys("qaA!<Esc>q");
    s.host.messages.clear();
    s.keys("1000@a");
    assert_eq!(s.messages(), [Message::Interrupted]);
    let done = s.text();
    assert!(done.starts_with("one!!") && done.len() < 1000, "{done}");
    s.keys("u");
    assert_eq!(s.text(), "one!\ntwo");

    // 次数使队列超过上限时不执行。
    let mut s = Session::new("one");
    s.keys("qaxq").keys("ix<Esc>");
    s.host.messages.clear();
    s.keys("999999999@a");
    assert_eq!(
        (s.text().as_str(), s.messages()),
        ("xne", vec![Message::TooLarge])
    );

    // 逐行执行的命令同样受限。
    let mut s = Session::new("a\na\na\na\na\na\na\na");
    s.vim.limit_playback(7, Duration::from_secs(60));
    s.ex("g/a/normal Ax");
    assert_eq!(s.messages(), [Message::Interrupted]);
    assert!(s.text().starts_with("ax\nax\n") && s.text().ends_with("\na"));
    s.keys("u");
    assert_eq!(s.text(), "a\na\na\na\na\na\na\na");
}

#[test]
fn substitutions_can_be_confirmed_one_by_one() {
    let mut s = Session::new("a b a\na a\nb a");
    s.ex("%s/a/X/gc");
    assert_eq!(s.vim.confirming(), Some("X"));
    // 当前询问的匹配显示为选区；撤销分组到确认结束为止。
    assert_eq!((s.host.selection.anchor, s.host.selection.head), (0, 1));
    assert_eq!(s.host.groups, 1);
    s.keys("y");
    assert_eq!(
        (s.text().as_str(), s.host.selection.anchor),
        ("X b a\na a\nb a", 4)
    );
    // 没有定义的按键不结束确认。
    s.keys("nzy");
    assert_eq!(s.text(), "X b a\nX a\nb a");
    s.keys("q");
    assert_eq!((s.vim.confirming(), s.host.groups), (None, 0));
    assert_eq!((s.vim.mode(), s.caret()), (Mode::Normal, (1, 2)));
    s.keys("u");
    assert_eq!(s.text(), "a b a\na a\nb a");

    // `a` 替换余下的全部，`l` 替换一处后结束，`Esc` 结束。
    s.ex("%s/a/XX/gc").keys("na");
    assert_eq!(
        (s.text().as_str(), s.vim.confirming()),
        ("a b XX\nXX XX\nb XX", None)
    );
    assert_eq!(s.messages(), [Message::Substituted { count: 4, lines: 0 }]);
    s.keys("u").ex("%s/a//gc").keys("l");
    assert_eq!(s.text(), " b a\na a\nb a");
    s.keys("u").ex("%s/a/-/c").keys("<Esc>");
    assert_eq!(
        (s.text().as_str(), s.vim.confirming()),
        ("a b a\na a\nb a", None)
    );

    // 选区被其他操作改动时确认结束。
    s.ex("%s/a/-/gc").keys("y");
    s.host.set_selection(winc_core::Selection::point(3));
    s.vim.sync(&mut s.host);
    assert_eq!((s.vim.confirming(), s.host.groups), (None, 0));
    assert_eq!(s.text(), "- b a\na a\nb a");
}

#[test]
fn file_and_tab_commands_go_to_the_host() {
    let mut s = Session::new("see src/main.rs here");
    let quit = |write, force| AppCommand::Quit { write, force };
    for (line, command) in [
        (
            "w",
            AppCommand::Write {
                path: None,
                force: false,
            },
        ),
        (
            "w out.txt",
            AppCommand::Write {
                path: Some("out.txt".into()),
                force: false,
            },
        ),
        (
            "w! out.txt",
            AppCommand::Write {
                path: Some("out.txt".into()),
                force: true,
            },
        ),
        ("wa", AppCommand::WriteAll),
        ("q", quit(false, false)),
        ("q!", quit(false, true)),
        ("wq", quit(true, false)),
        ("x", quit(true, false)),
        ("bd", quit(false, false)),
        ("tabc", quit(false, false)),
        (
            "qa!",
            AppCommand::QuitAll {
                write: false,
                force: true,
            },
        ),
        (
            "wqa",
            AppCommand::QuitAll {
                write: true,
                force: false,
            },
        ),
        (
            "e",
            AppCommand::Edit {
                path: None,
                force: false,
            },
        ),
        (
            "e! a b.txt",
            AppCommand::Edit {
                path: Some("a b.txt".into()),
                force: true,
            },
        ),
        ("bn", AppCommand::BufferStep(1)),
        ("2bn", AppCommand::BufferStep(2)),
        ("bp", AppCommand::BufferStep(-1)),
        ("b 3", AppCommand::BufferNumber(3)),
        ("b main", AppCommand::BufferName("main".into())),
        ("tabn", AppCommand::BufferStep(1)),
        ("tabn 2", AppCommand::BufferNumber(2)),
        ("tabp", AppCommand::BufferStep(-1)),
        ("tabo", AppCommand::Only),
        ("tabnew", AppCommand::New { path: None }),
        (
            "tabe x.rs",
            AppCommand::New {
                path: Some("x.rs".into()),
            },
        ),
        ("noh", AppCommand::NoHighlight),
        ("reg", AppCommand::List(Listing::Registers)),
        ("marks", AppCommand::List(Listing::Marks)),
        ("ls", AppCommand::List(Listing::Buffers)),
        ("h", AppCommand::Help),
    ] {
        s.ex(line);
        assert_eq!(s.host.apps.pop(), Some(command), "{line}");
    }
    for (keys, command) in [
        ("gd", AppCommand::Definition),
        ("gf", AppCommand::OpenFile),
        ("gt", AppCommand::BufferStep(1)),
        ("3gt", AppCommand::BufferNumber(3)),
        ("2gT", AppCommand::BufferStep(-2)),
        ("ZZ", quit(true, false)),
        ("ZQ", quit(false, true)),
        ("<C-^>", AppCommand::Alternate),
        ("<C-w>h", AppCommand::Focus(Focusing::Left)),
        ("<C-w><C-l>", AppCommand::Focus(Focusing::Right)),
        ("<C-w>j", AppCommand::Focus(Focusing::Down)),
        ("<C-w>k", AppCommand::Focus(Focusing::Up)),
        ("<C-w>w", AppCommand::Focus(Focusing::Next)),
        // 不在当前文档中的大写标记交给宿主。
        (
            "`Q",
            AppCommand::GlobalMark {
                name: 'Q',
                line: false,
            },
        ),
        (
            "'Q",
            AppCommand::GlobalMark {
                name: 'Q',
                line: true,
            },
        ),
    ] {
        s.keys(keys);
        assert_eq!(s.host.apps.pop(), Some(command), "{keys}");
    }
    assert_eq!(
        (s.messages(), s.text().as_str()),
        (vec![], "see src/main.rs here")
    );
    s.keys("ga");
    assert_eq!(s.messages(), [Message::Char('s')]);
}

#[test]
fn options_are_set_shown_and_toggled() {
    let mut s = Session::new("One one");
    s.ex("set ic nowrapscan so=3 ts=4 nu norelativenumber");
    assert!(s.shared.ignore_case && !s.shared.options.wrapscan);
    assert_eq!(s.shared.options.scrolloff, Some(3));
    assert_eq!(s.host.option(HostOption::TabStop), Some(4));
    assert_eq!(s.host.option(HostOption::Number), Some(1));
    assert_eq!(s.messages(), []);

    s.ex("set ic? so? ts tabstop? nowrapscan? clipboard?");
    let shown = |text: &str| Message::Option(text.into());
    assert_eq!(
        s.messages(),
        [
            shown("ignorecase"),
            shown("scrolloff=3"),
            shown("tabstop=4"),
            shown("tabstop=4"),
            shown("nowrapscan"),
            shown("clipboard="),
        ]
    );
    s.ex("set invic gdefault! cb=unnamed tm=300");
    assert!(!s.shared.ignore_case && s.shared.options.gdefault);
    assert_eq!(s.shared.clipboard, winc_vim::Clipboard::Always);
    assert_eq!(s.shared.options.timeoutlen, Some(300));
    // `gdefault` 之下 `g` 标志的含义相反。
    s.ex("s/n/N/");
    assert_eq!(s.text(), "ONe oNe");
    s.ex("s/N/n/g");
    assert_eq!(s.text(), "One oNe");
    s.ex("set ic=1");
    assert_eq!(s.messages(), [Message::BadArgument]);

    // 关闭 `wrapscan` 之后查找不回到文本的另一端。
    let mut s = Session::new("one\ntwo\none");
    s.ex("set nows").keys("G/two<CR>");
    assert_eq!((s.caret(), s.messages()), ((2, 0), vec![Message::NotFound]));
}

fn mapping(mode: Mode, keys: &str, to: &str) -> Mapping {
    Mapping {
        mode,
        keys: notation::parse(keys),
        to: Mapped::Keys(notation::parse(to)),
    }
}

#[test]
fn mappings_wait_for_more_keys_and_do_not_recurse() {
    let mut s = Session::new("one two\nthree");
    s.shared.set_mappings(vec![
        mapping(Mode::Insert, "jk", "<Esc>"),
        mapping(Mode::Normal, "Y", "y$"),
        mapping(Mode::Normal, "x", "dd"),
        mapping(Mode::Normal, "dd", "x"),
        mapping(Mode::Normal, ",w", "dw"),
        mapping(Mode::Normal, ",", "j"),
        mapping(Mode::OperatorPending, "L", "$"),
        mapping(Mode::Visual, "Q", "d"),
        Mapping {
            mode: Mode::Normal,
            keys: notation::parse("gs"),
            to: Mapped::Command("Save".into()),
        },
    ]);
    // 插入模式下等待中的字符先显示，匹配之后撤回。
    s.keys("ij");
    assert!(s.vim.awaits_mapping());
    assert_eq!(
        (s.text().as_str(), s.vim.mode()),
        ("jone two\nthree", Mode::Insert)
    );
    s.keys("k");
    assert!(!s.vim.awaits_mapping());
    assert_eq!(
        (s.text().as_str(), s.vim.mode()),
        ("one two\nthree", Mode::Normal)
    );
    // 没有匹配时已显示的字符留下，后面的按键按原样处理。
    s.keys("ijx<Esc>");
    assert_eq!(s.text(), "jxone two\nthree");
    // 等待超时：已显示的字符留下。
    s.keys("uij");
    s.vim.timeout(&mut s.shared, &mut s.host);
    assert!(!s.vim.awaits_mapping());
    s.keys("<Esc>");
    assert_eq!(s.text(), "jone two\nthree");

    // 右侧的按键不再经过映射：`x` 执行 `dd`，`dd` 执行 `x`。
    s.keys("uggx");
    assert_eq!(s.text(), "three");
    s.keys("udd");
    assert_eq!(s.text(), "ne two\nthree");
    s.keys("uY");
    assert_eq!(s.register('0'), "one two");
    // 等待动作时的映射，以及可视模式的映射。
    s.keys("wdL");
    assert_eq!(s.text(), "one \nthree");
    s.keys("uggvjQ");
    assert_eq!(s.text(), "hree");
    // 一条映射是另一条的前缀：超时执行较短的一条，后续的按键选出较长的一条。
    s.keys("ugg,");
    assert!(s.vim.awaits_mapping());
    s.vim.timeout(&mut s.shared, &mut s.host);
    assert_eq!(s.caret(), (1, 0));
    s.keys("gg,w");
    assert_eq!(s.text(), "two\nthree");
    s.keys("u,l");
    assert_eq!(s.caret(), (1, 1));
    // 多键命令的参数不经过映射；右侧可以是宿主的命令。
    s.keys("ggfx").keys("rx");
    assert_eq!(s.text(), "xne two\nthree");
    s.keys("gs");
    assert_eq!(s.host.apps, [AppCommand::Run("Save".into())]);
    // `:normal` 经过映射，`:normal!` 不经过。
    s.keys("gg").ex("normal Y");
    assert_eq!(s.register('0'), "xne two");
    s.keys("j").ex("normal! Y");
    assert_eq!(s.register('0'), "three");
    // 宏按键入的按键记录，回放时经过映射。
    s.keys("uqaYq").keys("j@a");
    assert_eq!(
        (s.register('a').as_str(), s.register('0').as_str()),
        ("Y", "three")
    );
}

#[test]
fn the_command_line_completes_names_options_and_paths() {
    let names = complete("so").unwrap();
    assert_eq!(
        (names.kind, names.candidates),
        (Completing::Command, vec!["sort".to_owned()])
    );
    assert_eq!(complete("set ").map(|c| c.candidates.len()), Some(16));
    assert_eq!(
        complete("w ").map(|c| (c.kind, c.start)),
        Some((Completing::Path, 2))
    );
    assert_eq!(
        complete("b ma").map(|c| (c.kind, c.start)),
        Some((Completing::Buffer, 2))
    );
    assert_eq!(complete("zzz"), None);
}

#[test]
fn normal_and_global_commands_end_what_they_started() {
    let mut s = Session::new("one\ntwo\nthree");
    // 没有结束的插入与多键命令按 `Esc` 结束。
    s.ex("%normal Ax");
    assert_eq!(
        (s.text().as_str(), s.vim.mode()),
        ("onex\ntwox\nthreex", Mode::Normal)
    );
    s.ex("normal d");
    assert_eq!(s.vim.pending(), [] as [Key; 0]);
    s.ex("normal :s/o/0/");
    assert_eq!(
        (s.vim.prompt(), s.text().as_str()),
        (None, "onex\ntwox\nthreex")
    );
    // 逐行执行的命令中不能再逐行执行，替换不逐处确认。
    s.ex("g/o/g/x/d");
    assert_eq!(s.text(), "onex\ntwox\nthreex");
    s.ex("g/x/s/x/!/c");
    assert_eq!(
        (s.text().as_str(), s.vim.confirming()),
        ("one!\ntwo!\nthree!", None)
    );
    s.keys("u");
    assert_eq!(s.text(), "onex\ntwox\nthreex");
    // 被标记的行随编辑移动。
    s.ex("g/e/normal O-");
    assert_eq!(s.text(), "-\nonex\ntwox\n-\nthreex");
    assert_eq!(s.host.tracked, [] as [usize; 0]);
}
