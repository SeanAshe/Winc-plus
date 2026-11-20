//! 把其他格式的文档（docx、EPUB）转换成 Markdown 时共用的转义与判断。

/// 把 `text` 作为 Markdown 正文写进 `out`：ASCII 标点前加反斜杠，原样显示，不被当作标记。
///
/// `CommonMark` 规定反斜杠后的任意 ASCII 标点都表示该字符本身，因此逐个转义即可，不必判断
/// 标点在上下文中是否构成标记。
pub(crate) fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
}

/// 返回可以放进 `<…>` 链接地址中的 `url`：去掉换行，`<`、`>` 和空格按百分号转义。
pub(crate) fn link_target(url: &str) -> String {
    url.chars()
        .filter(|c| *c != '\n' && *c != '\r')
        .map(|c| match c {
            '<' => "%3C".to_owned(),
            '>' => "%3E".to_owned(),
            ' ' => "%20".to_owned(),
            c => c.to_string(),
        })
        .collect()
}

/// 返回写在 Markdown 链接、图片 `(…)` 中的地址 `url`；含空白、括号或尖括号时用 `<…>` 包裹。
pub(crate) fn destination(url: &str) -> String {
    let url = url.trim();
    if url.contains(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>')) {
        format!("<{}>", link_target(url))
    } else {
        url.to_owned()
    }
}

/// 返回可以放进 HTML 属性值（双引号）中的 `text`。
pub(crate) fn html_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 扩展名为 `ext`（小写）的图片是否能在预览中显示。
pub(crate) fn previewable_image(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "jpe" | "gif" | "webp" | "bmp" | "ico" | "svg"
    )
}
