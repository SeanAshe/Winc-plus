//! [`Highlighter`] 的测试：行首状态的推算与失效、回看行数的限制、超长行的截断，
//! 以及冷扫描的性能测量（默认忽略）。

use ropey::Rope;

use super::*;

#[test]
fn state_advances_within_budget_and_invalidates() {
    let text = Rope::from_str("a = 1\n\"\"\"doc\nstill doc\n\"\"\"\nb = 2\n");
    let mut highlighter = Highlighter::new(detect("a.py"));

    assert_eq!(highlighter.state_at(&text, 3, 1), None);
    assert_eq!(
        highlighter.state_at(&text, 3, 10),
        Some(LineState::TripleDouble)
    );
    assert_eq!(highlighter.state_at(&text, 4, 10), Some(LineState::Normal));

    highlighter.invalidate_from(1);
    assert_eq!(highlighter.states.len(), 2);
    assert_eq!(
        highlighter.state_at(&text, 99, 100),
        Some(LineState::Normal)
    );
}

/// Astro 从文件第一行开始识别 frontmatter，关闭后的模板行恢复普通状态。
#[test]
fn astro_frontmatter_states() {
    let text = Rope::from_str("---\nconst a = 1;\n---\n<p>{a}</p>\n");
    let mut highlighter = Highlighter::new(detect("a.astro"));

    assert_eq!(highlighter.known_state(0), Some(LineState::Start));
    assert_eq!(
        highlighter.state_at(&text, 1, 10),
        Some(LineState::Frontmatter { inner: 0 })
    );
    assert_eq!(
        highlighter.state_at(&text, 2, 10),
        Some(LineState::Frontmatter { inner: 0 })
    );
    assert_eq!(highlighter.state_at(&text, 3, 10), Some(LineState::Normal));
    assert_eq!(
        LineState::Frontmatter { inner: 0 }
            .embedded_language()
            .map(|language| language.id),
        Some("typescript")
    );
}

#[test]
fn state_near_uses_only_nearby_lines() {
    let text = Rope::from_str("x = 1\n\"\"\"doc\nstill doc\nmore\n\"\"\"\n");
    let mut highlighter = Highlighter::new(detect("a.py"));

    assert_eq!(
        highlighter.state_near(&text, 3, 10),
        LineState::TripleDouble
    );
    assert_eq!(highlighter.states.len(), 1);
    // 已推算的一段继续使用：只多推算一行，结果与从更早处重新推算相同。
    assert_eq!(highlighter.near_from, 0);
    assert_eq!(highlighter.state_near(&text, 4, 1), LineState::TripleDouble);
    assert_eq!((highlighter.near_from, highlighter.near.len()), (0, 5));
    // 编辑后从修改所在行起失效，`near` 只保留到该行。
    highlighter.invalidate_from(2);
    assert_eq!(highlighter.near.len(), 3);

    // 回看的行数不够，看不到文档字符串的开头，按普通内容处理。
    let mut highlighter = Highlighter::new(detect("a.py"));
    assert_eq!(highlighter.state_near(&text, 3, 1), LineState::Normal);
}

#[test]
fn scan_sees_only_line_prefix() {
    // 文档字符串开头恰好落在前 `MAX_LINE_CHARS` 个字符之内与之外，各测一组。
    let inside = format!("{}\"\"\"", "a".repeat(MAX_LINE_CHARS - 3));
    let outside = format!("{}\"\"\"", "a".repeat(MAX_LINE_CHARS));
    let mut highlighter = Highlighter::new(detect("a.py"));

    let text = Rope::from_str(&format!("{inside}\nx\n"));
    assert_eq!(
        highlighter.state_at(&text, 1, 10),
        Some(LineState::TripleDouble)
    );
    assert_eq!(
        highlighter.state_near(&text, 1, 10),
        LineState::TripleDouble
    );

    highlighter.invalidate_from(0);
    let text = Rope::from_str(&format!("{outside}\nx\n"));
    assert_eq!(highlighter.state_at(&text, 1, 10), Some(LineState::Normal));
    assert_eq!(highlighter.state_near(&text, 1, 10), LineState::Normal);

    // 超长的行在文档字符串中间截断：原样返回进入该行时的状态，后续行不会全按字符串着色。
    highlighter.invalidate_from(0);
    let cut = format!("\"\"\"{}\"\"\"", "a".repeat(MAX_LINE_CHARS));
    let text = Rope::from_str(&format!("{cut}\nx\n"));
    assert_eq!(highlighter.state_at(&text, 1, 10), Some(LineState::Normal));
    assert_eq!(highlighter.state_near(&text, 1, 10), LineState::Normal);
}

mod measure {
    use ropey::Rope;

    use super::*;

    /// 测量从头推算 5 万行文件全部行首状态所需的时间。
    ///
    /// 运行方式为 `cargo test --release -p winc-syntax measure_cold_scan -- --ignored`。
    #[test]
    #[ignore]
    fn measure_cold_scan() {
        let text: String = (0..50_000)
            .map(|i| format!("    let value_{i} = compute(value_{i}, \"text {i}\"); // note {i}\n"))
            .collect();
        let rope = Rope::from_str(&text);
        let language = detect("a.rs");

        for _ in 0..5 {
            let mut highlighter = Highlighter::new(language);
            let start = std::time::Instant::now();
            let state = highlighter.state_at(&rope, usize::MAX, usize::MAX);
            eprintln!("5 万行：{:?}（{state:?}）", start.elapsed());
        }
    }
}
