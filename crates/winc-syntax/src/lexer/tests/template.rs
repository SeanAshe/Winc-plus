//! Liquid 与 Nunjucks 的词法测试：`{{ … }}` 输出、`{% … %}` 语句、模板注释，以及之外的 HTML。

use super::*;

fn kind_of(tokens: &[(String, TokenKind)], text: &str) -> Option<TokenKind> {
    tokens
        .iter()
        .find(|(t, _)| t == text)
        .map(|(_, kind)| *kind)
}

/// 输出与语句的定界符按装饰器着色，标签名按关键字，过滤器按函数，字符串和数字照常着色。
#[test]
fn template_tags_highlight_names_filters_and_literals() {
    for file in ["page.liquid", "page.njk"] {
        let (tokens, next) = lex(
            file,
            "<h1>{{ page.title | upcase }}</h1>{% if count > 1 %}",
            LineState::Normal,
        );
        assert_eq!(next, LineState::Normal, "{file}");
        assert_eq!(kind_of(&tokens, "h1"), Some(TokenKind::Tag));
        assert_eq!(kind_of(&tokens, "{{"), Some(TokenKind::Decorator));
        assert_eq!(kind_of(&tokens, "}}"), Some(TokenKind::Decorator));
        assert_eq!(kind_of(&tokens, "upcase"), Some(TokenKind::Function));
        assert_eq!(kind_of(&tokens, "{%"), Some(TokenKind::Decorator));
        assert_eq!(kind_of(&tokens, "if"), Some(TokenKind::Keyword));
        assert_eq!(kind_of(&tokens, "1"), Some(TokenKind::Number));
        assert_eq!(kind_of(&tokens, "%}"), Some(TokenKind::Decorator));

        let (tokens, _) = lex(
            file,
            "{%- assign name = 'a %} b' | append: \"c\" -%}",
            LineState::Normal,
        );
        assert_eq!(kind_of(&tokens, "{%-"), Some(TokenKind::Decorator));
        assert_eq!(kind_of(&tokens, "assign"), Some(TokenKind::Keyword));
        // 引号中的 `%}` 不结束语句。
        assert_eq!(kind_of(&tokens, "'a %} b'"), Some(TokenKind::String));
        assert_eq!(kind_of(&tokens, "append"), Some(TokenKind::Function));
        assert_eq!(kind_of(&tokens, "\"c\""), Some(TokenKind::String));
        assert_eq!(kind_of(&tokens, "-%}"), Some(TokenKind::Decorator));
    }
}

/// 语句可以跨行：下一行继续按模板表达式着色，收尾之后恢复 HTML。
#[test]
fn template_statements_span_lines() {
    let (_, next) = lex("page.njk", "{% set items = [", LineState::Normal);
    assert!(next.is_quoted());
    let (tokens, next) = lex("page.njk", "  'a', true", next);
    assert_eq!(kind_of(&tokens, "'a'"), Some(TokenKind::String));
    assert_eq!(kind_of(&tokens, "true"), Some(TokenKind::Builtin));
    let (tokens, next) = lex("page.njk", "] %}<p>", next);
    assert_eq!(kind_of(&tokens, "%}"), Some(TokenKind::Decorator));
    assert_eq!(kind_of(&tokens, "p"), Some(TokenKind::Tag));
    assert_eq!(next, LineState::Normal);
}

/// 模板注释可以跨行：Nunjucks 是 `{# … #}`，Liquid 是 `{% comment %}` 到 `{% endcomment %}`。
#[test]
fn template_comments_span_lines() {
    let (tokens, next) = lex("page.njk", "{# note", LineState::Normal);
    assert_eq!(tokens, vec![("{# note".into(), TokenKind::Comment)]);
    let (tokens, next) = lex("page.njk", "{{ x }} #}<p>", next);
    assert_eq!(tokens[0], ("{{ x }} #}".into(), TokenKind::Comment));
    assert_eq!(next, LineState::Normal);

    let (_, next) = lex("page.liquid", "{% comment %}", LineState::Normal);
    let (tokens, next) = lex("page.liquid", "{{ x }}", next);
    assert_eq!(tokens, vec![("{{ x }}".into(), TokenKind::Comment)]);
    let (_, next) = lex("page.liquid", "{% endcomment %}", next);
    assert_eq!(next, LineState::Normal);

    // Liquid 的行内注释。
    let (tokens, _) = lex("page.liquid", "{% # note %}", LineState::Normal);
    assert_eq!(kind_of(&tokens, "# note "), Some(TokenKind::Comment));
}

/// 标签内的模板标签不打断标签：之后的属性照常着色。
#[test]
fn template_tags_inside_an_html_tag() {
    let (tokens, next) = lex(
        "page.liquid",
        "<input {% if on %}checked{% endif %} type=\"a\">",
        LineState::Normal,
    );
    assert_eq!(kind_of(&tokens, "if"), Some(TokenKind::Keyword));
    assert_eq!(kind_of(&tokens, "endif"), Some(TokenKind::Keyword));
    assert_eq!(kind_of(&tokens, "type"), Some(TokenKind::Attribute));
    assert_eq!(next, LineState::Normal);
}

/// 其他 HTML 类语言不识别模板标签。
#[test]
fn plain_html_ignores_template_tags() {
    let (tokens, _) = lex("index.html", "<p>{% if a %}</p>", LineState::Normal);
    assert_eq!(kind_of(&tokens, "if"), None);
    assert_eq!(kind_of(&tokens, "{%"), None);
}
