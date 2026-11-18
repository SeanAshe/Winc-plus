//! 首次设置：第一次运行时开始页只显示语言、外观、主题和模块几项，点击「继续」后是快捷键一页
//! （选用键位方案，或从本机的其他编辑器导入），点击「完成」后换回开始操作和最近目录。页面出现
//! 之前播放一段启动动画：应用图标在窗口中央逐层出现（底板、丝带、圆点），停留片刻后淡出，
//! 之后标题和各行逐个淡入。

use std::time::Duration;

use super::animation::stage;
use super::*;
use crate::keymap::Preset;

/// 启动动画的时长（毫秒）：图标逐层出现、停留、淡出，之后内容淡入，各阶段占的比例见
/// [`BUILD_END`]、[`FADE`] 和 [`CONTENT_START`]。
///
/// 取 2200 毫秒时，图标逐层出现用 1.1 秒（每一层 0.3 秒以上），停留约 0.26 秒，淡出与内容
/// 的淡入共约 0.84 秒。
pub(in crate::app) const INTRO: Duration = Duration::from_millis(2200);

/// 图标逐层出现完成时的进度（占整段动画的比例）。
const BUILD_END: f32 = 0.50;

/// 图标淡出的起止进度：之前图标完整地停留在窗口中央。
const FADE: (f32, f32) = (0.62, 0.78);

/// 内容开始淡入的进度；之前页面上只有图标，各控件不响应点击。
pub(crate) const CONTENT_START: f32 = 0.70;

/// 图标淡出途中向上移动的距离（逻辑像素）。
const SPLASH_RISE: f32 = 12.0;

/// 点击「完成」之后从「首次设置」换到开始页的时长（毫秒）：前段「首次设置」淡出，后段开始页
/// 淡入，各自的起止见 [`outro_opacity`]。
///
/// 取自调参，没有在真实窗口中观察过：淡出约 170 毫秒、淡入约 210 毫秒，与设置浮层出现的时长
/// 相近。两段不重叠：两个页面的标题位置不同，同时半透明时标题叠成两层。
pub(in crate::app) const OUTRO: Duration = Duration::from_millis(380);

/// 应用图标，与各平台安装包的图标同一份。
pub(crate) const ICON: &str = include_str!("../../assets/icon.svg");

/// 图标中底板、丝带和圆点三层各自开始的位置（SVG 源码中的标记）。
const LAYERS: [&str; 3] = ["<!-- 四周透明 -->", "<!-- 丝带轮廓与折面 -->", "<circle"];

/// 丝带从左到右显露时，裁剪矩形右边缘的起点和终点（SVG 用户单位）：略宽于丝带的左右两端。
const RIBBON_SPAN: (f32, f32) = (150.0, 860.0);

/// 圆点落下之前在最终位置上方的距离（SVG 用户单位），约为圆点直径。
const DOT_DROP: f32 = 120.0;

/// 「首次设置」中快捷键一页的状态。
#[derive(Debug, Default)]
pub(crate) struct SetupKeys {
    /// 是否在显示这一页。
    pub shown: bool,
    /// 本机可以导入快捷键的来源，进入这一页时查找。
    pub sources: Vec<crate::keymap::import::Source>,
    /// 选中的一项：0 和 1 依次是 [`Preset::ALL`] 中的方案，其后依次是各来源。
    pub choice: usize,
    /// 是否在这一页点击了「完成」：「首次设置」淡出时绘制的是这一页。
    pub leaving: bool,
}

impl App {
    /// 开始页是否正在显示「首次设置」：首次设置尚未完成，且没有打开目录，也没有打开的标签。
    ///
    /// 是否完成记在状态中（`State::setup_pending`），各窗口和各进程看到的是同一份。
    pub(crate) fn setup_shown(&self) -> bool {
        self.state.setup_pending && self.editor.workspace.is_none() && self.editor.active.is_none()
    }

    /// 窗口建好后调用：开始页显示「首次设置」时播放启动动画。
    ///
    /// 动态效果关闭时没有时钟，页面直接显示最终的样子。
    pub(in crate::app) fn start_setup_intro(&mut self) {
        if self.setup_shown() {
            self.motion.setup_intro = self.clock(INTRO);
        }
    }

    /// 处理开始页显示「首次设置」时的按键 `key`，返回是否已处理：回车等同于点击「继续」或
    /// 「完成」；在快捷键一页，上下键换选中的一项，`Esc` 回到上一页。
    ///
    /// 启动动画期间不处理：此时各项尚未显示，启动应用时仍按着的回车不应结束首次设置。按住不放
    /// 产生的重复按键由调用方排除。
    pub(in crate::app) fn setup_key(&mut self, key: &winit::keyboard::Key) -> bool {
        use winit::keyboard::{Key, NamedKey};
        if !self.setup_shown() || self.motion.setup_intro.is_some() {
            return false;
        }
        let Key::Named(named) = key else {
            return false;
        };
        let last = Preset::ALL.len() + self.setup.sources.len() - 1;
        match (named, self.setup.shown) {
            (NamedKey::Enter, false) => self.setup_next(),
            (NamedKey::Enter, true) => self.finish_setup(),
            (NamedKey::Escape, true) => self.setup_back(),
            (NamedKey::ArrowUp, true) => self.setup_choose(self.setup.choice.saturating_sub(1)),
            (NamedKey::ArrowDown, true) => self.setup_choose((self.setup.choice + 1).min(last)),
            _ => return false,
        }
        true
    }

    /// 进入快捷键一页：查找本机可以导入快捷键的来源。
    pub(in crate::app) fn setup_next(&mut self) {
        self.setup.sources = crate::keymap::import::find_sources();
        self.setup.choice = self
            .setup
            .choice
            .min(Preset::ALL.len() + self.setup.sources.len() - 1);
        self.turn_setup_page(true);
    }

    /// 从快捷键一页回到上一页。
    pub(in crate::app) fn setup_back(&mut self) {
        self.turn_setup_page(false);
    }

    /// 换到「首次设置」的第一页或快捷键一页（`keys`）：当前一页先淡出，另一页随后淡入，与
    /// 结束首次设置的过渡相同（见 [`OUTRO`]）。
    fn turn_setup_page(&mut self, keys: bool) {
        if self.setup.shown != keys {
            self.setup.shown = keys;
            self.motion.setup_page = self.clock(OUTRO);
        }
        self.request_redraw();
    }

    /// 在快捷键一页选中第 `choice` 项（见 [`SetupKeys::choice`]）。
    pub(in crate::app) fn setup_choose(&mut self, choice: usize) {
        self.setup.choice = choice;
        self.request_redraw();
    }

    /// 采用快捷键一页选中的一项：选用方案，或从来源导入（结果用提示说明）。
    fn apply_setup_keys(&mut self) {
        let choice = self.setup.choice;
        match Preset::ALL.get(choice) {
            Some(&preset) => self.set_keymap_preset(preset),
            None => {
                let index = choice - Preset::ALL.len();
                if let Some(source) = self.setup.sources.get(index).cloned() {
                    self.import_keymap_from(&source);
                }
            }
        }
    }

    /// 结束首次设置：开始页换回开始操作和最近目录，之后的启动不再显示。
    ///
    /// 「首次设置」先淡出，开始页随后淡入（见 [`OUTRO`]）；动态效果关闭时直接换过去。
    pub(in crate::app) fn finish_setup(&mut self) {
        self.setup.leaving = std::mem::take(&mut self.setup.shown);
        if self.setup.leaving {
            self.apply_setup_keys();
        }
        self.motion.setup_intro = None;
        self.motion.setup_page = None;
        self.motion.setup_outro = self.clock(OUTRO);
        self.update_state(|state| state.setup_pending = false);
        self.request_redraw();
    }
}

/// 回弹缓动：从 0 到 1，途中略微越过 1 再回到 1。`x` 为 0 到 1 的线性进度。
fn overshoot(x: f32) -> f32 {
    // 标准的 back-out 曲线，回弹量 1.70158 使峰值约超出终点 10%。
    const BACK: f32 = 1.701_58;
    let x = x.clamp(0.0, 1.0) - 1.0;
    1.0 + (BACK + 1.0) * x * x * x + BACK * x * x
}

/// 返回 `t` 在 `start` 到 `end` 之间的线性进度，0 到 1。
fn span(t: f32, start: f32, end: f32) -> f32 {
    ((t - start) / (end - start)).clamp(0.0, 1.0)
}

/// 启动动画进行到某一时刻时窗口中央图标的样子。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Splash {
    /// 图标逐层出现的进度，0 到 1（见 [`logo_frame`]）。
    pub build: f32,
    /// 图标的不透明度，0 到 1。
    pub opacity: f32,
    /// 图标比最终位置高出的距离（逻辑像素）：淡出途中向上移动。
    pub rise: f32,
}

/// 返回启动动画进行到 `t`（0 到 1，未加缓动）时窗口中央图标的样子；图标已经淡出时返回 `None`。
pub(crate) fn splash(t: f32) -> Option<Splash> {
    let fade = stage(t, FADE.0, FADE.1);
    (fade < 1.0).then(|| Splash {
        build: span(t, 0.0, BUILD_END),
        opacity: 1.0 - fade,
        rise: SPLASH_RISE * fade,
    })
}

/// 返回逐层出现进行到 `build`（0 到 1）时应用图标的 RGBA 像素，边长 `size`（物理像素）。
///
/// 三层依次出现：底板淡入并从略小回弹到原大，丝带从左到右显露，圆点从上方落下并回弹。`build`
/// 为 1 时与应用图标相同。图标的源码中找不到三层的标记时始终返回完整的图标。
pub(crate) fn logo_frame(size: u32, build: f32) -> Vec<u8> {
    let svg = posed(build).unwrap_or_else(|| ICON.to_owned());
    winc_preview::svg::load(&svg, None, None)
        .map(|picture| picture.render(size, size).rgba)
        .unwrap_or_default()
}

/// 返回逐层出现进行到 `build` 时图标的 SVG 源码：在三层外各套一个分组，写入当前的变换、
/// 不透明度和裁剪；找不到三层的标记时返回 `None`。
fn posed(build: f32) -> Option<String> {
    use std::fmt::Write as _;
    let tile_at = ICON.find(LAYERS[0])?;
    let ribbon_at = tile_at + ICON[tile_at..].find(LAYERS[1])?;
    let dot_at = ribbon_at + ICON[ribbon_at..].find(LAYERS[2])?;
    let end = dot_at + ICON[dot_at..].find("</svg>")?;

    let tile_opacity = stage(build, 0.0, 0.26);
    let tile_scale = 0.82 + 0.18 * overshoot(span(build, 0.0, 0.42));
    let reveal = stage(build, 0.28, 0.82);
    let right = RIBBON_SPAN.0 + (RIBBON_SPAN.1 - RIBBON_SPAN.0) * reveal;
    let dot_opacity = stage(build, 0.70, 0.84);
    let dot_rise = DOT_DROP * (1.0 - overshoot(span(build, 0.70, 1.0)));

    let mut svg = String::with_capacity(ICON.len() + 512);
    svg.push_str(&ICON[..tile_at]);
    let scale = format!("scale({tile_scale:.4},{tile_scale:.4})");
    let about_center = format!("translate(512,512) {scale} translate(-512,-512)");
    let _ = write!(
        svg,
        r#"<g opacity="{tile_opacity:.3}" transform="{about_center}">"#
    );
    svg.push_str(&ICON[tile_at..ribbon_at]);
    svg.push_str("</g>");
    let shown = format!(r#"<rect x="0" y="0" width="{right:.1}" height="1024"/>"#);
    let _ = write!(
        svg,
        r#"<defs><clipPath id="introReveal">{shown}</clipPath></defs>"#
    );
    svg.push_str(r#"<g clip-path="url(#introReveal)">"#);
    svg.push_str(&ICON[ribbon_at..dot_at]);
    svg.push_str("</g>");
    let _ = write!(
        svg,
        r#"<g opacity="{dot_opacity:.3}" transform="translate(0,{:.1})">"#,
        -dot_rise
    );
    svg.push_str(&ICON[dot_at..end]);
    svg.push_str("</g>");
    svg.push_str(&ICON[end..]);
    Some(svg)
}

/// 返回结束首次设置的过渡进行到 `t`（0 到 1，未加缓动）时，「首次设置」和开始页各自的不透明度，
/// 0 到 1：前者在前 45% 淡出，后者在其后淡入。
pub(crate) fn outro_opacity(t: f32) -> (f32, f32) {
    (1.0 - stage(t, 0.0, 0.45), stage(t, 0.45, 1.0))
}

/// 返回启动动画进行到 `t`（0 到 1，未加缓动）时第 `index` 项内容的不透明度，0 到 1。
///
/// 内容从 [`CONTENT_START`] 起逐个淡入：标题是第 0 项，其后是说明、各行和「完成」按钮，每一项
/// 比前一项晚开始。
pub(crate) fn content_opacity(t: f32, index: usize) -> f32 {
    let start = CONTENT_START + index as f32 * 0.022;
    stage(t, start, start + 0.12)
}

#[cfg(test)]
mod tests;
