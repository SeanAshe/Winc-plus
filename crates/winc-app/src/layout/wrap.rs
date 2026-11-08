//! 按宽度折行：对话框、提示、提交信息和设置说明共用。

/// 把一段文字按宽度 `width`（逻辑像素）折成几行，返回各行的文字。
///
/// 中英文都按字符折：连着的英文字母、数字和符号算一个词，行与行之间从词边界断开，一个词
/// 比一行还长时才按字拆开。
///
/// `measure` 返回一段文字的宽度（逻辑像素）。
pub fn wrap_text(text: &str, width: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<&str> {
    // 先切成不能拆开的小段：连着的英文字母、数字和符号是一个词，空格单独一段，中文按字。
    // 一行放不下时在段与段之间换行；一个词比一行还长时才按字拆开。
    // 界面字体不做字距调整：一段文字的宽度就是各段宽度之和，每段只量一次，累加出当前行的宽度。
    let mut lines = Vec::new();
    // 当前行是 `text[start..end]`，宽 `w`。
    let (mut start, mut end, mut w) = (0, 0, 0.0);
    let mut pieces = text.char_indices().peekable();
    while let Some((from, c)) = pieces.next() {
        let mut to = from + c.len_utf8();
        if c.is_ascii_graphic() {
            while let Some(&(i, next)) = pieces.peek().filter(|(_, n)| n.is_ascii_graphic()) {
                to = i + next.len_utf8();
                pieces.next();
            }
        }
        let piece = &text[from..to];
        let piece_w = measure(piece);
        if w + piece_w <= width || start == end {
            end = to;
            w += piece_w;
        } else {
            lines.push(text[start..end].trim_end());
            let rest = piece.trim_start();
            (start, end) = (to - rest.len(), to);
            w = if rest.is_empty() { 0.0 } else { piece_w };
        }
        // 一个词本身就比一行长：按字拆，每行放下尽量多的字。
        while w > width && text[start..end].chars().nth(1).is_some() {
            let (mut head, mut head_w) = (start, 0.0);
            for (i, c) in text[start..end].char_indices() {
                let c_w = measure(c.encode_utf8(&mut [0; 4]));
                if head_w + c_w > width {
                    break;
                }
                head = start + i + c.len_utf8();
                head_w += c_w;
            }
            if head == start {
                break;
            }
            lines.push(&text[start..head]);
            start = head;
            w -= head_w;
        }
    }
    lines.push(&text[start..end]);
    lines
}
