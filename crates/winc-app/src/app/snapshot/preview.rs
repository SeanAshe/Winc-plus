//! Markdown 预览的测试：三段切换、后台解析、分栏联动滚动、点击链接、本地与远程图片、全屏查看
//! mermaid 图、截图与计时。

use winc_editor::PreviewMode;

use super::*;

mod images;
mod live;
mod markdown;
mod navigation;
mod pdf;
#[cfg(any(target_os = "macos", windows))]
mod pdf_places;
mod reading;
mod tables;

/// 覆盖常用 Markdown 写法的 README 样本。
const SAMPLE: &str = r#"<p align="center"><img src="logo.svg" width="64" alt="logo"></p>

# winc 编辑器

一个用 **Rust** 写的轻量编辑器，*启动快*、占内存少。支持 `cargo xtask test`、~~旧的写法~~，
中文和 English 混排时按字换行，链接比如 [GitHub](https://github.com) 和 [安装](#安装)。

> [!NOTE]
> 预览跟着左边的文字实时更新。

## 安装

1. 下载发行版
2. 解压到 `/Applications`
   - 第一次打开要在设置里允许
   - 也可以自己编译

- [x] 三段切换
- [ ] Typora 式编辑

```rust
fn main() {
    // 打印一行
    println!("hello, {}", 42);
}
```

| 功能 | 状态 | 说明 |
|:-----|:----:|-----:|
| 分栏 | ✅ | 左边改，右边看 |
| 预览 | ✅ | 只读 |
| 表格 | ✅ | 隔行铺底色 |

> 引用的文字是灰色的，
> 可以有多行。

---

脚注写在最后[^1]。

[^1]: 这是脚注的内容。
"#;

const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect x="5" y="5" width="90" height="90" rx="22" fill="#3296FC"/>
  <path d="M28 34 L40 68 L50 44 L60 68 L72 34" fill="none" stroke="#fff" stroke-width="8"
        stroke-linecap="round" stroke-linejoin="round"/>
</svg>"##;

/// 打开 README，等预览完成解析与图片加载，返回应用和临时目录。
///
/// 读取与绘制图片的请求在绘制一帧时发出，因此用 `settle` 绘制多轮。
fn readme(name: &str, dark: bool, mode: PreviewMode) -> (App, PathBuf) {
    let dir = sample_folder(name);
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    let path = dir.join("README.md");
    std::fs::write(&path, SAMPLE).unwrap();
    let mut app = app_with(&path, dark);
    paint(&mut app);
    app.set_preview_mode(mode);
    settle(&mut app);
    (app, dir)
}

fn settle(app: &mut App) {
    for _ in 0..4 {
        app.wait_for_markdown();
        paint(app);
    }
}

/// 按住 ⌘（其他平台为 Ctrl）在 `at` 处单击。
fn command_click(app: &mut App, at: (f32, f32)) {
    app.pointer.modifiers = if cfg!(target_os = "macos") {
        command_modifiers()
    } else {
        winit::keyboard::ModifiersState::CONTROL
    };
    super::click(app, at);
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
}

/// 返回文字光标在原文中的位置（字节偏移）之后的一段原文。
fn after_caret(app: &App, len: usize) -> String {
    let doc = app.editor.active().unwrap();
    let text = doc.text().rope().unwrap().to_string();
    let at = app.live_caret_source().unwrap();
    text[at..].chars().take(len).collect()
}

/// 返回光标在屏幕上的纵坐标（窗口坐标）。
fn caret_screen_y(app: &App) -> f32 {
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let (_, y, _) = laid.caret_point(app.live_caret_source().unwrap()).unwrap();
    app.preview.markdown.area.y + y - app.preview.markdown.scroll()
}

#[test]
fn clicking_where_markers_appear_does_not_select() {
    let (mut app, dir) = live_doc(
        "md-reveal-click",
        "# winc 模块设计\n\n正文 **粗** 字\n\n| 名 |\n|---|\n| **甲** |\n\n```rust\nfn main() {}\n```\n",
    );
    // 点击标题开头、粗体、表格、代码块：按下时不显示标记（文字不移动），松开后才显示，
    // 因此单击不会选中一段文字。
    for (needle, marker) in [
        ("winc", "# winc"),
        ("粗", "**粗**"),
        ("甲", "**甲**"),
        ("main", "fn main() {}"),
    ] {
        let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
        let at = shown[..shown.find(needle).unwrap()].chars().count();
        let (x, y) = point_of(&app, at);
        app.on_mouse_move(x - 1.0, y);
        app.on_mouse_down(MouseButton::Left);
        paint(&mut app);
        app.on_mouse_move(x, y);
        app.on_mouse_up(MouseButton::Left);
        settle(&mut app);
        assert!(app.editor.active().unwrap().selection.is_empty());
        assert!(after_caret(&app, 1) == needle[..needle.chars().next().unwrap().len_utf8()]);
        let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
        assert!(shown.contains(marker), "{shown}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// 打开内容为 `text` 的 Markdown 文件，只看预览，等待解析、排版完成。
fn live_doc(name: &str, text: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("LIVE.md");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    (app, dir)
}

#[test]
fn deeply_nested_documents_lay_out_and_paint() {
    // 嵌套超过上限的引用、列表、公式：排版和绘制同样按层递归，测试线程与后台线程的栈都是 2 MB。
    let lists: String = (0..300)
        .map(|i| format!("{}- x{i}\n", "  ".repeat(i)))
        .collect();
    let text = format!(
        "{} deep\n\n{lists}\n$${}x$$\n\n${}x$\n",
        ">".repeat(20_000),
        "{".repeat(50_000),
        "\\hat".repeat(50_000),
    );
    let (app, dir) = live_doc("md-deep", &text);
    assert!(app.preview.markdown.height > app.preview.markdown.area.h);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 返回文档当前的文字。
fn doc_text(app: &App) -> String {
    app.editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string()
}

/// 将光标放到第一处 `needle` 之后（`after` 为假时之前），等待排版完成。
fn put_caret(app: &mut App, needle: &str, after: bool) {
    let text = doc_text(app);
    let byte = text.find(needle).unwrap() + if after { needle.len() } else { 0 };
    let doc = app.editor.active_mut().unwrap();
    let pos = doc.text().rope().unwrap().byte_to_char(byte);
    doc.selection = winc_core::Selection::point(pos);
    settle(app);
}

/// 返回在光标处插入 `|` 后的文档文字。
fn with_caret(app: &App) -> String {
    let mut text = doc_text(app);
    text.insert(app.live_caret_source().unwrap(), '|');
    text
}

#[test]
fn details_open_and_close_on_click() {
    let dir = sample_folder("md-details");
    let path = dir.join("README.md");
    std::fs::write(
        &path,
        "开头\n\n<details>\n<summary>点开看</summary>\n\n藏起来的话\n\n</details>\n\n结尾\n",
    )
    .unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let shown = |app: &App| {
        crate::app::preview::markdown::laid::full_text(app)
            .unwrap()
            .contains("藏起来的话")
    };
    assert!(!shown(&app));
    let height = app.preview.markdown.height;
    let toggle = |app: &mut App| {
        let (rect, _) = app
            .hits
            .markdown_links
            .iter()
            .find(|(_, url)| url.starts_with(crate::app::preview::markdown::laid::DETAILS))
            .cloned()
            .expect("折叠块的标题能点");
        super::click(app, (rect.x + 4.0, rect.y + 4.0));
        settle(app);
    };
    toggle(&mut app);
    assert!(shown(&app));
    assert!(app.preview.markdown.height > height);
    toggle(&mut app);
    assert!(!shown(&app));
    assert_eq!(app.preview.markdown.height, height);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 返回预览上按下位置落在字符偏移 `offset` 之前的一点（逻辑像素）。
///
/// # Panics
///
/// 预览上找不到落在该字符上的一点时 panic。
fn point_of(app: &App, offset: usize) -> (f32, f32) {
    let area = app.preview.markdown.area;
    // 在预览上逐行扫描，寻找落在该字符上的位置。
    let mut y = area.y;
    while y < area.bottom() {
        let mut x = area.x;
        while x < area.right() {
            if crate::app::preview::markdown::laid::offset_at(app, x, y) == Some(offset)
                && crate::app::preview::markdown::laid::offset_at(app, x + 4.0, y)
                    == Some(offset + 1)
            {
                return (x, y);
            }
            x += 2.0;
        }
        y += 4.0;
    }
    panic!("预览上未找到第 {offset} 个字符");
}

/// GFM 补充写法的样本：front matter、短代码表情、上下标、`<details>` 折叠块、脚注，用于截图。
const EXTRAS: &str = r#"---
title: 发布说明
version: "1.2"
---

# 新东西 :tada: :rocket:

水的分子式是 H<sub>2</sub>O，面积 10 m<sup>2</sup>，再看脚注[^n]。

<details>
<summary>点开看 <b>细节</b></summary>

- 收起时只显示标题
- 点一下展开

</details>

<details open>
<summary>默认展开的</summary>

藏在里面的一段话。

</details>

```sh
cargo xtask test
```

[^n]: 脚注末尾可以跳回去。
"#;

/// 公式、内联 HTML 与 mermaid 图的样本，用于截图。
const RICH: &str = r#"# 公式

行内的 $E = mc^2$，还有 $\alpha_i^2 + \beta' \le \sqrt{x_1 + x_2}$ 和 $\frac{1}{2}$。

$$
\sum_{i=1}^{n} i = \frac{n(n+1)}{2} \qquad \int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}
$$

$$
\left( \frac{a}{b} \right)^2 = \lim_{x \to 0} \frac{\sin x}{x} \cdot \sqrt[3]{\frac{x^2+1}{y}} \quad \hat{x}\ \vec{v}\ \overline{AB}\ \mathbb{R}^n
$$

```math
A = \begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}, \quad f(x) = \begin{cases} x^2 & x \ge 0 \\ -x & \text{否则} \end{cases}
```

$$
\begin{aligned} a &= b + c \\ &= \underbrace{d + e}_{\text{两项}} \end{aligned}
$$

<table>
<tr><th>名字</th><th align="right">数量</th></tr>
<tr><td>苹果</td><td>3</td></tr>
<tr><td>梨</td><td><b>5</b></td></tr>
</table>

<ul><li>HTML 列表一</li><li>二</li></ul>
"#;

/// README 顶部常见的 HTML：居中的标题和图标、徽章、按百分比并排的截图、表格里的图。
const HTML_README: &str = r#"<h1 align="center">
  <img src="logo.svg" width="64"><br>
  DEEIX Chat
</h1>

<p align="center">
  <a href="https://example.com"><img src="badge.svg" alt="build"></a>
  <a href="https://example.com"><img src="badge.svg" alt="license"></a>
</p>

<p align="center">
  <img src="wide.svg" alt="一" width="49.45%" />
  <img src="wide.svg" alt="二" width="49.45%" />
</p>

<p align="center">
  <img src="wide.svg" width="32.3%" />
  <img src="wide.svg" width="32.3%" />
  <img src="wide.svg" width="32.3%" />
</p>

按 <kbd>⌘</kbd> + <kbd>K</kbd>，<u>下划线</u>，<ins>插入</ins>，<mark>高亮</mark>，<small>小字</small>，H<sub>2</sub>O。

<div align="right">右对齐的一段</div>

<table>
  <thead><tr><th>功能</th><th>截图</th></tr></thead>
  <tbody>
    <tr><td>聊天</td><td><img src="wide.svg" width="160"></td></tr>
  </tbody>
</table>
"#;

const BADGE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="90" height="20"><rect width="90" height="20" rx="3" fill="#4c1"/></svg>"##;

const WIDE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1000"><rect width="1600" height="1000" fill="#6a8fd8"/><circle cx="800" cy="500" r="300" fill="#f2c94c"/></svg>"##;

fn html_readme(name: &str, dark: bool) -> (App, PathBuf) {
    let dir = sample_folder(name);
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    std::fs::write(dir.join("badge.svg"), BADGE).unwrap();
    std::fs::write(dir.join("wide.svg"), WIDE).unwrap();
    let path = dir.join("README.md");
    std::fs::write(&path, HTML_README).unwrap();
    let mut app = app_with(&path, dark);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    (app, dir)
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_html_readme() {
    for dark in [false, true] {
        let (mut app, dir) = html_readme("md-html", dark);
        shoot(
            &mut app,
            &format!("md-html-{}", if dark { "dark" } else { "light" }),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// mermaid 图的样本，每项为一个文档：
///
/// - 流程图与状态图；
/// - 时序图与饼图；
/// - 含多个子图的架构图，高度超过一屏；
/// - 节点文字较多、从左向右的架构图，以及含多条回边的状态图。
const DIAGRAMS: [&str; 4] = [
    r#"```mermaid
flowchart TD
    A[开始] --> B{登录了吗?}
    B -->|是| C[打开首页]
    B -->|否| D([登录页])
    D --> E[(用户库)]
    E -.-> B
    C ==> F((完成))
    subgraph 后台
        E
    end
```

```mermaid
graph LR
    a>不对称] --> b[[子程序]] --> c{{六边形}} --> d[/平行/]
    c --o e[\梯形/]
    c --x f(圆角)
```

```mermaid
stateDiagram-v2
    [*] --> 空闲
    空闲 --> 运行: 开始
    运行 --> 空闲: 停下
    运行 --> [*]
```
"#,
    r#"```mermaid
sequenceDiagram
    autonumber
    actor 用户
    participant W as 网页
    participant S as 服务器
    用户->>W: 点登录
    W->>+S: POST /login
    Note right of S: 查用户库
    alt 密码对
        S-->>W: 200 OK
    else 密码错
        S--xW: 401
    end
    S-->>-W: 结束
    W->>W: 刷新
    loop 每分钟
        W-)S: 心跳
    end
```

```mermaid
pie showData
    title 宠物
    "狗" : 386
    "猫" : 85
    "兔子" : 15
```
"#,
    r#"```mermaid
flowchart TB
    U[User / Admin Browser]
    subgraph FE[Frontend Build]
        N[Next.js 16 / React 19<br/>Chat UI / Admin Console]
    end
    subgraph GO[Go Single Runtime]
        S[Static Asset Serving]
        A[Gin HTTP API]
        APP[Application<br/>Auth / Routing / Files / Billing / Audit]
        I[Infra Adapters<br/>Protocols / Data / Cache / Storage]
    end
    subgraph EXT[External Capabilities]
        M[Model Providers<br/>OpenAI / Anthropic / Google / xAI / OpenRouter]
        T[Tool Services<br/>MCP / Provider Native Tools]
        F[Optional File Processing<br/>Tika / Docling / OCR]
    end
    subgraph DS[Data and Storage]
        P[PostgreSQL + pgvector<br/>or SQLite + sqlite-vec]
        R[Redis<br/>or In-Memory Cache]
        L[Local Filesystem<br/>or S3-Compatible Storage]
    end
    U --> S
    U --> A
    N --> S
    A --> APP --> I
    I --> M
    I --> T
    I --> F
    I --> P
    I --> R
    I --> L
```

```mermaid
flowchart LR
    subgraph 外层
        subgraph 内层
            a --> b
        end
        b --> c
    end
    x --> a
    c --> y
```
"#,
    r#"# qcAgent 架构图

## 1. 总体架构

```mermaid
flowchart LR
    caller[调用方平台]
    config[配置目录<br/>规则表 · 规则包 · 文档类型表 · 模型档案]
    subgraph svc[qcAgent 服务（一个代码库，两个入口）]
        api[qc-api<br/>鉴权 · 校验 · 数据等级闸门<br/>幂等 · 结果复用 · 规则预览]
        worker[qc-worker<br/>领取任务（租约）· 心跳 / 取消 · 清扫<br/>执行流水线 · 上传结果 · 回调]
        subgraph rt[运行环境（每个任务一套）]
            shard[分片 s01 … sNN<br/>容器模式：工具容器 + 每轮一个 pi 容器<br/>本地模式：pi 和执行服务是本机进程]
            net[容器模式：internal 网络 + 出口代理，只放行模型端点<br/>本地模式（QC_RUNTIME=local）：没有网络隔离]
        end
    end
    model[私有化模型]
    subgraph store[存储]
        db[(Postgres / SQLite<br/>任务 · 事件 · 幂等键 · 回调发件箱)]
        fs[(文件存储（本地目录）<br/>输入（按调用方、内容寻址）<br/>结果 · 审计包)]
    end
    caller -- 上传文件 / 提交任务 / 查询 / 取结果 --> api
    config --> worker
    api --> db
    api --> fs
    worker -- 领取 · 续租 · 写终态 --> db
    worker -- 读输入 · 写结果和审计包 --> fs
    worker --> shard
    shard --> net
    net --> model
    worker -. 签名回调（只含任务 ID 和状态） .-> caller
```

## 2. 任务生命周期

```mermaid
stateDiagram-v2
    [*] --> queued: 提交通过校验
    [*] --> succeeded: 输入与通过验收的旧任务完全相同（复用结果）
    queued --> running: worker 领取（设置租约）
    running --> queued: 租约过期（worker 失联），重试次数未用完
    queued --> failed: 排队超过截止时间（expired）
    queued --> canceled: 取消
    running --> canceled: 心跳发现取消，终止 pi 容器
    running --> succeeded: 有结果（passed 或 degraded）
    running --> failed: 拿不出结果（input_error / refused / system_error）
    succeeded --> [*]
    failed --> [*]
    canceled --> [*]
```
"#,
];

/// 宽度超过一行的流程图：多个子图并排，节点文字较长，结构来自实际的架构图。
const WIDE_DIAGRAM: &str = "```mermaid
flowchart TD
  subgraph FE[前端构建]
    Next[Next.js 16 / React 19<br/>Chat UI / Admin Console]
  end
  User[用户 / 管理员浏览器]
  subgraph Go[Go 单运行时]
    Static[静态资源托管]
    Gin[Gin HTTP API]
    App[Application<br/>权限 / 路由 / 文件 / 计费 / 审计]
    Infra[Infra Adapters<br/>协议 / 数据 / 缓存 / 存储]
  end
  subgraph Ext[外部能力]
    Models[模型服务商<br/>OpenAI / Anthropic / Google / xAI / OpenRouter]
    Tools[工具服务<br/>MCP / 官方原生工具]
    Extractors[可选文件处理<br/>Tika / Docling / OCR]
  end
  subgraph Data[数据与存储]
    DB[PostgreSQL + pgvector<br/>或 SQLite + sqlite-vec]
    Cache[Redis 或进程内缓存]
    Storage[本地文件 / S3 兼容存储]
  end
  Next --> Static
  User --> Static
  User --> Gin
  Gin --> App
  App --> Infra
  Infra --> Models
  Infra --> Tools
  Infra --> Extractors
  Infra --> DB
  Infra --> Cache
  Infra --> Storage
```
";

/// 类图、实体关系图、思维导图、Git 提交图、甘特图的样本，每项为一个文档。
const MORE_DIAGRAMS: [&str; 5] = [
    r#"```mermaid
classDiagram
  direction LR
  class Shape {
    <<interface>>
    +area() double
    +perimeter() double
  }
  class Circle {
    -double radius
    +area() double
  }
  class Square {
    -double side
    +area() double
  }
  class Canvas {
    +List~Shape~ shapes
    +draw()
  }
  class Palette
  Shape <|.. Circle
  Shape <|.. Square
  Canvas "1" *-- "many" Shape : 包含
  Canvas o-- Palette
  Canvas ..> Logger : 使用
```
"#,
    r#"```mermaid
erDiagram
  SHELF ||--o{ BOOK : holds
  BOOK ||--|{ PAGE : contains
  BOOK }o..o| SERIES : "belongs to"
  SHELF {
    int id PK
    string label
  }
  BOOK {
    string isbn PK "编号"
    int shelf_id FK
    string title
    int pages
  }
  PAGE {
    int number PK
    string isbn FK
  }
```
"#,
    r#"```mermaid
mindmap
  root((学习计划))
    基础
      数据结构
      算法
        排序
        查找
    工具
      [编辑器]
      (版本控制)
    实践
      小项目
      代码评审
    阅读
      {{技术书籍}}
      文档
```
"#,
    r#"```mermaid
gitGraph
  commit id: "init"
  commit id: "readme"
  branch develop
  checkout develop
  commit id: "parser"
  branch feature
  checkout feature
  commit id: "layout"
  commit id: "draw" type: HIGHLIGHT
  checkout develop
  merge feature
  checkout main
  commit id: "hotfix" type: REVERSE
  merge develop tag: "v1.0"
  cherry-pick id: "parser"
```
"#,
    r#"```mermaid
gantt
  title 版本计划
  dateFormat YYYY-MM-DD
  section 设计
  需求整理 :done, a1, 2024-03-01, 4d
  界面草图 :done, a2, after a1, 3d
  section 开发
  解析器 :active, b1, 2024-03-08, 6d
  布局 :crit, b2, after b1, 5d
  绘制 :b3, after b2, 4d
  section 发布
  测试 :c1, after b3, 3d
  发布 :milestone, after c1, 0d
```
"#,
];

/// 打开内容为 `DIAGRAMS[k]` 的文档，切换到只看预览，鼠标移到第一张图上后点击全屏按钮。
fn diagram_view_app(name: &str, k: usize, dark: bool) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("MERMAID.md");
    std::fs::write(&path, DIAGRAMS[k]).unwrap();
    let mut app = app_with(&path, dark);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let diagram = *app.hits.markdown_codes.first().expect("画出了图");
    app.on_mouse_move(diagram.x + diagram.w / 2.0, diagram.y + 20.0);
    paint(&mut app);
    let expand = crate::app::preview::markdown::laid::EXPAND;
    let (button, _) = app
        .hits
        .markdown_links
        .iter()
        .find(|(_, url)| url == expand)
        .cloned()
        .expect("鼠标在图上时有全屏按钮");
    click(
        &mut app,
        (button.x + button.w / 2.0, button.y + button.h / 2.0),
    );
    paint(&mut app);
    (app, dir)
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_github_extras() {
    for dark in [false, true] {
        let dir = sample_folder("md-extras");
        let path = dir.join("EXTRAS.md");
        std::fs::write(&path, EXTRAS).unwrap();
        let mut app = app_with(&path, dark);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        let code = *app.hits.markdown_codes.first().unwrap();
        app.on_mouse_move(code.x + 20.0, code.y + 10.0);
        shoot(
            &mut app,
            &format!("md-extras-{}", if dark { "dark" } else { "light" }),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 计时：`DIAGRAMS` 重复 5 遍组成的文档首次排版的耗时，以及只看预览时滚动 50 帧的耗时；
/// 同一文档的图按普通代码块显示时的耗时作为对照。
#[test]
#[ignore = "计时，手动运行"]
fn time_mermaid() {
    for lang in ["mermaid", "text"] {
        let text = DIAGRAMS
            .join("\n\n")
            .repeat(5)
            .replace("```mermaid", &format!("```{lang}"));
        let dir = sample_folder("md-time-mermaid");
        let path = dir.join("MERMAID.md");
        std::fs::write(&path, &text).unwrap();
        let mut app = app_with(&path, false);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        app.wait_for_markdown();
        let started = std::time::Instant::now();
        paint(&mut app);
        eprintln!("mermaid 第一次排版 {:?}", started.elapsed());
        let started = std::time::Instant::now();
        for i in 0..50 {
            app.preview.markdown.scroll_to((i * 173) as f32);
            paint(&mut app);
        }
        eprintln!("{lang} 滚 50 次 {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 打印解析结果中各部分占用的堆内存（按容量计算）。
#[cfg(target_os = "macos")]
fn heap_breakdown(md: &winc_preview::markdown::Markdown) {
    use winc_preview::markdown::{Block, Inline};
    #[derive(Default, Debug)]
    struct Sum {
        blocks: usize,
        block_count: usize,
        inlines: usize,
        text: usize,
        text_len: usize,
        runs: usize,
        run_count: usize,
        map: usize,
        anchors: usize,
        links: usize,
        code: usize,
        other: usize,
    }
    fn inline(i: &Inline, s: &mut Sum) {
        s.inlines += 1;
        s.text += i.text.capacity();
        s.text_len += i.text.len();
        s.runs += i.runs.capacity() * std::mem::size_of::<winc_preview::markdown::Run>();
        s.run_count += i.runs.len();
        s.map += i.map.heap_bytes();
        s.anchors += i.map.anchors();
        s.links += i.links.iter().map(String::capacity).sum::<usize>() + i.links.capacity() * 24;
        s.other += i.images.capacity() * std::mem::size_of::<winc_preview::markdown::Image>();
    }
    fn blocks(v: &Vec<Block>, s: &mut Sum) {
        s.blocks += v.capacity() * std::mem::size_of::<Block>();
        for b in v {
            s.block_count += 1;
            match b {
                Block::Paragraph(i) | Block::Heading(_, i) => inline(i, s),
                Block::Code { lang, text, map } => {
                    s.code += lang.capacity() + text.capacity() + map.heap_bytes();
                }
                Block::Quote(_, b) | Block::Footnote(_, b) => blocks(b, s),
                Block::List(_, items) => {
                    s.other +=
                        items.capacity() * std::mem::size_of::<winc_preview::markdown::Item>();
                    for it in items {
                        blocks(&it.blocks, s);
                    }
                }
                Block::Table(a, rows, _) => {
                    s.other += a.capacity() + rows.capacity() * 24;
                    for r in rows {
                        s.other += r.capacity() * std::mem::size_of::<Inline>();
                        for c in r {
                            inline(c, s);
                        }
                    }
                }
                Block::Math(t, _) => s.code += t.capacity(),
                Block::Rule(_) => {}
                Block::Details {
                    summary, blocks: b, ..
                } => {
                    inline(summary, s);
                    blocks(b, s);
                }
            }
        }
    }
    let mut s = Sum::default();
    blocks(&md.blocks, &mut s);
    eprintln!(
        "size_of Block {} Inline {}",
        std::mem::size_of::<Block>(),
        std::mem::size_of::<Inline>()
    );
    eprintln!("{s:#?}");
    eprintln!(
        "hashes {} lines {} ranges {} leaves {}",
        md.hashes.capacity() * 8,
        md.lines.capacity() * 8,
        md.ranges.capacity() * 16,
        md.leaves.capacity() * std::mem::size_of::<winc_preview::markdown::LeafRange>()
    );
}

#[test]
fn the_chosen_preview_mode_applies_to_files_opened_later() {
    use winc_editor::PreviewMode;

    let dir = sample_folder("preview-mode-remembered");
    let (a, b, c) = (dir.join("a.md"), dir.join("b.md"), dir.join("c.md"));
    for path in [&a, &b, &c] {
        std::fs::write(path, "# 标题\n").unwrap();
    }
    let mut app = app_with(&a, false);
    paint(&mut app);
    let mode = |app: &App| app.editor.active().unwrap().preview_mode();
    assert_eq!(mode(&app), PreviewMode::Source);

    // 选择只看预览：记入状态，之后打开的 Markdown 文件同样只看预览。
    app.choose_preview_mode(PreviewMode::Preview);
    assert_eq!(app.state.preview_mode.as_deref(), Some("preview"));
    app.open_path(&b);
    paint(&mut app);
    assert_eq!(mode(&app), PreviewMode::Preview);

    // 改回源代码：状态清空，之后打开的文件显示源代码。
    app.choose_preview_mode(PreviewMode::Source);
    assert_eq!(app.state.preview_mode, None);
    app.open_path(&c);
    paint(&mut app);
    assert_eq!(mode(&app), PreviewMode::Source);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 把 `files`（包中路径、内容）原样存放（不压缩）写成 zip，按 ZIP 文件格式（APPNOTE.TXT）
/// 逐字段写出；读取时不核对 CRC，校验字段固定写 0。
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let sizes = [
            (data.len() as u32).to_le_bytes(),
            (data.len() as u32).to_le_bytes(),
        ];
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0, 0, 0]);
        out.extend([0; 8]);
        out.extend(sizes.concat());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(*data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0, 0, 0]);
        central.extend([0; 8]);
        central.extend(sizes.concat());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend([0; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend([0, 0]);
    out
}

/// 一份 docx：两个标题、一段正文和包中的一张图（[`LOGO`]）。
fn sample_docx() -> Vec<u8> {
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let root = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{rel}/officeDocument\" Target=\"word/document.xml\"/>\
         </Relationships>"
    );
    let rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rI\" Type=\"{rel}/image\" Target=\"media/logo.svg\"/>\
         </Relationships>"
    );
    let heading = |level: u8, text: &str| {
        format!(
            "<w:p><w:pPr><w:outlineLvl w:val=\"{level}\"/></w:pPr>\
             <w:r><w:t>{text}</w:t></w:r></w:p>"
        )
    };
    let document = format!(
        "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:r=\"{rel}\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
         xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\">\
         <w:body>{}{}<w:p><w:r><w:t>正文，</w:t></w:r><w:r><w:rPr><w:b/></w:rPr>\
         <w:t>加粗</w:t></w:r></w:p><w:p><w:r><w:drawing><wp:inline>\
         <wp:extent cx=\"914400\" cy=\"914400\"/><wp:docPr id=\"1\" name=\"标志\"/>\
         <a:graphic><a:graphicData><a:blip r:embed=\"rI\"/></a:graphicData></a:graphic>\
         </wp:inline></w:drawing></w:r></w:p></w:body></w:document>",
        heading(0, "季度报告"),
        heading(1, "概况"),
    );
    stored_zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/media/logo.svg", LOGO.as_bytes()),
    ])
}

/// 写一份 `count` 页的 PDF（每页 US Letter 大小，612 × 792 点）：每页顶部用 Helvetica 写页号，
/// 下方绘制一条长度随页号变化的红色横条。交叉引用表按各对象的实际偏移写出。
#[cfg(any(target_os = "macos", windows))]
fn sample_pdf(count: usize) -> Vec<u8> {
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {count} >>",
            (0..count)
                .map(|k| format!("{} 0 R", 3 + k * 2))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];
    for k in 0..count {
        let content = format!(
            "BT /F1 36 Tf 72 700 Td (Page {}) Tj ET 1 0 0 rg 72 620 {} 40 re f",
            k + 1,
            100 + k * 40
        );
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R \
             /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 \
             /BaseFont /Helvetica >> >> >> >>",
            4 + k * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (k, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", k + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// 打开 `path`，等待渲染线程完成打开和视口附近各页的渲染。
#[cfg(any(target_os = "macos", windows))]
fn open_pdf(path: &Path, dark: bool) -> App {
    let mut app = app_with(path, dark);
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    paint(&mut app);
    app
}

/// 跳到脚注和跳回引用处时，目标位于预览上部三分之一处（滚动不到时尽量靠近），上方保留一段
/// 上下文；跳到标题时标题仍在顶部。测试中没有动画时钟，标示目标的底色不在此检查。
#[test]
fn footnote_jumps_keep_context_above_and_mark_the_target() {
    let mut text = String::from("# 标题\n\n");
    for i in 1..=40 {
        if i == 20 {
            text.push_str("这一段引用了一个脚注[^a]。\n\n");
        } else {
            text.push_str(&format!("第 {i} 段正文。\n\n"));
        }
    }
    text.push_str("[^a]: 脚注的内容。\n");
    let (mut app, dir) = live_doc("md-footnote-jump", &text);
    let max = app.preview.markdown.height - app.preview.markdown.area.h;
    let third = app.preview.markdown.area.h / 3.0;

    app.jump_to_anchor("fn-1");
    let note = crate::app::preview::markdown::laid::anchor_top(&app, "fn-1").unwrap();
    assert_eq!(
        app.preview.markdown.scroll(),
        (note - third).clamp(0.0, max)
    );

    app.jump_to_anchor("fnref-1");
    let back = crate::app::preview::markdown::laid::anchor_top(&app, "fnref-1").unwrap();
    assert_eq!(
        app.preview.markdown.scroll(),
        (back - third).clamp(0.0, max)
    );

    // 标题照旧滚动到顶部。
    app.jump_to_anchor("标题");
    assert_eq!(app.preview.markdown.scroll(), 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 光标移到分隔线上：显示原文，高度不变，下方内容不移动；列表里的分隔线只显示这一行的原文，
/// 列表的其余项仍按排版显示。
#[test]
fn rules_show_their_source_in_place() {
    let text = "前文\n\n---\n\n1. 一\n2. ---\n\n后文\n";
    let (mut app, dir) = live_doc("md-rule-live", text);
    put_caret(&mut app, "前文", true);
    let height = app.preview.markdown.height;
    put_caret(&mut app, "\n---", false);
    let doc = app.editor.active_mut().unwrap();
    doc.selection = winc_core::Selection::point(doc.selection.head + 1);
    settle(&mut app);
    assert!((app.preview.markdown.height - height).abs() < 0.5);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("---") && shown.contains("一"), "{shown}");
    put_caret(&mut app, "2. ---", true);
    assert!((app.preview.markdown.height - height).abs() < 0.5);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("一") && !shown.contains("1."), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 段落之后的空行上组字时，组字文字画在光标处；尚未点击预览就开始组字时同样显示。
#[test]
fn composing_on_a_blank_line_shows_the_text() {
    use crate::app::preview::markdown::laid::full_text;
    use winit::event::Ime;

    let (mut app, dir) = live_doc("md-preedit-blank", "# 标题\n\n是的\n\n\n");
    app.run(winc_editor::Command::DocEnd { extend: false });
    settle(&mut app);
    let caret = app.ime.cursor.unwrap();
    app.on_ime(Ime::Preedit("xiexie".into(), None));
    paint(&mut app);
    let after = app.ime.cursor.unwrap();
    assert!(after.x > caret.x + 20.0, "{caret:?} {after:?}");
    assert_eq!((after.y, after.h), (caret.y, caret.h));
    let _ = std::fs::remove_dir_all(&dir);

    // 刚打开、尚未点击：组字开始后显示光标。
    let (mut app, dir) = live_doc("md-preedit-fresh", "你好\n");
    assert!(app.ime.cursor.is_none());
    app.on_ime(Ime::Preedit("ni".into(), None));
    paint(&mut app);
    assert!(app.ime.cursor.is_some());
    assert!(full_text(&app).unwrap().contains("ni"));
    let _ = std::fs::remove_dir_all(&dir);
}
