//! SVG 属性值的基础语法：数、长度、颜色、变换、路径数据，以及 data URL 用到的 base64 和百分号解码。
//!
//! CSS 声明和选择器在 `css` 模块中解析。

use zeno::Command;

use super::{Aspect, PaintSpec};

/// 二维仿射变换，六个分量的含义与 SVG 的 `matrix(a b c d e f)` 相同。
///
/// 变换公式为 x' = a x + c y + e、y' = b x + d y + f。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Affine(pub(super) [f32; 6]);

impl Affine {
    /// 恒等变换。
    pub(super) const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// 创建平移 (x, y) 的变换。
    pub(super) fn translate(x: f32, y: f32) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// 创建 x、y 方向分别缩放 `x`、`y` 倍的变换。
    pub(super) fn scale(x: f32, y: f32) -> Affine {
        Affine([x, 0.0, 0.0, y, 0.0, 0.0])
    }

    /// 返回先应用 `self`、再应用 `then` 的复合变换。
    pub(super) fn then(self, then: Affine) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let [na, nb, nc, nd, ne, nf] = then.0;
        Affine([
            na * a + nc * b,
            nb * a + nd * b,
            na * c + nc * d,
            nb * c + nd * d,
            na * e + nc * f + ne,
            nb * e + nd * f + nf,
        ])
    }

    /// 返回点 (x, y) 经变换后的坐标。
    pub(super) fn apply(self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// 返回逆变换；行列式的绝对值小于 1e-12 时视为不可逆，返回 `None`。
    pub(super) fn invert(self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return None;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Affine([
            ia,
            ib,
            ic,
            id,
            -(ia * e + ic * f),
            -(ib * e + id * f),
        ]))
    }
}

/// 解析 `preserveAspectRatio` 属性；缺省时按 `xMidYMid meet` 处理。
pub(super) fn aspect(value: Option<&str>) -> Aspect {
    let mut words = value.unwrap_or("").split_whitespace();
    let mut align = words.next().unwrap_or("xMidYMid");
    if align == "defer" {
        align = words.next().unwrap_or("xMidYMid");
    }
    let slice = words.next() == Some("slice");
    let pick = |s: &str| {
        if s.contains("Min") {
            0.0
        } else if s.contains("Max") {
            1.0
        } else {
            0.5
        }
    };
    // 不能直接按第 4 个字节切开：属性值里有非 ASCII 字符时，第 4 个字节可能落在字符中间。
    let (x, y) = align.split_at_checked(4).unwrap_or((align, ""));
    Aspect {
        stretch: align == "none",
        align: (pick(x), pick(y)),
        slice,
    }
}

/// 返回把 `view_box` 放入 `w` × `h` 区域的变换，按 `aspect` 缩放和对齐。
///
/// `view_box` 为 `[x, y, 宽, 高]`；几何尺寸均为用户坐标单位。
pub(super) fn fit(view_box: [f32; 4], aspect: Aspect, w: f32, h: f32) -> Affine {
    let [vx, vy, vw, vh] = view_box;
    let (mut sx, mut sy) = (w / vw, h / vh);
    let (mut dx, mut dy) = (0.0, 0.0);
    if !aspect.stretch {
        let s = if aspect.slice { sx.max(sy) } else { sx.min(sy) };
        (sx, sy) = (s, s);
        dx = (w - vw * s) * aspect.align.0;
        dy = (h - vh * s) * aspect.align.1;
    }
    Affine::translate(-vx, -vy)
        .then(Affine::scale(sx, sy))
        .then(Affine::translate(dx, dy))
}

/// 向 `path` 追加圆角矩形；`rx` 或 `ry` 不大于 0 时追加直角矩形。
pub(super) fn rounded_rect(
    path: &mut Vec<Command>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rx: f32,
    ry: f32,
) {
    let p = |x: f32, y: f32| zeno::Point::new(x, y);
    if rx <= 0.0 || ry <= 0.0 {
        path.extend([
            Command::MoveTo(p(x, y)),
            Command::LineTo(p(x + w, y)),
            Command::LineTo(p(x + w, y + h)),
            Command::LineTo(p(x, y + h)),
            Command::Close,
        ]);
        return;
    }
    // 四分之一圆弧用三次贝塞尔近似。
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let (r, b) = (x + w, y + h);
    path.extend([
        Command::MoveTo(p(x + rx, y)),
        Command::LineTo(p(r - rx, y)),
        Command::CurveTo(p(r - rx + kx, y), p(r, y + ry - ky), p(r, y + ry)),
        Command::LineTo(p(r, b - ry)),
        Command::CurveTo(p(r, b - ry + ky), p(r - rx + kx, b), p(r - rx, b)),
        Command::LineTo(p(x + rx, b)),
        Command::CurveTo(p(x + rx - kx, b), p(x, b - ry + ky), p(x, b - ry)),
        Command::LineTo(p(x, y + ry)),
        Command::CurveTo(p(x, y + ry - ky), p(x + rx - kx, y), p(x + rx, y)),
        Command::Close,
    ]);
}

/// 返回把 `path` 的每个点（端点和控制点）用 `f` 映射之后的新路径。
pub(super) fn map_points(path: &[Command], f: impl Fn(zeno::Point) -> zeno::Point) -> Vec<Command> {
    path.iter().map(|c| map_command(*c, &f)).collect()
}

/// 把 `path` 的每个点（端点和控制点）原地用 `f` 映射。
pub(super) fn map_points_in_place(path: &mut [Command], f: impl Fn(zeno::Point) -> zeno::Point) {
    for c in path {
        *c = map_command(*c, &f);
    }
}

fn map_command(c: Command, f: impl Fn(zeno::Point) -> zeno::Point) -> Command {
    match c {
        Command::MoveTo(p) => Command::MoveTo(f(p)),
        Command::LineTo(p) => Command::LineTo(f(p)),
        Command::QuadTo(a, p) => Command::QuadTo(f(a), f(p)),
        Command::CurveTo(a, b, p) => Command::CurveTo(f(a), f(b), f(p)),
        Command::Close => Command::Close,
    }
}

/// 用三次贝塞尔曲线近似四分之一圆弧时，控制点到端点的距离与半径之比（无量纲）。
///
/// 值等于 4/3 · (√2 − 1)。
pub(super) const KAPPA: f32 = 0.552_284_8;

/// 向 `path` 追加以 (cx, cy) 为圆心、半径为 (rx, ry) 的椭圆。
pub(super) fn ellipse(path: &mut Vec<Command>, cx: f32, cy: f32, rx: f32, ry: f32) {
    rounded_rect(path, cx - rx, cy - ry, rx * 2.0, ry * 2.0, rx, ry);
}

/// 解析路径数据（`d` 属性），返回路径命令；圆弧转换为三次贝塞尔曲线。
///
/// 一个命令之后可以跟多组参数，其中 `M` 之后多出的坐标按 `L` 处理；数可以连写（`1-2`、`.5.5`）；
/// 圆弧的两个标志位之间可以不加分隔（`a1 1 0 011 1`）。
///
/// 遇到语法错误时保留出错位置之前已解析的部分；路径不以 `M` 开头时返回空路径。
pub(super) fn path_data(d: &str) -> Vec<Command> {
    let p = |x: f32, y: f32| zeno::Point::new(x, y);
    let mut out = Vec::new();
    let mut reader = PathReader {
        bytes: d.as_bytes(),
        at: 0,
    };
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    // 上一段曲线的控制点，`S`、`T` 命令据此求对称点。
    let mut last_cubic: Option<(f32, f32)> = None;
    let mut last_quad: Option<(f32, f32)> = None;
    let mut command = 0u8;
    let mut open = false;
    loop {
        reader.skip_separators();
        let Some(&c) = reader.bytes.get(reader.at) else {
            break;
        };
        if c.is_ascii_alphabetic() {
            command = c;
            reader.at += 1;
        } else if command == 0 || matches!(command, b'Z' | b'z') {
            // 开头不是命令，或 `Z` 之后直接跟数字：语法错误。
            break;
        }
        let relative = command.is_ascii_lowercase();
        let (ox, oy) = if relative { (cx, cy) } else { (0.0, 0.0) };
        let upper = command.to_ascii_uppercase();
        let mut cubic = None;
        let mut quad = None;
        let ok = (|| -> Option<()> {
            match upper {
                b'M' => {
                    let (x, y) = (reader.number()? + ox, reader.number()? + oy);
                    out.push(Command::MoveTo(p(x, y)));
                    (cx, cy, sx, sy) = (x, y, x, y);
                    open = true;
                    // `M` 之后多出的坐标按 `L`（相对坐标时为 `l`）处理。
                    command = if relative { b'l' } else { b'L' };
                }
                b'L' => {
                    let (x, y) = (reader.number()? + ox, reader.number()? + oy);
                    out.push(Command::LineTo(p(x, y)));
                    (cx, cy) = (x, y);
                }
                b'H' => {
                    cx = reader.number()? + ox;
                    out.push(Command::LineTo(p(cx, cy)));
                }
                b'V' => {
                    cy = reader.number()? + oy;
                    out.push(Command::LineTo(p(cx, cy)));
                }
                b'C' => {
                    let (x1, y1) = (reader.number()? + ox, reader.number()? + oy);
                    let (x2, y2) = (reader.number()? + ox, reader.number()? + oy);
                    let (x, y) = (reader.number()? + ox, reader.number()? + oy);
                    out.push(Command::CurveTo(p(x1, y1), p(x2, y2), p(x, y)));
                    cubic = Some((x2, y2));
                    (cx, cy) = (x, y);
                }
                b'S' => {
                    let (x1, y1) =
                        last_cubic.map_or((cx, cy), |(lx, ly)| (2.0 * cx - lx, 2.0 * cy - ly));
                    let (x2, y2) = (reader.number()? + ox, reader.number()? + oy);
                    let (x, y) = (reader.number()? + ox, reader.number()? + oy);
                    out.push(Command::CurveTo(p(x1, y1), p(x2, y2), p(x, y)));
                    cubic = Some((x2, y2));
                    (cx, cy) = (x, y);
                }
                b'Q' => {
                    let (x1, y1) = (reader.number()? + ox, reader.number()? + oy);
                    let (x, y) = (reader.number()? + ox, reader.number()? + oy);
                    out.push(Command::QuadTo(p(x1, y1), p(x, y)));
                    quad = Some((x1, y1));
                    (cx, cy) = (x, y);
                }
                b'T' => {
                    let (x1, y1) =
                        last_quad.map_or((cx, cy), |(lx, ly)| (2.0 * cx - lx, 2.0 * cy - ly));
                    let (x, y) = (reader.number()? + ox, reader.number()? + oy);
                    out.push(Command::QuadTo(p(x1, y1), p(x, y)));
                    quad = Some((x1, y1));
                    (cx, cy) = (x, y);
                }
                b'A' => {
                    let (rx, ry) = (reader.number()?, reader.number()?);
                    let angle = reader.number()?;
                    let (large, sweep) = (reader.flag()?, reader.flag()?);
                    let (x, y) = (reader.number()? + ox, reader.number()? + oy);
                    arc(&mut out, (cx, cy), (rx, ry), angle, large, sweep, (x, y));
                    (cx, cy) = (x, y);
                }
                b'Z' => {
                    if open {
                        out.push(Command::Close);
                    }
                    (cx, cy) = (sx, sy);
                }
                _ => return None,
            }
            Some(())
        })();
        if ok.is_none() {
            break;
        }
        // 没有 `M` 就开始绘制：语法错误，丢弃已解析的内容。
        if !open && upper != b'M' {
            out.clear();
            break;
        }
        last_cubic = cubic;
        last_quad = quad;
    }
    out
}

/// 路径数据的字节读取器。
pub(super) struct PathReader<'a> {
    bytes: &'a [u8],
    /// 当前读取位置（字节偏移，从 0 起）。
    at: usize,
}

impl PathReader<'_> {
    /// 跳过空白和逗号。
    pub(super) fn skip_separators(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|c| c.is_ascii_whitespace() || *c == b',')
        {
            self.at += 1;
        }
    }

    /// 读取下一个数；格式不对或结果不是有限值时返回 `None`。
    pub(super) fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let bytes = &self.bytes[self.at..];
        let len = number_end(bytes);
        let text = std::str::from_utf8(&bytes[..len]).ok()?;
        let n: f32 = text.parse().ok()?;
        self.at += len;
        n.is_finite().then_some(n)
    }

    /// 读取圆弧的标志位（单个字符 `0` 或 `1`），其后可以紧跟下一个数；其他字符返回 `None`。
    pub(super) fn flag(&mut self) -> Option<bool> {
        self.skip_separators();
        let c = *self.bytes.get(self.at)?;
        self.at += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
}

/// 把 SVG 的圆弧转换为若干段三次贝塞尔曲线，每段不超过 90°，追加到 `out`。
///
/// 圆弧按终点参数化：`(x1, y1)` 是起点，`(rx, ry)` 是半径，`angle` 是 x 轴旋转角（度），
/// `large` 是大弧标志，`sweep` 是扫掠方向标志，`(x2, y2)` 是终点。起点与终点重合时不追加任何
/// 命令；任一半径为 0 时追加一条直线。算法依据 SVG 1.1 附录 F.6。
pub(super) fn arc(
    out: &mut Vec<Command>,
    (x1, y1): (f32, f32),
    (rx, ry): (f32, f32),
    angle: f32,
    large: bool,
    sweep: bool,
    (x2, y2): (f32, f32),
) {
    let p = |x: f32, y: f32| zeno::Point::new(x, y);
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if (x1 - x2).abs() < 1e-9 && (y1 - y2).abs() < 1e-9 {
        return;
    }
    if rx < 1e-9 || ry < 1e-9 {
        out.push(Command::LineTo(p(x2, y2)));
        return;
    }
    let (sin, cos) = angle.to_radians().sin_cos();
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (x1p, y1p) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    // 半径过小、无法连接起点和终点时，按比例放大到恰好能连接（SVG 1.1 附录 F.6.6）。
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let k = lambda.sqrt();
        rx *= k;
        ry *= k;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den > 0.0 {
        (num / den).max(0.0).sqrt()
    } else {
        0.0
    };
    if large == sweep {
        coef = -coef;
    }
    let (cxp, cyp) = (coef * rx * y1p / ry, -coef * ry * x1p / rx);
    let (cx, cy) = (
        cos * cxp - sin * cyp + (x1 + x2) / 2.0,
        sin * cxp + cos * cyp + (y1 + y2) / 2.0,
    );
    let vector_angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 { -a } else { a }
    };
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let start = vector_angle(1.0, 0.0, ux, uy);
    let mut delta = vector_angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let segments = (delta.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = delta / segments as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let point = |t: f32| {
        let (s, c) = t.sin_cos();
        let (ex, ey) = (rx * c, ry * s);
        (cos * ex - sin * ey + cx, sin * ex + cos * ey + cy)
    };
    let derivative = |t: f32| {
        let (s, c) = t.sin_cos();
        let (ex, ey) = (-rx * s, ry * c);
        (cos * ex - sin * ey, sin * ex + cos * ey)
    };
    for i in 0..segments {
        let (t0, t1) = (start + step * i as f32, start + step * (i + 1) as f32);
        let (p0, p3) = (point(t0), point(t1));
        let (d0, d1) = (derivative(t0), derivative(t1));
        let end = if i + 1 == segments { (x2, y2) } else { p3 };
        out.push(Command::CurveTo(
            p(p0.0 + k * d0.0, p0.1 + k * d0.1),
            p(p3.0 - k * d1.0, p3.1 - k * d1.1),
            p(end.0, end.1),
        ));
    }
}

/// 解析以空白或逗号分隔的一串数，也接受 `1-2`、`.5.5` 形式的连写；无法解析的字符被跳过。
///
/// 每个数的范围由 `number_end` 确定，与 `length` 和路径数据的规则相同。
pub(super) fn numbers(text: &str) -> Vec<f32> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() || bytes[i] == b',' {
            i += 1;
            continue;
        }
        // `number_end` 只越过 ASCII 字节，`i + len` 一定落在字符边界上。
        let len = number_end(&bytes[i..]);
        match text[i..i + len].parse() {
            Ok(n) => {
                out.push(n);
                i += len;
            }
            // 一个数都读不出时跳过整个字符，否则 `i` 不前进，循环不会结束。
            _ => i += text[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out
}

/// 返回 `bytes` 开头的数占用的长度（字节）。
///
/// 数由正负号、数字和小数点组成；`e` 之后跟着数字时才算指数，`1em` 中的 `e` 属于单位。
pub(super) fn number_end(bytes: &[u8]) -> usize {
    let mut i = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let mut dot = false;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || (bytes[i] == b'.' && !dot)) {
        dot |= bytes[i] == b'.';
        i += 1;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            i = j;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    i
}

/// 解析长度，返回用户坐标单位（px）下的值；无法解析时返回 `None`。
///
/// 百分比相对 `reference`；绝对单位按 CSS 的换算，96 px = 1 in，即 `pt` 为 4/3 px、`pc` 为 16 px，
/// `mm`、`cm` 按 25.4 mm = 1 in 换算。`em`、`rem` 按 16 px、`ex` 按 8 px 估算，为经验值，
/// 依据是这里没有字体信息。
pub(super) fn length(text: &str, reference: f32) -> Option<f32> {
    let text = text.trim();
    let split = number_end(text.as_bytes());
    let (number, unit) = text.split_at(split);
    let n: f32 = number.trim().parse().ok()?;
    let scale = match unit.trim() {
        "" | "px" => 1.0,
        "%" => reference / 100.0,
        "pt" => 4.0 / 3.0,
        "pc" => 16.0,
        "mm" => 96.0 / 25.4,
        "cm" => 96.0 / 2.54,
        "in" => 96.0,
        "em" | "rem" => 16.0,
        "ex" => 8.0,
        _ => return None,
    };
    Some(n * scale).filter(|v| v.is_finite())
}

/// 解析不透明度（数或百分比），结果限制在 0 到 1；无法解析时按 1 处理。
pub(super) fn opacity(text: &str) -> f32 {
    let text = text.trim();
    let value = match text.strip_suffix('%') {
        Some(p) => p.trim().parse::<f32>().map(|p| p / 100.0),
        None => text.parse::<f32>(),
    };
    value.unwrap_or(1.0).clamp(0.0, 1.0)
}

/// 解析 `fill`、`stroke` 的值，可以是 `none`、`currentColor`、`url(#id)`（可带回退值）或颜色。
///
/// 无法解析时返回 `None`。
pub(super) fn paint(text: &str) -> Option<PaintSpec> {
    let text = text.trim();
    if text == "none" {
        return Some(PaintSpec::None);
    }
    if text == "currentColor" {
        return Some(PaintSpec::CurrentColor);
    }
    if let Some(rest) = text.strip_prefix("url(") {
        let (inside, after) = rest.split_once(')')?;
        let id = inside
            .trim()
            .trim_matches(['"', '\''])
            .strip_prefix('#')?
            .to_owned();
        let fallback = match after.trim() {
            "" => PaintSpec::None,
            f => paint(f).unwrap_or(PaintSpec::None),
        };
        return Some(PaintSpec::Url(id, Box::new(fallback)));
    }
    color(text).map(PaintSpec::Color)
}

/// 解析 CSS 颜色，返回非预乘的 `[r, g, b, a]`；无法解析时返回 `None`。
///
/// 支持 `#rgb`、`#rgba`、`#rrggbb`、`#rrggbbaa`、`rgb()`、`rgba()`、`hsl()`、`hsla()`、
/// `transparent` 和颜色名（见 [`NAMED`]）。`light-dark()` 取浅色的一项，`var()` 取回退值。
pub(super) fn color(text: &str) -> Option<[u8; 4]> {
    let text = text.trim();
    for (function, item) in [("light-dark(", 0), ("var(", 1)] {
        let lower = text.get(..function.len()).map(str::to_ascii_lowercase);
        if lower.as_deref() == Some(function) {
            let inside = text[function.len()..].strip_suffix(')')?;
            return color(arguments(inside).nth(item)?);
        }
    }
    if let Some(hex) = text.strip_prefix('#') {
        // 下面按字节切片：含非 ASCII 字符时，字节偏移可能落在字符中间，切片会 panic。
        if !hex.is_ascii() {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|d| d * 17);
        let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            3 => Some([digit(0)?, digit(1)?, digit(2)?, 255]),
            4 => Some([digit(0)?, digit(1)?, digit(2)?, digit(3)?]),
            6 => Some([pair(0)?, pair(2)?, pair(4)?, 255]),
            8 => Some([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
            _ => None,
        };
    }
    let lower = text.to_ascii_lowercase();
    if let Some((name, args)) = lower.split_once('(') {
        let args = args.strip_suffix(')')?;
        let parts: Vec<&str> = args
            .split([',', ' ', '/'])
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() < 3 {
            return None;
        }
        let alpha = parts
            .get(3)
            .map_or(255, |a| (opacity(a) * 255.0).round() as u8);
        let channel = |s: &str| -> Option<u8> {
            let v = match s.strip_suffix('%') {
                Some(p) => p.parse::<f32>().ok()? * 2.55,
                None => s.parse::<f32>().ok()?,
            };
            Some(v.round().clamp(0.0, 255.0) as u8)
        };
        return match name.trim() {
            "rgb" | "rgba" => Some([
                channel(parts[0])?,
                channel(parts[1])?,
                channel(parts[2])?,
                alpha,
            ]),
            "hsl" | "hsla" => {
                let h = parts[0].trim_end_matches("deg").parse::<f32>().ok()?;
                let pct = |s: &str| {
                    s.trim_end_matches('%')
                        .parse::<f32>()
                        .ok()
                        .map(|v| v / 100.0)
                };
                let [r, g, b] = hsl(h, pct(parts[1])?, pct(parts[2])?);
                Some([r, g, b, alpha])
            }
            _ => None,
        };
    }
    if lower == "transparent" {
        return Some([0, 0, 0, 0]);
    }
    let rgb = NAMED
        .binary_search_by(|(n, _)| n.cmp(&lower.as_str()))
        .ok()
        .map(|i| NAMED[i].1)?;
    Some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255])
}

/// 按括号之外的逗号拆分函数的参数 `text`。
fn arguments(text: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0usize;
    text.split(move |c| {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        c == ',' && depth == 0
    })
}

/// 把 HSL 颜色转换为 RGB。
///
/// 色相 `h` 以度为单位，允许超出 360；饱和度 `s` 和亮度 `l` 取 0 到 1，超出范围时截断。
pub(super) fn hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// 解析 `transform` 属性，返回复合变换。
///
/// 支持 `matrix`、`translate`、`scale`、`rotate`、`skewX`、`skewY`，可以连续写多个；
/// 无法识别的函数被忽略。
pub(super) fn transform(text: &str) -> Affine {
    let mut result = Affine::IDENTITY;
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let args = numbers(&rest[open + 1..open + close]);
        rest = &rest[open + close + 1..];
        let arg = |i: usize, default: f32| args.get(i).copied().unwrap_or(default);
        let step = match name {
            "matrix" if args.len() == 6 => {
                Affine([args[0], args[1], args[2], args[3], args[4], args[5]])
            }
            "translate" => Affine::translate(arg(0, 0.0), arg(1, 0.0)),
            "scale" => Affine::scale(arg(0, 1.0), arg(1, arg(0, 1.0))),
            "rotate" => {
                let (sin, cos) = arg(0, 0.0).to_radians().sin_cos();
                let (cx, cy) = (arg(1, 0.0), arg(2, 0.0));
                Affine::translate(-cx, -cy)
                    .then(Affine([cos, sin, -sin, cos, 0.0, 0.0]))
                    .then(Affine::translate(cx, cy))
            }
            "skewX" => Affine([1.0, 0.0, arg(0, 0.0).to_radians().tan(), 1.0, 0.0, 0.0]),
            "skewY" => Affine([1.0, arg(0, 0.0).to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => continue,
        };
        // 先写的变换在外层：后写的变换先作用于坐标。
        result = step.then(result);
    }
    result
}

/// CSS 颜色名及其 RGB 值（`0xRRGGBB`，不含透明度）的表。
///
/// 按名字升序排列，查找时对名字做二分。
pub(super) const NAMED: &[(&str, u32)] = &[
    ("aliceblue", 0xF0F8FF),
    ("antiquewhite", 0xFAEBD7),
    ("aqua", 0x00FFFF),
    ("aquamarine", 0x7FFFD4),
    ("azure", 0xF0FFFF),
    ("beige", 0xF5F5DC),
    ("bisque", 0xFFE4C4),
    ("black", 0x000000),
    ("blanchedalmond", 0xFFEBCD),
    ("blue", 0x0000FF),
    ("blueviolet", 0x8A2BE2),
    ("brown", 0xA52A2A),
    ("burlywood", 0xDEB887),
    ("cadetblue", 0x5F9EA0),
    ("chartreuse", 0x7FFF00),
    ("chocolate", 0xD2691E),
    ("coral", 0xFF7F50),
    ("cornflowerblue", 0x6495ED),
    ("cornsilk", 0xFFF8DC),
    ("crimson", 0xDC143C),
    ("cyan", 0x00FFFF),
    ("darkblue", 0x00008B),
    ("darkcyan", 0x008B8B),
    ("darkgoldenrod", 0xB8860B),
    ("darkgray", 0xA9A9A9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xA9A9A9),
    ("darkkhaki", 0xBDB76B),
    ("darkmagenta", 0x8B008B),
    ("darkolivegreen", 0x556B2F),
    ("darkorange", 0xFF8C00),
    ("darkorchid", 0x9932CC),
    ("darkred", 0x8B0000),
    ("darksalmon", 0xE9967A),
    ("darkseagreen", 0x8FBC8F),
    ("darkslateblue", 0x483D8B),
    ("darkslategray", 0x2F4F4F),
    ("darkslategrey", 0x2F4F4F),
    ("darkturquoise", 0x00CED1),
    ("darkviolet", 0x9400D3),
    ("deeppink", 0xFF1493),
    ("deepskyblue", 0x00BFFF),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1E90FF),
    ("firebrick", 0xB22222),
    ("floralwhite", 0xFFFAF0),
    ("forestgreen", 0x228B22),
    ("fuchsia", 0xFF00FF),
    ("gainsboro", 0xDCDCDC),
    ("ghostwhite", 0xF8F8FF),
    ("gold", 0xFFD700),
    ("goldenrod", 0xDAA520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xADFF2F),
    ("grey", 0x808080),
    ("honeydew", 0xF0FFF0),
    ("hotpink", 0xFF69B4),
    ("indianred", 0xCD5C5C),
    ("indigo", 0x4B0082),
    ("ivory", 0xFFFFF0),
    ("khaki", 0xF0E68C),
    ("lavender", 0xE6E6FA),
    ("lavenderblush", 0xFFF0F5),
    ("lawngreen", 0x7CFC00),
    ("lemonchiffon", 0xFFFACD),
    ("lightblue", 0xADD8E6),
    ("lightcoral", 0xF08080),
    ("lightcyan", 0xE0FFFF),
    ("lightgoldenrodyellow", 0xFAFAD2),
    ("lightgray", 0xD3D3D3),
    ("lightgreen", 0x90EE90),
    ("lightgrey", 0xD3D3D3),
    ("lightpink", 0xFFB6C1),
    ("lightsalmon", 0xFFA07A),
    ("lightseagreen", 0x20B2AA),
    ("lightskyblue", 0x87CEFA),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xB0C4DE),
    ("lightyellow", 0xFFFFE0),
    ("lime", 0x00FF00),
    ("limegreen", 0x32CD32),
    ("linen", 0xFAF0E6),
    ("magenta", 0xFF00FF),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66CDAA),
    ("mediumblue", 0x0000CD),
    ("mediumorchid", 0xBA55D3),
    ("mediumpurple", 0x9370DB),
    ("mediumseagreen", 0x3CB371),
    ("mediumslateblue", 0x7B68EE),
    ("mediumspringgreen", 0x00FA9A),
    ("mediumturquoise", 0x48D1CC),
    ("mediumvioletred", 0xC71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xF5FFFA),
    ("mistyrose", 0xFFE4E1),
    ("moccasin", 0xFFE4B5),
    ("navajowhite", 0xFFDEAD),
    ("navy", 0x000080),
    ("oldlace", 0xFDF5E6),
    ("olive", 0x808000),
    ("olivedrab", 0x6B8E23),
    ("orange", 0xFFA500),
    ("orangered", 0xFF4500),
    ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA),
    ("palegreen", 0x98FB98),
    ("paleturquoise", 0xAFEEEE),
    ("palevioletred", 0xDB7093),
    ("papayawhip", 0xFFEFD5),
    ("peachpuff", 0xFFDAB9),
    ("peru", 0xCD853F),
    ("pink", 0xFFC0CB),
    ("plum", 0xDDA0DD),
    ("powderblue", 0xB0E0E6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xFF0000),
    ("rosybrown", 0xBC8F8F),
    ("royalblue", 0x4169E1),
    ("saddlebrown", 0x8B4513),
    ("salmon", 0xFA8072),
    ("sandybrown", 0xF4A460),
    ("seagreen", 0x2E8B57),
    ("seashell", 0xFFF5EE),
    ("sienna", 0xA0522D),
    ("silver", 0xC0C0C0),
    ("skyblue", 0x87CEEB),
    ("slateblue", 0x6A5ACD),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xFFFAFA),
    ("springgreen", 0x00FF7F),
    ("steelblue", 0x4682B4),
    ("tan", 0xD2B48C),
    ("teal", 0x008080),
    ("thistle", 0xD8BFD8),
    ("tomato", 0xFF6347),
    ("turquoise", 0x40E0D0),
    ("violet", 0xEE82EE),
    ("wheat", 0xF5DEB3),
    ("white", 0xFFFFFF),
    ("whitesmoke", 0xF5F5F5),
    ("yellow", 0xFFFF00),
    ("yellowgreen", 0x9ACD32),
];

/// 解码 data URL 中的 base64 文本，忽略其中的空白；遇到 `=` 即结束，遇到非法字符返回 `None`。
///
/// 同时接受标准字母表（`+`、`/`）和 URL 安全字母表（`-`、`_`）。
pub(super) fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in text.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// 解码百分号转义（如 `%20`），非法的转义原样保留。
pub(super) fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}
