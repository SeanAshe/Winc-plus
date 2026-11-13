//! 各类块的排版：标题、引用、独占一行的公式和可折叠的块。

use super::*;

impl Cx<'_, '_> {
    /// 排一个 `level` 级标题；一、二级标题下方加一道横线，并在 `out` 中记下标题的锚点。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn heading(
        &mut self,
        level: u8,
        inline: &Inline,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let base = self.base;
        let scale = [2.0, 1.5, 1.25, 1.0, 0.875, 0.85][(level as usize).clamp(1, 6) - 1];
        let size = base * scale;
        let color = if level == 6 {
            self.colors.muted
        } else {
            env.color
        };
        let para = Para {
            size,
            bold: true,
            color,
            line: 1.25,
            align: Align::None,
        };
        out.anchors.push((slug(&inline.text), y));
        let mut flow = self.flow(inline, x, y, width, para, &mut out.items);
        if level <= 2 {
            let line_y = y + flow.height + size * 0.3;
            out.items.push(Item::Fill {
                rect: Rect::new(x, line_y, width, 1.0),
                color: self.colors.border,
                radius: 0.0,
            });
            flow.height += size * 0.3 + 1.0;
        }
        flow
    }

    /// 排一段引用：左侧一道竖线，内容缩进；`alert` 不为 `None` 时是提示块，竖线和标题用其颜色。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn quote(
        &mut self,
        alert: Option<Alert>,
        blocks: &[Block],
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let base = self.base;
        let para = self.body_para(env);
        let bar = (base * 0.25).round();
        let inner = x + bar + base;
        let room = (width - bar - base).max(1.0);
        let (bar_color, env) = match alert {
            Some(alert) => (Colors::alert(self.dark, alert), env),
            None => (
                self.colors.border,
                Env {
                    color: self.colors.muted,
                    ..env
                },
            ),
        };
        let mut top = y;
        if let Some(alert) = alert {
            let title = match alert {
                Alert::Note => "Note",
                Alert::Tip => "Tip",
                Alert::Important => "Important",
                Alert::Warning => "Warning",
                Alert::Caution => "Caution",
            };
            let inline = Inline {
                text: title.to_owned(),
                runs: vec![winc_preview::markdown::Run {
                    end: title.len(),
                    style: Style::default(),
                }],
                ..Inline::default()
            };
            let para = Para {
                bold: true,
                color: bar_color,
                ..para
            };
            let flow = self.flow(&inline, inner, top, room, para, &mut out.items);
            top += flow.height + base * 0.25;
        }
        let flow = self.blocks(blocks, inner, top, room, env, out);
        let height = top + flow.height - y;
        out.items.push(Item::Fill {
            rect: Rect::new(x, y, bar, height),
            color: bar_color,
            radius: 0.0,
        });
        Flow {
            height,
            first_baseline: flow.first_baseline,
        }
    }

    /// 排一个独占一行的公式，水平居中；`source` 是公式在原文中的位置，点击公式时光标移到那里。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn math_block(
        &mut self,
        tex: &str,
        source: usize,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let base = self.base;
        let mut b = self.math(tex, base, true, env.color);
        // 比一行宽的缩小（最多缩到一半），还宽就让它超出去。
        if b.w > width {
            let size = base * (width / b.w).max(0.5);
            b = self.math(tex, size, true, env.color);
        }
        let pad = base * 0.5;
        let (dx, baseline) = (x + ((width - b.w) / 2.0).max(0.0), y + pad + b.asc);
        out.items
            .extend(b.items.iter().map(|item| item.moved(dx, baseline, 0)));
        // 点击公式时光标移到公式的原文处，进入编辑。
        let height = pad * 2.0 + b.asc + b.desc;
        out.items.push(Item::Spot {
            rect: Rect::new(x, y, width, height),
            at: (self.chars, u32::try_from(source).unwrap_or(0)),
        });
        // 复制、查找时是公式的原文。
        self.text.push_str(tex);
        self.chars += tex.chars().count();
        self.end_paragraph();
        Flow {
            height,
            first_baseline: Some(baseline),
        }
    }

    /// 排一个可折叠的块：标题一行前加展开或收起的箭头，展开时在其下排 `blocks`。
    ///
    /// `id` 标识这个块，`open` 是原文中的初始状态；用户点击标题切换过时取相反的状态。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn details(
        &mut self,
        id: u64,
        open: bool,
        summary: &Inline,
        blocks: &[Block],
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let base = self.base;
        let para = self.body_para(env);
        let open = open != self.toggled.contains(&id);
        let indent = (base * 1.25).round();
        let flow = self.flow(summary, x + indent, y, width - indent, para, &mut out.items);
        let arrow = if open { "▾" } else { "▸" };
        self.marker(
            arrow,
            x + indent,
            y,
            flow.first_baseline,
            env.color,
            &mut out.items,
        );
        // 整行都能点；排在标题的链接后面，标题里的链接先认。
        out.items.push(Item::Link {
            rect: Rect::new(x, y, width, flow.height),
            url: format!("{DETAILS}{id:x}").into(),
        });
        if !open {
            return flow;
        }
        let top = y + flow.height + base * 0.5;
        let inner = self.blocks(blocks, x, top, width, env, out);
        Flow {
            height: top + inner.height - y,
            first_baseline: flow.first_baseline,
        }
    }

    /// 返回正文段落的样式：正文字号、不加粗、颜色取自 `env`。
    pub(super) fn body_para(&self, env: Env) -> Para {
        Para {
            size: self.base,
            bold: false,
            color: env.color,
            line: 1.5,
            align: Align::None,
        }
    }
}
