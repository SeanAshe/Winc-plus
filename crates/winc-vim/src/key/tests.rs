//! 按键的归并与按键记法的测试。

use super::notation::{parse, write};
use super::*;

/// 带修饰键 `mods` 的字符键。
fn modified(c: char, mods: Mods) -> Key {
    Key::new(Code::Char(c), mods)
}

#[test]
fn control_characters_and_ctrl_combinations_have_one_form() {
    // 控制字符、`Ctrl` 与大小写字母的组合是同一个按键。
    assert_eq!(Key::char('\u{4}'), Key::ctrl('d'));
    assert_eq!(Key::ctrl('D'), Key::ctrl('d'));
    assert_eq!(Key::ctrl('d').code(), Code::Char('d'));
    assert_eq!(Key::ctrl('d').mods(), Mods::CTRL);
    assert_eq!(Key::char('\u{7f}'), Key::ctrl('?'));
    assert_eq!(Key::char('\0'), Key::ctrl('@'));

    assert_eq!(Key::ctrl('['), Key::named(Named::Esc));
    assert_eq!(Key::char('\u{1b}'), Key::named(Named::Esc));
    assert_eq!(Key::ctrl('i'), Key::named(Named::Tab));
    assert_eq!(Key::char('\t'), Key::named(Named::Tab));
    assert_eq!(Key::ctrl('M'), Key::named(Named::Enter));
    assert_eq!(Key::char('\r'), Key::named(Named::Enter));
    assert_eq!(Key::char('\n'), Key::ctrl('j'));
    // `Ctrl` 与 `h` 的组合不是退格键。
    assert_ne!(Key::ctrl('h'), Key::named(Named::Backspace));
}

#[test]
fn shift_folds_into_letters_only() {
    assert_eq!(modified('a', Mods::SHIFT), Key::char('A'));
    assert_eq!(modified('A', Mods::SHIFT), Key::char('A'));
    // 带 `Ctrl` 时字母不区分大小写，`Shift` 保留。
    let key = modified('A', Mods::CTRL.with(Mods::SHIFT));
    assert_eq!(key.code(), Code::Char('a'));
    assert_eq!(key.mods(), Mods::CTRL.with(Mods::SHIFT));
    // 其他字符和功能键上的 `Shift` 保留。
    assert_eq!(modified('1', Mods::SHIFT).mods(), Mods::SHIFT);
    let left = Key::new(Code::Named(Named::Left), Mods::SHIFT);
    assert_eq!(left.mods(), Mods::SHIFT);
    assert_ne!(left, Key::named(Named::Left));
}

#[test]
fn as_char_is_the_typed_character() {
    assert_eq!(Key::char('x').as_char(), Some('x'));
    assert_eq!(Key::char('中').as_char(), Some('中'));
    assert_eq!(modified('1', Mods::SHIFT).as_char(), Some('1'));
    assert_eq!(Key::ctrl('x').as_char(), None);
    assert_eq!(modified('x', Mods::ALT).as_char(), None);
    assert_eq!(Key::named(Named::Tab).as_char(), None);
}

#[test]
fn notation_parses_plain_and_special_keys() {
    let esc = Key::named(Named::Esc);
    assert_eq!(
        parse("d2w<Esc>"),
        [Key::char('d'), Key::char('2'), Key::char('w'), esc]
    );
    assert_eq!(parse("<C-d>"), [Key::ctrl('d')]);
    assert_eq!(parse("<c-D>"), [Key::ctrl('d')]);
    assert_eq!(parse("<C-[><esc><ESC>"), [esc, esc, esc]);
    assert_eq!(
        parse("<CR><Return><Enter><C-m>"),
        [Key::named(Named::Enter); 4]
    );
    assert_eq!(parse("<Tab><C-i>"), [Key::named(Named::Tab); 2]);
    assert_eq!(parse("<BS><BackSpace>"), [Key::named(Named::Backspace); 2]);
    assert_eq!(parse("<Del><Delete>"), [Key::named(Named::Delete); 2]);
    assert_eq!(parse("<NL><C-j>"), [Key::ctrl('j'); 2]);
    assert_eq!(parse("<Nul>"), [Key::ctrl('@')]);
    assert_eq!(
        parse("<lt><Space><Bar><Bslash>"),
        ['<', ' ', '|', '\\'].map(Key::char)
    );
    assert_eq!(
        parse("<F1><f12>"),
        [Named::F(1), Named::F(12)].map(Key::named)
    );
    assert_eq!(
        parse("<S-Left><C-S-Right><M-x><A-x><D-s>"),
        [
            Key::new(Code::Named(Named::Left), Mods::SHIFT),
            Key::new(Code::Named(Named::Right), Mods::CTRL.with(Mods::SHIFT)),
            modified('x', Mods::ALT),
            modified('x', Mods::ALT),
            modified('s', Mods::CMD),
        ]
    );
    assert_eq!(parse("<S-a>"), [Key::char('A')]);
    assert_eq!(parse("<C-->"), [Key::ctrl('-')]);
    assert_eq!(parse("<C->>"), [Key::ctrl('>')]);
    assert_eq!(parse("<C-lt>"), [Key::ctrl('<')]);
    // 文本中的控制字符按键归并。
    assert_eq!(
        parse("a\n\t"),
        [Key::char('a'), Key::ctrl('j'), Key::named(Named::Tab)]
    );
    assert_eq!(parse("中é"), [Key::char('中'), Key::char('é')]);
}

#[test]
fn text_that_is_not_notation_is_taken_literally() {
    let literal = |text: &str| text.chars().map(Key::char).collect::<Vec<_>>();
    for text in [
        "<", "<>", "<<", "<ip>ip", "<x>", "<Esc", "<F0>", "<F36>", "<F01>", "<Fx>", "<C->", "<C-",
        "<-x>", "<Z-x>", "<C-xy>", "<Escape>", "<S->", "a<b", "<Esc <CR",
    ] {
        assert_eq!(parse(text), literal(text), "{text}");
    }
    // 记法之前的 `<` 是字符，其后的记法仍然解析成按键。
    assert_eq!(parse("<<Esc>"), [Key::char('<'), Key::named(Named::Esc)]);
    assert_eq!(parse(">><lt><lt>"), literal(">><<"));
}

#[test]
fn notation_writes_the_canonical_form() {
    assert_eq!(write(&parse("d2w<esc>")), "d2w<Esc>");
    assert_eq!(
        write(&parse("<Return><c-D><C-[><C-I>")),
        "<CR><C-d><Esc><Tab>"
    );
    assert_eq!(write(&parse("< >|\\")), "<lt><Space>>|\\");
    assert_eq!(write(&parse("<Bar><Bslash><NL><Nul>")), "|\\<C-j><C-@>");
    assert_eq!(
        write(&parse("<s-left><d-s-c-m-F5>")),
        "<S-Left><M-C-S-D-F5>"
    );
    assert_eq!(
        write(&parse("<C-S-a><S-Space><C-Space>")),
        "<C-S-a><S-Space><C-Space>"
    );
    assert_eq!(Key::ctrl('>').to_string(), "<C->>");
    assert_eq!(Key::ctrl('<').to_string(), "<C-lt>");
    assert_eq!(Key::char('<').to_string(), "<lt>");
    assert_eq!(Key::named(Named::F(200)).to_string(), "<F35>");
    assert_eq!(format!("{:?}", Mods::CTRL.with(Mods::ALT)), "Mods(M-C-)");
}

#[test]
fn every_key_survives_a_round_trip_through_notation() {
    let mut codes: Vec<Code> = (0..=0x24F_u32)
        .filter_map(char::from_u32)
        .chain(['中', '\u{301}', '😀', '\u{2028}', '\u{feff}'])
        .map(Code::Char)
        .collect();
    codes.extend(
        [
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
        ]
        .map(Code::Named),
    );
    codes.extend((0..=40).map(|n| Code::Named(Named::F(n))));
    let all = [Mods::CTRL, Mods::SHIFT, Mods::ALT, Mods::CMD];

    let mut keys = Vec::new();
    for bits in 0..16 {
        let mods = (all.iter().enumerate())
            .filter(|(i, _)| bits & (1 << i) != 0)
            .fold(Mods::NONE, |mods, (_, m)| mods.with(*m));
        keys.extend(codes.iter().map(|code| Key::new(*code, mods)));
    }
    for key in &keys {
        assert_eq!(parse(&key.to_string()), [*key], "{key:?} 写作 {key}");
    }
    // 连在一起的记法之间互不影响。
    for pair in keys.chunks(2) {
        assert_eq!(parse(&write(pair)), pair, "{pair:?}");
    }
    assert_eq!(parse(&write(&keys)), keys);
}

#[test]
fn parsing_reads_a_bounded_distance_after_each_bracket() {
    // 每个 `<` 之后最多向后读取固定的长度，全是 `<` 的长文本在线性时间内解析完。
    let text = "<".repeat(200_000);
    assert_eq!(parse(&text).len(), 200_000);
    let text = "<C-".repeat(100_000);
    assert_eq!(parse(&text).len(), 300_000);
}
