//! 随机按键测试：固定种子的随机按键序列直接驱动解析器与状态机，核对不 panic 与各项不变量。
//!
//! 序列由种子决定，失败时的信息含种子与步数，可以原样重现。按键大多取自命令表用到的按键，
//! 使序列经常构成完整的命令；其余是任意的字符、功能键与修饰键组合。

mod common;

use std::time::{Duration, Instant};

use winc_core::{Lines, Selection, movement};
use winc_vim::command::ENTRIES;
use winc_vim::parser::MAX_COUNT;
use winc_vim::{Action, Body, Code, Command, Force, Key, Mode, Mods, Motion, Named, Outcome};
use winc_vim::{Host, Parser, Shared, Target, Vim, notation};

use common::TestHost;

/// 伪随机数生成器（SplitMix64）：同一个种子给出同一个序列，不依赖平台。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// 返回 `0..n` 之内的数；`n` 不为 0。
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

const NAMED: [Named; 16] = [
    Named::Esc,
    Named::Enter,
    Named::Tab,
    Named::Backspace,
    Named::Delete,
    Named::Insert,
    Named::Up,
    Named::Down,
    Named::Left,
    Named::Right,
    Named::Home,
    Named::End,
    Named::PageUp,
    Named::PageDown,
    Named::F(1),
    Named::F(35),
];

const MODES: [Mode; 4] = Mode::ALL;

/// 返回常用的按键：命令表中出现的全部按键、数字、寄存器名、功能键与若干非 ASCII 字符。
fn common_keys() -> Vec<Key> {
    let mut keys: Vec<Key> = (ENTRIES.iter())
        .flat_map(|entry| notation::parse(entry.keys))
        .collect();
    keys.extend(
        "0123456789\"-_+*/:.%=#azAZ!@ \u{301}中é😀"
            .chars()
            .map(Key::char),
    );
    keys.extend(NAMED.map(Key::named));
    // 取消与次数的删除在序列中多出现一些。
    keys.extend([Key::named(Named::Esc), Key::named(Named::Delete)].repeat(4));
    keys
}

/// 返回一个随机的按键：多数取自 `common`，其余是任意的主体与修饰键。
fn random_key(rng: &mut Rng, common: &[Key]) -> Key {
    if rng.below(100) < 85 {
        return rng.pick(common);
    }
    let code = match rng.below(4) {
        0 => Code::Named(rng.pick(&NAMED)),
        1 => Code::Named(Named::F(u8::try_from(rng.below(256)).unwrap())),
        // 任意码位：包括控制字符与 ASCII 之外的字符；代理区不是字符，改用替换字符。
        2 => Code::Char(
            char::from_u32(u32::try_from(rng.below(0x11_0000)).unwrap()).unwrap_or('\u{fffd}'),
        ),
        _ => Code::Char(char::from(u8::try_from(rng.below(128)).unwrap())),
    };
    let mods = [Mods::CTRL, Mods::SHIFT, Mods::ALT, Mods::CMD]
        .into_iter()
        .filter(|_| rng.below(4) == 0)
        .fold(Mods::NONE, Mods::with);
    Key::new(code, mods)
}

/// 是否参数与命令相符：行内查找、标记、替换字符与插入寄存器带字符参数，其余命令不带。
fn argument_fits(command: &Command) -> bool {
    let find = |motion: Motion| {
        matches!(
            motion,
            Motion::FindForward
                | Motion::FindBackward
                | Motion::TillForward
                | Motion::TillBackward
                | Motion::Mark
                | Motion::MarkLine
        )
    };
    match command.body {
        Body::Motion(movement)
        | Body::Operator {
            target: Target::Motion(movement),
            ..
        } => find(movement.motion) == movement.arg.is_some(),
        Body::Action { action, arg } => {
            let expects = matches!(
                action,
                Action::ReplaceChar
                    | Action::SetMark
                    | Action::InsertRegister
                    | Action::Record
                    | Action::Play
                    | Action::DeleteSurround
                    | Action::ChangeSurround(_)
            );
            expects == arg.is_some()
        }
        _ => true,
    }
}

/// 核对一条命令自身的不变量，以及与所在模式的关系。
fn check_command(mode: Mode, command: &Command, context: &str) {
    if let Some(count) = command.count {
        assert!((1..=MAX_COUNT).contains(&count), "{context}：次数 {count}");
    }
    if let Some(name) = command.register {
        assert!(
            name.is_ascii_alphanumeric() || "\"-_+*/:.%".contains(name),
            "{context}：寄存器 {name:?}"
        );
    }
    assert!(argument_fits(command), "{context}：参数与命令不符");

    match (mode, command.body) {
        (Mode::Insert, body) => {
            assert!(matches!(body, Body::Action { .. }), "{context}");
            assert_eq!((command.register, command.count), (None, None), "{context}");
        }
        (Mode::Visual, Body::Operator { target, force, .. }) => {
            let selected = matches!(target, Target::Selection | Target::SelectedLines { .. });
            assert!(selected, "{context}");
            assert!(matches!(force, None | Some(Force::Linewise)), "{context}");
        }
        (_, Body::Operator { target, .. }) => {
            let selected = matches!(target, Target::Selection | Target::SelectedLines { .. });
            assert!(!selected, "{context}");
        }
        (Mode::Normal | Mode::OperatorPending, Body::Object(_)) => {
            panic!("{context}：普通模式下不能单独使用文本对象")
        }
        _ => {}
    }
}

/// 以种子 `seed` 生成 `steps` 个按键驱动解析器，返回各类结果出现的次数。
///
/// 另有一个解析器在每条命令结束后重置：两者的结果必须始终相同，即命令结束后解析器不保留状态。
fn run(seed: u64, steps: usize, common: &[Key]) -> [usize; 6] {
    let mut rng = Rng(seed);
    let mut parser = Parser::default();
    let mut fresh = Parser::default();
    let mut mode = Mode::Normal;
    let mut seen = [0; 6];

    for step in 0..steps {
        if rng.below(50) == 0 {
            mode = rng.pick(&MODES);
        }
        let key = random_key(&mut rng, common);
        let context = format!("种子 {seed} 第 {step} 步，{mode:?} 模式，按键 {key}");

        let outcome = parser.feed(mode, key);
        assert_eq!(
            fresh.feed(mode, key),
            outcome,
            "{context}：重置后的解析器结果不同"
        );
        if outcome == Outcome::Pending {
            let pending = parser.pending();
            assert!(!pending.is_empty(), "{context}");
            assert_eq!(
                notation::parse(&notation::write(pending)),
                pending,
                "{context}"
            );
        } else {
            assert!(parser.pending().is_empty(), "{context}");
            assert!(!parser.is_operator_pending(), "{context}");
            fresh.reset();
        }
        if let Outcome::Command(command) = &outcome {
            check_command(mode, command, &context);
        }
        if mode == Mode::Insert {
            assert!(!parser.is_operator_pending(), "{context}");
        }

        seen[match outcome {
            Outcome::Pending => 0,
            Outcome::Command(_) => 1,
            Outcome::Unsupported => 2,
            Outcome::Invalid => 3,
            Outcome::Cancelled => 4,
            Outcome::Unbound => 5,
        }] += 1;
    }
    seen
}

#[test]
fn random_keys_never_break_the_parser() {
    let common = common_keys();
    let mut total = [0; 6];
    for seed in 0..200 {
        let seen = run(seed, 2_000, &common);
        // 同一个种子给出同一个结果。
        if seed < 4 {
            assert_eq!(run(seed, 2_000, &common), seen);
        }
        for (sum, count) in total.iter_mut().zip(seen) {
            *sum += count;
        }
    }
    // 每一类结果都大量出现，序列没有退化成只有无效按键。
    let [pending, commands, unsupported, invalid, cancelled, unbound] = total;
    assert!(pending > 50_000, "{total:?}");
    assert!(commands > 50_000, "{total:?}");
    assert!(unsupported > 2_000, "{total:?}");
    assert!(invalid > 10_000, "{total:?}");
    assert!(cancelled > 500, "{total:?}");
    assert!(unbound > 10_000, "{total:?}");
}

#[test]
fn random_text_parses_as_notation_and_writes_back_stably() {
    let pieces = [
        "<", ">", "-", "C-", "S-", "M-", "D-", "A-", "Esc", "CR", "lt", "Space", "F1", "F99", "x",
        "d", "\"", " ", "\n", "\t", "中", "\u{301}", "<C-", "<S-Left>", "<Esc>", "<lt>", ">>",
    ];
    let mut rng = Rng(0x5eed);
    for round in 0..20_000 {
        let length = rng.below(12);
        let text: String = (0..length).map(|_| rng.pick(&pieces)).collect();
        let keys = notation::parse(&text);
        let written = notation::write(&keys);
        assert_eq!(
            notation::parse(&written),
            keys,
            "第 {round} 轮：{text:?} 写作 {written:?}"
        );
        assert_eq!(
            notation::write(&notation::parse(&written)),
            written,
            "第 {round} 轮"
        );
    }
}

/// 随机测试所用的文本：词与标点、缩进与制表符、空行、宽字符、组合字符与由多个码位组成的
/// 表情、括号与引号、没有内容的文本、不以换行符结尾的文本。
const TEXTS: &[&str] = &[
    "foo bar.baz  qux_1 (x) end\n   lead, tail;  \n\n中文 text，全角。next\na-b #$% e\u{301}x\n",
    "fn main() {\n\tcall(a, [b, c], \"s)\");\n    if (x) { y('(') }\n}\n\n{\n  inner } {\n}\ntail\n",
    "First one.  Second one! Third?  \nStill (third.) Fourth.\n\n\nNew para. End\n   \nlast",
    "👨\u{200d}👩\u{200d}👧 family 👍\u{1f3fd} flag 🇨🇳 e\u{301}\u{302}\n\u{301}lead\n",
    "",
    "\n",
    "x",
];

/// 核对状态机在一次按键之后的不变量。环境变量 `WINC_VIM_SEEDS` 可以把种子数调大，用于开发时
/// 更长时间的随机测试。
fn check_state(vim: &Vim, host: &TestHost, context: &str) {
    let text = &host.rope;
    let end = text.end_pos();
    let boundary = |pos: usize| pos <= end && movement::snap_to_grapheme(text, pos) == pos;
    let selection = host.selection;
    let caret = vim.caret(host);
    assert!(
        caret <= end && selection.anchor <= end && selection.head <= end,
        "{context}"
    );
    // 插入模式中的删除由宿主执行，光标可以落在被拆开的字素之间；离开时重新对齐。
    if vim.mode() != Mode::Insert {
        assert!(boundary(caret), "{context}：光标 {caret} 不在字素边界上");
        assert!(
            boundary(selection.anchor),
            "{context}：选区起点 {}",
            selection.anchor
        );
        assert!(
            boundary(selection.head),
            "{context}：选区终点 {}",
            selection.head
        );
    }

    // 逐处确认期间撤销分组未结束，当前询问的匹配显示为选区。
    let confirming = vim.confirming().is_some();
    match vim.mode() {
        Mode::Insert => assert_eq!(host.groups, 1, "{context}：插入期间有一个撤销分组"),
        _ if confirming => assert_eq!(host.groups, 1, "{context}：确认期间有一个撤销分组"),
        mode => {
            assert_eq!(host.groups, 0, "{context}：撤销分组已结束");
            if mode == Mode::Normal {
                // 普通模式：选区为空，光标在字符上（空行除外），不在末尾的空行上。
                assert_eq!(selection, Selection::point(caret), "{context}");
                let line = text.line_of(caret);
                // 插入模式中执行一条普通模式的命令期间，光标可以停在行尾之后。
                let on_char = caret < text.line_end(line) || text.line_len(line) == 0;
                assert!(
                    on_char || vim.is_suspended(),
                    "{context}：光标 {caret} 在行尾之后"
                );
                let phantom = line > 0 && line + 1 == text.line_count() && text.line_len(line) == 0;
                assert!(!phantom, "{context}：光标在末尾的空行上");
            } else {
                assert!(vim.visual_kind().is_some(), "{context}");
            }
        }
    }
}

/// 随机按键之间穿插的按键序列：命令行、替换、逐行执行、排序，以及宏的记录与回放。
const SCRIPTS: &[&str] = &[
    ":s/a/X/g<CR>",
    ":%s/e/E/<CR>",
    r":%s/\w\+/\u&/g<CR>",
    r":%s/\n//<CR>",
    r":%s/<Space>/\r/g<CR>",
    ":%s/a/b/gc<CR>",
    ":%s/e/x/gc<CR>yna",
    ":s/./&&/gc<CR>yyq",
    ":g/a/d<CR>",
    ":v/a/d<CR>",
    ":g/^/m0<CR>",
    ":g/e/s/e/E/g<CR>",
    ":g/./normal<Space>Ax<CR>",
    ":g/a/t.<CR>",
    ":%normal<Space>Ax<CR>",
    ":%normal<Space>dd<CR>",
    ":2,3normal<Space>ihello<CR>",
    ":normal<Space>dw<CR>",
    ":sort<CR>",
    ":sort!<Space>u<CR>",
    ":sort<Space>n<CR>",
    ":2,3d<CR>",
    ":2,4m0<CR>",
    ":m$<CR>",
    ":t.<CR>",
    ":%j<CR>",
    ":%><CR>",
    ":1,2y<Space>a<CR>",
    ":pu<CR>",
    ":0pu<Space>a<CR>",
    ":/a/d<CR>",
    ":$<CR>",
    ":set<Space>ic<Space>gdefault<CR>",
    ":set<Space>noic<Space>nogdefault<Space>nowrapscan<CR>",
    ":u<CR>",
    ":wq<CR>",
    ":zzz<CR>",
    "qaxjq",
    "qaA!<Esc>jq",
    "qbdwq",
    "qaIab<Esc>jq",
    "qaqqaxj@aq",
    "@a",
    "3@a",
    "@@",
    "@b",
    "100@a",
    "@:",
    "5@:",
    "&",
    "g&",
    "\"ayy@a",
    "ga",
    "gt",
    "ZZ",
    "ysiw)",
    "ysiw[",
    "yss\"",
    "ys$}",
    "viwS'",
    "VjS)",
    "ds)",
    "ds\"",
    "cs)]",
    "cs'\"",
    "dst",
    "dii",
    "yai",
    ">ii",
    "vii",
    "<C-w>w",
    "<C-^>",
    ":h<CR>",
    "`Q",
];

/// 以种子 `seed` 生成 `steps` 个按键驱动状态机，核对不变量与撤销；返回单个按键的最长耗时与
/// 核对过撤销、重做的命令数。
fn drive(seed: u64, steps: usize, common: &[Key]) -> (Duration, usize) {
    let mut rng = Rng(seed ^ 0xA5A5_5A5A);
    let mut host = TestHost::new(rng.pick(TEXTS));
    let mut vim = Vim::default();
    let mut shared = Shared::default();
    // 进入插入模式之前的文本：离开时撤销一次应回到这里。
    let mut insert_start: Option<String> = None;
    // 开始逐处确认之前的文本：确认结束时撤销一次应回到这里。
    let mut confirm_start: Option<String> = None;
    // 穿插的按键序列中尚未输入的按键。
    let mut script: std::collections::VecDeque<Key> = std::collections::VecDeque::new();
    let mut slowest = Duration::ZERO;
    let mut verified = 0;
    let undo_key = |key: Key| key == Key::char('u') || key == Key::ctrl('r');

    for step in 0..steps {
        // 偶尔由宿主改动选区，模拟鼠标与应用的命令；其中一部分是多个光标。
        if rng.below(40) == 0 && vim.prompt().is_none() {
            let end = host.rope.end_pos();
            let pos = |rng: &mut Rng| movement::snap_to_grapheme(&host.rope, rng.below(end + 1));
            let (anchor, head) = (pos(&mut rng), pos(&mut rng));
            let selection = if rng.below(2) == 0 {
                Selection::point(head)
            } else {
                Selection::new(anchor, head)
            };
            if rng.below(3) == 0 {
                let mut all: Vec<usize> = (0..2 + rng.below(3)).map(|_| pos(&mut rng)).collect();
                all.sort_unstable();
                all.dedup();
                let all = all.into_iter().map(Selection::point).collect();
                host.set_selections(all, 0);
            } else {
                host.set_selections(vec![selection], 0);
            }
            if vim.mode() == Mode::Insert {
                vim.insert_moved(&mut host);
                insert_start = None;
            } else {
                vim.sync(&mut host);
            }
            confirm_start = None;
            check_state(
                &vim,
                &host,
                &format!("种子 {seed} 第 {step} 步，选区被改动之后"),
            );
        }

        // 普通模式下空闲时，偶尔输入一段命令行或宏。
        let idle = vim.mode() == Mode::Normal && vim.pending().is_empty() && vim.prompt().is_none();
        if script.is_empty() && idle && vim.confirming().is_none() && rng.below(25) == 0 {
            script.extend(notation::parse(rng.pick(SCRIPTS)));
        }
        let key = match script.pop_front() {
            Some(key) => key,
            None => random_key(&mut rng, common),
        };
        let context = format!(
            "种子 {seed} 第 {step} 步，{:?} 模式，按键 {key}",
            vim.mode()
        );
        let before_text = host.text();
        let before_mode = vim.mode();
        // 是否正在插入模式中执行一条普通模式的命令：命令结束后回到插入模式，另起一个撤销单元。
        let suspended = vim.is_suspended();
        let was_confirming = vim.confirming().is_some();
        let undone = host.undone;
        // 设置 `WINC_VIM_TRACE` 为种子时打印该种子的每个按键，便于重现失败。
        if std::env::var("WINC_VIM_TRACE").is_ok_and(|trace| trace == seed.to_string()) {
            println!("{step} {before_mode:?} {key} {:?}", host.selection);
        }
        let started = Instant::now();
        common::press(&mut vim, &mut shared, &mut host, key);
        let elapsed = started.elapsed();
        if elapsed > Duration::from_secs(3) {
            println!("慢：{context}，耗时 {elapsed:?}");
        }
        slowest = slowest.max(elapsed);
        check_state(&vim, &host, &context);

        let after_mode = vim.mode();
        // 插入模式的 `Ctrl-O`：之后的一条命令结束时回到插入模式，其修改不属于新的插入。
        let resumed = suspended && after_mode == Mode::Insert;
        // 回到插入模式的可能是原来的插入，也可能是命令自己进入的插入：这次插入不核对撤销。
        if resumed {
            insert_start = None;
        } else if before_mode != Mode::Insert && after_mode == Mode::Insert {
            // 进入插入的回放中有撤销、重做时，撤销一次回到的不是回放之前的文本，不核对。
            insert_start = (host.undone == undone).then(|| before_text.clone());
        }
        // 一条命令（或一次插入连同引起插入的命令）撤销后文本逐字节还原，重做后回到执行之后。
        let confirming = vim.confirming().is_some();
        if confirming && !was_confirming {
            confirm_start = Some(before_text.clone());
        }
        let finished = match (before_mode, after_mode) {
            _ if resumed || vim.prompt().is_some() || confirming => None,
            // 按键引发的回放或命令行中有撤销、重做时不核对。
            _ if host.undone != undone => {
                confirm_start = None;
                None
            }
            _ if was_confirming => confirm_start.take(),
            (Mode::Insert, Mode::Insert) => None,
            (Mode::Insert, _) => insert_start.take(),
            (_, Mode::Insert) => None,
            _ if undo_key(key) => None,
            _ => Some(before_text),
        };
        if let Some(original) = finished {
            let after = host.text();
            if after != original && vim.pending().is_empty() {
                let (selection, others) = (host.selection, host.others.clone());
                assert!(host.undo(), "{context}：有可撤销的单元");
                assert_eq!(host.text(), original, "{context}：撤销后还原");
                assert!(host.redo(), "{context}：有可重做的单元");
                assert_eq!(host.text(), after, "{context}：重做后恢复");
                host.selection = selection;
                host.others = others;
                verified += 1;
            }
        }
    }
    (slowest, verified)
}

#[test]
fn random_keys_keep_the_state_machine_consistent() {
    let common = common_keys();
    let seeds = std::env::var("WINC_VIM_SEEDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(150);
    let (mut slowest, mut verified) = (Duration::ZERO, 0);
    for seed in 0..seeds {
        let (time, count) = drive(seed, 1_500, &common);
        slowest = slowest.max(time);
        verified += count;
    }
    // 次数与生成的文本都有上限：任何一个按键都在有限的时间内处理完。
    // 回放的时间上限是两秒，达到上限的按键略多于此。
    assert!(
        slowest < Duration::from_secs(4),
        "最慢的按键耗时 {slowest:?}"
    );
    assert!(
        verified > 10 * usize::try_from(seeds).unwrap(),
        "只核对了 {verified} 次撤销"
    );
    println!("核对撤销与重做 {verified} 次，最慢的按键耗时 {slowest:?}");
}
