//! Astro 的词法测试：文件开头 `---` 之间的 TypeScript frontmatter，以及之后的 HTML 模板。

use super::*;

/// Astro 的文件开头 `---` 之间按 TypeScript 着色，之后的模板按 HTML；关闭后的 `---` 不再
/// 开始 frontmatter。
#[test]
fn astro_frontmatter_and_template() {
    let (tokens, next) = lex("a.astro", "---", LineState::Start);
    assert_eq!(tokens, vec![("---".into(), TokenKind::Decorator)]);
    assert_eq!(next, LineState::Frontmatter { inner: 0 });

    let (tokens, next) = lex("a.astro", "import Layout from './Layout.astro';", next);
    assert_eq!(tokens[0], ("import".into(), TokenKind::Keyword));
    assert!(
        tokens
            .iter()
            .any(|(text, kind)| text == "'./Layout.astro'" && *kind == TokenKind::String)
    );
    assert!(matches!(next, LineState::Frontmatter { .. }));

    let (tokens, next) = lex("a.astro", "---", next);
    assert_eq!(tokens, vec![("---".into(), TokenKind::Decorator)]);
    assert_eq!(next, LineState::Normal);

    // 模板沿用 HTML 的标签、属性与内嵌脚本。
    let (tokens, next) = lex("a.astro", "<h1 class=\"title\">{title}</h1>", next);
    assert_eq!(tokens[0], ("h1".into(), TokenKind::Tag));
    assert!(tokens.contains(&("class".into(), TokenKind::Attribute)));
    assert_eq!(next, LineState::Normal);

    let (tokens, next) = lex("a.astro", "---", next);
    assert!(tokens.is_empty());
    assert_eq!(next, LineState::Normal);

    let (_, next) = lex("a.astro", "<script>", next);
    assert!(matches!(next, LineState::Embedded { script: true, .. }));

    // frontmatter 中的块注释和模板字符串同样跨行。
    let (_, mut next) = lex("a.astro", "---", LineState::Start);
    let (tokens, state) = lex("a.astro", "/* note", next);
    assert_eq!(tokens, vec![("/* note".into(), TokenKind::Comment)]);
    next = state;
    let (_, state) = lex("a.astro", "still note */", next);
    next = state;
    let (tokens, state) = lex("a.astro", "const s = `a", next);
    assert_eq!(tokens[0], ("const".into(), TokenKind::Keyword));
    next = state;
    let (_, next) = lex("a.astro", "b`;", next);
    assert!(matches!(next, LineState::Frontmatter { .. }));
}

/// 首行不是 `---` 时没有 frontmatter；只有第一行的 `---` 才算开始，普通行传入 `Normal`
/// 不会开始 frontmatter。
#[test]
fn astro_frontmatter_starts_only_at_the_first_line() {
    let (tokens, next) = lex("a.astro", "<p>Hello</p>", LineState::Start);
    assert_eq!(tokens[0], ("p".into(), TokenKind::Tag));
    assert_eq!(next, LineState::Normal);

    let (tokens, next) = lex("a.astro", "---", LineState::Normal);
    assert!(tokens.is_empty());
    assert_eq!(next, LineState::Normal);

    // 分隔线可以带行尾空白。
    let (_, next) = lex("a.astro", "---  ", LineState::Start);
    assert_eq!(next, LineState::Frontmatter { inner: 0 });
}

/// frontmatter 中多行模板字符串里的 `---` 是 TypeScript 的内容，不结束 frontmatter。
#[test]
fn astro_fence_inside_a_template_literal_does_not_close_frontmatter() {
    let (_, next) = lex("a.astro", "---", LineState::Start);
    let (_, next) = lex("a.astro", "const md = `", next);
    let (tokens, next) = lex("a.astro", "---", next);
    assert_eq!(tokens, vec![("---".into(), TokenKind::String)]);
    let (_, next) = lex("a.astro", "`;", next);
    assert_eq!(next, LineState::Frontmatter { inner: 0 });
    let (tokens, next) = lex("a.astro", "---", next);
    assert_eq!(tokens, vec![("---".into(), TokenKind::Decorator)]);
    assert_eq!(next, LineState::Normal);
}

/// 内层状态放不进紧凑编码时只丢掉内层状态，仍留在 frontmatter 中。
#[test]
fn astro_frontmatter_survives_an_inner_state_that_does_not_fit() {
    let state = LineState::Frontmatter { inner: 0x23 };
    assert_eq!(expand(compact(state)), LineState::Frontmatter { inner: 0 });
}
