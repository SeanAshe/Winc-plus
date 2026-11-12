//! 文件和目录的图标：按扩展名区分的文件图标（有的画成字母）、目录、新建文件、文档。

use super::*;

/// 文件图标纸面上的记号，表示文件的大类；具体语言靠颜色区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileGlyph {
    /// 源代码：一对尖括号。
    Code,
    /// 文档和日志：一张折角的纸，纸上几行字。
    Text,
    /// 配置和数据：「键 值」两行。
    Data,
    /// 脚本：提示符 >_。
    Shell,
    /// 图片：山和太阳。
    Image,
    /// 压缩包：拉链。
    Archive,
    /// 音频：音符。
    Audio,
    /// 视频：圆角框里一个播放用的三角形。
    Video,
    /// 字体：一个 A。
    Font,
    /// 程序、库这些二进制文件：一张折角的纸，纸上两行点。
    Binary,
    /// 锁文件、证书、密钥：一把锁。
    Lock,
    /// JSON：一对花括号 { }。
    Json,
    /// 配置（TOML、YAML、INI、.editorconfig、各种 rc）：一个齿轮，按格式配色。
    Config,
    /// Markdown：一个井号 #（标题）。
    Markdown,
    /// Docker（Dockerfile、compose、.dockerignore）：叠起来的集装箱。
    Docker,
    /// Git（.gitignore、.gitattributes、.gitmodules）：分支图。
    Git,
    /// 许可证：一枚带两条飘带的徽章。
    License,
    /// README：一个 i（说明）。
    Info,
    /// 环境变量、密钥配置（.env）：一把钥匙。
    Key,
    /// 包的清单（package.json、Cargo.toml、go.mod、pyproject.toml）：一个立方体的包裹。
    Package,
    /// 表格（csv、xlsx）：带格线的框。
    Table,
    /// 数据库和 SQL：一个圆柱。
    Database,
    /// 编程语言的缩写，用线条画的两三个字母（TS、ETS、JS、PY、GO、RS、C、C++、C#、KT、RB、SW），
    /// 扫描一列文件时可直接分辨语言；没有缩写的语言画 `Code`。
    Letters(&'static str),
    /// Java：一杯冒着热气的咖啡。
    Java,
    /// 未识别的类型：空白的纸。
    Plain,
}

impl FileGlyph {
    /// 全部记号（包括 `file_kind` 用到的每一个缩写）：目录菜单按它预先画好每种图标，漏掉的记号
    /// 在菜单里没有图标。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub const ALL: [FileGlyph; 36] = [
        FileGlyph::Code,
        FileGlyph::Text,
        FileGlyph::Data,
        FileGlyph::Shell,
        FileGlyph::Image,
        FileGlyph::Archive,
        FileGlyph::Audio,
        FileGlyph::Video,
        FileGlyph::Font,
        FileGlyph::Binary,
        FileGlyph::Lock,
        FileGlyph::Json,
        FileGlyph::Config,
        FileGlyph::Markdown,
        FileGlyph::Docker,
        FileGlyph::Git,
        FileGlyph::License,
        FileGlyph::Info,
        FileGlyph::Key,
        FileGlyph::Package,
        FileGlyph::Table,
        FileGlyph::Database,
        FileGlyph::Letters("TS"),
        FileGlyph::Letters("ETS"),
        FileGlyph::Letters("JS"),
        FileGlyph::Letters("PY"),
        FileGlyph::Letters("GO"),
        FileGlyph::Letters("RS"),
        FileGlyph::Letters("C"),
        FileGlyph::Letters("C++"),
        FileGlyph::Letters("C#"),
        FileGlyph::Letters("KT"),
        FileGlyph::Letters("RB"),
        FileGlyph::Letters("SW"),
        FileGlyph::Java,
        FileGlyph::Plain,
    ];
}

/// 绘制目录图标，15×12：线条风格，笔宽 `STROKE`，与顶栏图标一致。
///
/// 左上一个标签，下面是圆角的
/// 夹子。
///
/// 打开时前片向前倾倒：后片的右边只画到前片上沿，前片是一个上宽下窄的平行四边形，左下角和
/// 后片合在一起。
pub fn folder(frame: &mut Frame<'_>, (x, y): (f32, f32), open: bool, color: Color, clip: Rect) {
    let h = STROKE / 2.0;
    // 线画在框里：外沿正好是 15×12。
    let (l, t, r, b) = (x + h, y + h, x + 15.0 - h, y + 12.0 - h);
    let c = 2.0;
    let tab_end = l + 5.0;
    let lid = t + 2.0;
    let mut shapes: Strokes<12> = Strokes::new();
    shapes.extend([
        // 左边、左上角、标签（上沿到斜口）。
        Stroke::Line(l, b - c, l, t + c),
        Stroke::Arc(l + c, t + c, c, -PI, -FRAC_PI_2),
        Stroke::Line(l + c, t, tab_end, t),
        Stroke::Line(tab_end, t, tab_end + 1.5, lid),
        Stroke::Line(tab_end + 1.5, lid, r - c, lid),
        Stroke::Arc(r - c, lid + c, c, -FRAC_PI_2, 0.0),
    ]);
    if open {
        // 后片的右边只到前片上沿；前片上沿往右伸出去一点，右边斜着收到底。
        let front = lid + 3.0;
        shapes.extend([
            Stroke::Line(r, lid + c, r, front),
            Stroke::Line(l + 2.75, front, r + 0.25, front),
            Stroke::Line(r + 0.25, front, r - 2.0, b),
            Stroke::Line(r - 2.0, b, l + c, b),
            Stroke::Arc(l + c, b - c, c, FRAC_PI_2, PI),
            Stroke::Line(l + 2.75, front, l + 0.4, b - 0.75),
        ]);
    } else {
        shapes.extend([
            Stroke::Line(r, lid + c, r, b - c),
            Stroke::Arc(r - c, b - c, c, 0.0, FRAC_PI_2),
            Stroke::Line(r - c, b, l + c, b),
            Stroke::Arc(l + c, b - c, c, FRAC_PI_2, PI),
        ]);
    }
    frame.stroke(shapes.as_slice(), STROKE, color, clip);
}

/// 绘制小号目录图标，12×10，以 `(cx, cy)` 为中心：形状和合上的 [`folder`] 相同，线条和小号分支
/// 图标一样细，用于侧栏小号文字旁。
pub fn folder_small(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    const WIDTH: f32 = 1.25;
    let h = WIDTH / 2.0;
    let (l, t, r, b) = (cx - 6.0 + h, cy - 5.0 + h, cx + 6.0 - h, cy + 5.0 - h);
    let c = 1.5;
    let tab_end = l + 4.0;
    let lid = t + 1.5;
    let shapes = [
        Stroke::Line(l, b - c, l, t + c),
        Stroke::Arc(l + c, t + c, c, -PI, -FRAC_PI_2),
        Stroke::Line(l + c, t, tab_end, t),
        Stroke::Line(tab_end, t, tab_end + 1.25, lid),
        Stroke::Line(tab_end + 1.25, lid, r - c, lid),
        Stroke::Arc(r - c, lid + c, c, -FRAC_PI_2, 0.0),
        Stroke::Line(r, lid + c, r, b - c),
        Stroke::Arc(r - c, b - c, c, 0.0, FRAC_PI_2),
        Stroke::Line(r - c, b, l + c, b),
        Stroke::Arc(l + c, b - c, c, FRAC_PI_2, PI),
    ];
    frame.stroke(&shapes, WIDTH, color, clip);
}

/// 绘制「新建文件」图标，占 12×15 的位置（左上角 `(x, y)`）：和文件树工具栏的 [`add_file`] 是
/// 同一个图标，居中放在这块位置。
///
/// 开始页、空编辑区用它，与工具栏上的「新建文件」保持一致。
pub fn new_file(frame: &mut Frame<'_>, (x, y): (f32, f32), color: Color, clip: Rect) {
    add_file(frame, (x + 6.0, y + 7.5), color, clip);
}

/// 绘制描边的文档图标，大小为 `w`×`h`：右上角折起，其余三个角是小圆角。
///
/// 用于无法预览的文件的占位页。
pub fn document(
    frame: &mut Frame<'_>,
    (x, y): (f32, f32),
    (w, h): (f32, f32),
    color: Color,
    clip: Rect,
) {
    let (r, fold) = (3.0, w * 0.3);
    let (right, bottom) = (x + w, y + h);
    let outline = [
        Stroke::Arc(x + r, y + r, r, -PI, -FRAC_PI_2),
        Stroke::Line(x + r, y, right - fold, y),
        Stroke::Line(right - fold, y, right, y + fold),
        Stroke::Line(right, y + fold, right, bottom - r),
        Stroke::Arc(right - r, bottom - r, r, 0.0, FRAC_PI_2),
        Stroke::Line(right - r, bottom, x + r, bottom),
        Stroke::Arc(x + r, bottom - r, r, FRAC_PI_2, PI),
        Stroke::Line(x, bottom - r, x, y + r),
        // 折角
        Stroke::Line(right - fold, y, right - fold, y + fold - r),
        Stroke::Arc(right - fold + r, y + fold - r, r, FRAC_PI_2, PI),
        Stroke::Line(right - fold + r, y + fold, right, y + fold),
    ];
    frame.stroke(&outline, STROKE, color, clip);
}

/// 绘制文件图标，占 12×15 的位置：线条风格，只画表示类型的记号，约 11 像素见方，与目录图标
/// 一样大，笔宽 `STROKE`；文档类（`Text`、`Binary`、`Plain`）在记号外加一张折角的纸。
///
/// 记号就是这个图标的全部内容，不套纸的轮廓，否则一列文件里只剩一排相同的灰框，类型反而看不清。
pub fn file(frame: &mut Frame<'_>, (x, y): (f32, f32), glyph: FileGlyph, color: Color, clip: Rect) {
    // 记号居中在 12×15 这块位置里。
    let (cx, cy) = (x + FILE_WIDTH / 2.0, y + 7.5);
    let k = MARK_SCALE;
    let line = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Stroke::Line(cx + x0 * k, cy + y0 * k, cx + x1 * k, cy + y1 * k)
    };
    // 文档类（文字、二进制、没认出来的）外面是一张折角的纸，约 10×13。
    let page = |frame: &mut Frame<'_>| {
        // 和侧栏开关一样高（12），一行里的图标上下沿齐平。
        let (w, h) = (9.5, 12.0);
        document(frame, (cx - w / 2.0, cy - h / 2.0), (w, h), color, clip);
    };

    match glyph {
        FileGlyph::Code => {
            let marks = [
                line(-1.25, -2.5, -3.5, 0.0),
                line(-3.5, 0.0, -1.25, 2.5),
                line(1.25, -2.5, 3.5, 0.0),
                line(3.5, 0.0, 1.25, 2.5),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Text => {
            // 文档有纸：折角的纸，纸上三行字（最后一行短）。
            page(frame);
            let row = |dy: f32, w: f32| Stroke::Line(cx - 2.5, cy + dy, cx - 2.5 + w, cy + dy);
            let marks = [row(-1.5, 5.0), row(1.0, 5.0), row(3.5, 3.25)];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Data => {
            // 零长度的线段画出来就是圆点。
            let keys = [
                line(-2.75, -1.5, -2.75, -1.5),
                line(-2.75, 1.75, -2.75, 1.75),
            ];
            let values = [line(-0.5, -1.5, 3.0, -1.5), line(-0.5, 1.75, 2.0, 1.75)];
            frame.stroke(&keys, DOT, color, clip);
            frame.stroke(&values, STROKE, color, clip);
        }
        FileGlyph::Shell => {
            let marks = [
                line(-3.25, -2.25, -1.0, 0.0),
                line(-1.0, 0.0, -3.25, 2.25),
                line(0.5, 2.25, 3.25, 2.25),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Image => {
            // 一个圆角的相框，里面山和太阳；山脚落在框的底边上。
            let frame_outline = rounded_outline(cx - 3.5 * k, cy - 3.25 * k, 7.0 * k, 6.5 * k, 1.5);
            let mountains = [
                line(-3.5, 2.5, -1.0, -0.25),
                line(-1.0, -0.25, 1.0, 1.75),
                line(1.0, 1.75, 2.0, 0.75),
                line(2.0, 0.75, 3.5, 2.25),
            ];
            let marks: Strokes<12> = joined(&[&frame_outline, &mountains]);
            frame.stroke(marks.as_slice(), STROKE, color, clip);
            frame.stroke(&[line(1.5, -1.5, 1.5, -1.5)], DOT - 0.4, color, clip);
        }
        FileGlyph::Archive => {
            // 一个圆角的盒子，中间一条拉链：齿左右交错，下面是拉头。
            let case = rounded_outline(cx - 3.5 * k, cy - 3.5 * k, 7.0 * k, 7.0 * k, 1.5);
            let teeth = [
                line(-0.6, -3.5, 0.0, -3.5),
                line(-0.6, -2.25, 0.6, -2.25),
                line(-0.6, -1.0, 0.6, -1.0),
            ];
            let marks: Strokes<11> = joined(&[&case, &teeth]);
            frame.stroke(marks.as_slice(), STROKE, color, clip);
            let pull = Rect::new(cx - 1.0 * k, cy + 0.5 * k, 2.0 * k, 2.25 * k);
            frame.rect_corners(pull, color, [0.75; 4], clip);
        }
        FileGlyph::Audio => {
            let marks = [line(0.75, 2.0, 0.75, -3.25), line(0.75, -3.25, 3.0, -2.25)];
            frame.stroke(&marks, STROKE, color, clip);
            frame.stroke(&[line(-0.9, 2.25, -0.9, 2.25)], DOT + 1.0, color, clip);
        }
        FileGlyph::Video => {
            // 圆角的画面框，里面一个播放三角。
            let screen = rounded_outline(cx - 3.5 * k, cy - 3.0 * k, 7.0 * k, 6.0 * k, 1.5);
            let play = [
                line(-1.0, -1.5, -1.0, 1.5),
                line(-1.0, -1.5, 1.75, 0.0),
                line(1.75, 0.0, -1.0, 1.5),
            ];
            let marks: Strokes<11> = joined(&[&screen, &play]);
            frame.stroke(marks.as_slice(), STROKE, color, clip);
        }
        FileGlyph::Font => {
            let marks = [
                line(-3.25, 3.0, 0.0, -3.25),
                line(0.0, -3.25, 3.25, 3.0),
                line(-1.75, 0.75, 1.75, 0.75),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Binary => {
            // 折角的纸，纸上两行点，每行三个。零长度的线段画出来就是圆点。
            page(frame);
            let dot = |x: f32, y: f32| Stroke::Line(cx + x, cy + y, cx + x, cy + y);
            let dots = [
                dot(-2.25, -0.25),
                dot(0.0, -0.25),
                dot(2.25, -0.25),
                dot(-2.25, 2.5),
                dot(0.0, 2.5),
                dot(2.25, 2.5),
            ];
            frame.stroke(&dots, 1.75, color, clip);
        }
        FileGlyph::Lock => {
            frame.stroke(
                &[
                    Stroke::Arc(cx, cy - 1.25 * k, 1.75 * k, -PI, 0.0),
                    line(-1.75, -1.25, -1.75, 0.0),
                    line(1.75, -1.25, 1.75, 0.0),
                ],
                STROKE,
                color,
                clip,
            );
            let body = rounded_outline(cx - 3.0 * k, cy, 6.0 * k, 3.75 * k, 1.0);
            frame.stroke(&body, STROKE, color, clip);
        }
        FileGlyph::Json => {
            // 两个花括号：竖着的一笔中间向外顶出一个尖。
            let brace = |s: f32| {
                [
                    line(s * 1.0, -3.5, s * 2.0, -3.0),
                    line(s * 2.0, -3.0, s * 2.0, -0.75),
                    line(s * 2.0, -0.75, s * 3.0, 0.0),
                    line(s * 3.0, 0.0, s * 2.0, 0.75),
                    line(s * 2.0, 0.75, s * 2.0, 3.0),
                    line(s * 2.0, 3.0, s * 1.0, 3.5),
                ]
            };
            frame.stroke(&brace(-1.0), STROKE, color, clip);
            frame.stroke(&brace(1.0), STROKE, color, clip);
        }
        FileGlyph::Markdown => {
            // 井号的两竖略向右倾。
            let marks = [
                line(-0.75, -3.25, -1.5, 3.25),
                line(1.75, -3.25, 1.0, 3.25),
                line(-3.0, -1.25, 3.25, -1.25),
                line(-3.25, 1.25, 3.0, 1.25),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Docker => {
            // 下面三个、上面两个集装箱（实心小方块），底下一道船身。
            let box_at = |dx: f32, dy: f32| {
                let s = 2.0 * k;
                Rect::new(cx + dx * k - s / 2.0, cy + dy * k - s / 2.0, s, s)
            };
            for (dx, dy) in [
                (-2.4, 0.6),
                (0.0, 0.6),
                (2.4, 0.6),
                (-1.2, -1.8),
                (1.2, -1.8),
            ] {
                frame.rect_clipped(box_at(dx, dy), color, 0.4, clip);
            }
            frame.stroke(&[line(-3.5, 3.0, 3.5, 3.0)], STROKE, color, clip);
        }
        FileGlyph::Git => {
            // 和 [`branch_shape`] 同一个图形：主干、两个提交、一道四分之一圆弧。
            let node = 1.1;
            let (top, base) = ((2.25, -2.25), (-2.25, 2.25));
            let p = |(x, y): (f32, f32)| (cx + x * k, cy + y * k);
            let (tx, ty) = p(top);
            let (bx, by) = p(base);
            let (ax, ay) = p((base.0 + node, top.1 + node));
            let marks = [
                line(-2.25, -3.75, -2.25, base.1 - node),
                Stroke::Circle(tx, ty, node * k),
                Stroke::Circle(bx, by, node * k),
                Stroke::Arc(ax, ay, (top.0 - base.0 - node) * k, 0.0, FRAC_PI_2),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::License => {
            let (bx, by) = (cx, cy - 1.0 * k);
            let marks = [
                Stroke::Circle(bx, by, 2.25 * k),
                line(-1.1, 1.0, -1.75, 3.5),
                line(1.1, 1.0, 1.75, 3.5),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Info => {
            frame.stroke(&[line(0.0, -3.0, 0.0, -3.0)], DOT, color, clip);
            let marks = [
                line(-1.0, -0.75, 0.0, -0.75),
                line(0.0, -0.75, 0.0, 3.0),
                line(-1.25, 3.0, 1.25, 3.0),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Key => {
            let (rx, ry) = (cx - 1.5 * k, cy - 1.5 * k);
            let marks = [
                Stroke::Circle(rx, ry, 1.6 * k),
                line(-0.35, -0.35, 3.0, 3.0),
                line(1.75, 1.75, 2.75, 0.75),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Package => {
            let marks = [
                line(0.0, -3.5, 3.0, -1.75),
                line(3.0, -1.75, 3.0, 1.75),
                line(3.0, 1.75, 0.0, 3.5),
                line(0.0, 3.5, -3.0, 1.75),
                line(-3.0, 1.75, -3.0, -1.75),
                line(-3.0, -1.75, 0.0, -3.5),
                line(-3.0, -1.75, 0.0, 0.0),
                line(0.0, 0.0, 3.0, -1.75),
                line(0.0, 0.0, 0.0, 3.5),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Table => {
            let marks = [
                line(-3.25, -3.0, 3.25, -3.0),
                line(3.25, -3.0, 3.25, 3.0),
                line(3.25, 3.0, -3.25, 3.0),
                line(-3.25, 3.0, -3.25, -3.0),
                line(-3.25, -0.75, 3.25, -0.75),
                line(-3.25, 1.25, 3.25, 1.25),
                line(-0.5, -3.0, -0.5, 3.0),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Database => {
            // 圆柱：上面一个整椭圆，下面半个，两边两道竖线（中间不再加一圈，小图里太挤）。
            let (rx, ry) = (3.0, 1.2);
            // 椭圆弧用 10 段折线逼近。
            let ellipse = |y: f32, from: f32, to: f32| -> [Stroke; 10] {
                let at = |t: f32| {
                    let a = from + (to - from) * t;
                    (rx * a.cos(), y + ry * a.sin())
                };
                std::array::from_fn(|i| {
                    let (x0, y0) = at(i as f32 / 10.0);
                    let (x1, y1) = at((i + 1) as f32 / 10.0);
                    line(x0, y0, x1, y1)
                })
            };
            let sides = [line(-rx, -2.5, -rx, 2.5), line(rx, -2.5, rx, 2.5)];
            let marks: Strokes<22> = joined(&[
                &ellipse(-2.5, 0.0, 2.0 * PI),
                &ellipse(2.5, 0.0, PI),
                &sides,
            ]);
            frame.stroke(marks.as_slice(), STROKE, color, clip);
        }
        FileGlyph::Letters(text) => {
            frame.stroke(monogram(text, (cx, cy)).as_slice(), STROKE, color, clip);
        }
        FileGlyph::Java => {
            // 杯身上宽下窄，右边一个杯把，杯口上方两缕热气。
            let marks = [
                line(-3.25, -0.5, 1.75, -0.5),
                line(-3.25, -0.5, -2.5, 3.0),
                line(-2.5, 3.0, 1.0, 3.0),
                line(1.0, 3.0, 1.75, -0.5),
                Stroke::Arc(cx + 2.0 * k, cy + 0.9 * k, 1.1 * k, -FRAC_PI_2, FRAC_PI_2),
                line(-1.6, -2.0, -1.0, -3.0),
                line(-1.0, -3.0, -1.6, -4.0),
                line(0.3, -2.0, 0.9, -3.0),
                line(0.9, -3.0, 0.3, -4.0),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        FileGlyph::Config => {
            frame.stroke(&gear_shape((cx, cy), (4.1, 5.7, 1.6)), STROKE, color, clip);
        }
        // 没认出来的类型：一张空白的折角的纸。
        FileGlyph::Plain => page(frame),
    }
}

/// 用线条画的编程语言缩写（`FileGlyph::Letters`），整体居中在 `(cx, cy)`。
///
/// 每个字母在 4×6 的格子里（左上角为原点）用折线画，字母间隔 1.5，整体再放大到约 11 像素宽；
/// 「+」「#」窄一些、矮一些，跟在字母后面。没有字体的粗体可用、界面字体在这个字号下又太细，
/// 所以自己画，笔宽与其他图标相同。
fn monogram(text: &str, (cx, cy): (f32, f32)) -> Strokes<MONOGRAM_STROKES> {
    /// 一个字的折线（每条是一串点）。
    type Paths = &'static [&'static [(f32, f32)]];
    // 一个字的折线和它占的宽度。
    fn glyph(ch: char) -> (Paths, f32) {
        let paths: Paths = match ch {
            'T' => &[&[(0.0, 0.0), (4.0, 0.0)], &[(2.0, 0.0), (2.0, 6.0)]],
            'S' => &[&[
                (4.0, 0.9),
                (3.1, 0.0),
                (0.9, 0.0),
                (0.0, 0.9),
                (0.0, 2.1),
                (0.9, 3.0),
                (3.1, 3.0),
                (4.0, 3.9),
                (4.0, 5.1),
                (3.1, 6.0),
                (0.9, 6.0),
                (0.0, 5.1),
            ]],
            'J' => &[&[(4.0, 0.0), (4.0, 4.8), (2.8, 6.0), (1.2, 6.0), (0.0, 4.8)]],
            'P' => &[&[
                (0.0, 6.0),
                (0.0, 0.0),
                (3.0, 0.0),
                (4.0, 1.0),
                (4.0, 2.0),
                (3.0, 3.0),
                (0.0, 3.0),
            ]],
            'Y' => &[
                &[(0.0, 0.0), (2.0, 3.0), (4.0, 0.0)],
                &[(2.0, 3.0), (2.0, 6.0)],
            ],
            'G' => &[&[
                (4.0, 1.0),
                (3.0, 0.0),
                (1.0, 0.0),
                (0.0, 1.0),
                (0.0, 5.0),
                (1.0, 6.0),
                (3.0, 6.0),
                (4.0, 5.0),
                (4.0, 3.0),
                (2.25, 3.0),
            ]],
            'O' => &[&[
                (1.0, 0.0),
                (3.0, 0.0),
                (4.0, 1.0),
                (4.0, 5.0),
                (3.0, 6.0),
                (1.0, 6.0),
                (0.0, 5.0),
                (0.0, 1.0),
                (1.0, 0.0),
            ]],
            'R' => &[
                &[
                    (0.0, 6.0),
                    (0.0, 0.0),
                    (3.0, 0.0),
                    (4.0, 1.0),
                    (4.0, 2.0),
                    (3.0, 3.0),
                    (0.0, 3.0),
                ],
                &[(2.0, 3.0), (4.0, 6.0)],
            ],
            'C' => &[&[
                (4.0, 1.0),
                (3.0, 0.0),
                (1.0, 0.0),
                (0.0, 1.0),
                (0.0, 5.0),
                (1.0, 6.0),
                (3.0, 6.0),
                (4.0, 5.0),
            ]],
            'K' => &[
                &[(0.0, 0.0), (0.0, 6.0)],
                &[(4.0, 0.0), (0.0, 4.0)],
                &[(1.5, 2.5), (4.0, 6.0)],
            ],
            'B' => &[
                &[
                    (0.0, 3.0),
                    (0.0, 0.0),
                    (2.8, 0.0),
                    (3.7, 0.9),
                    (3.7, 2.1),
                    (2.8, 3.0),
                    (0.0, 3.0),
                ],
                &[
                    (0.0, 3.0),
                    (0.0, 6.0),
                    (3.1, 6.0),
                    (4.0, 5.1),
                    (4.0, 3.9),
                    (3.1, 3.0),
                ],
            ],
            'W' => &[&[(0.0, 0.0), (1.0, 6.0), (2.0, 2.5), (3.0, 6.0), (4.0, 0.0)]],
            'E' => &[
                &[(4.0, 0.0), (0.0, 0.0), (0.0, 6.0), (4.0, 6.0)],
                &[(0.0, 3.0), (3.0, 3.0)],
            ],
            '+' => &[&[(0.0, 3.0), (2.5, 3.0)], &[(1.25, 1.75), (1.25, 4.25)]],
            '#' => &[
                &[(1.0, 1.25), (0.6, 5.25)],
                &[(2.4, 1.25), (2.0, 5.25)],
                &[(0.0, 2.5), (3.0, 2.5)],
                &[(0.0, 4.0), (3.0, 4.0)],
            ],
            _ => &[],
        };
        let width = match ch {
            '+' => 2.5,
            '#' => 3.0,
            _ => 4.0,
        };
        (paths, width)
    }

    const GAP: f32 = 1.5;
    let count = text.chars().count();
    let width: f32 =
        text.chars().map(|c| glyph(c).1).sum::<f32>() + GAP * count.saturating_sub(1) as f32;
    // 两个字母时放大到约 11 像素宽；三个（C++、ETS）时宽度一样，字小一点。
    let scale = (11.0 / width).min(1.3);
    let (left, top) = (cx - width * scale / 2.0, cy - 3.0 * scale);
    let mut x = 0.0;
    let mut out = Strokes::new();
    for (paths, w) in text.chars().map(glyph) {
        for path in paths {
            for pair in path.windows(2) {
                let p = |(px, py): (f32, f32)| (left + (x + px) * scale, top + py * scale);
                let (a, b) = (p(pair[0]), p(pair[1]));
                out.push(Stroke::Line(a.0, a.1, b.0, b.1));
            }
        }
        x += w + GAP;
    }
    out
}

/// [`monogram`] 最多的笔画数：「RB」的 18 笔，留些余量。
const MONOGRAM_STROKES: usize = 24;
