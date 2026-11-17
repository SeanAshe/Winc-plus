//! 样式：属性的继承、`style` 和 CSS 里的声明、元素自己的属性。

use super::*;

impl<'a, 'input> Parser<'a, 'input> {
    /// 计算元素 `node` 的样式。
    ///
    /// 先继承父元素 `parent` 的样式，再依次应用属性、`<style>` 里的规则和 `style` 属性，
    /// 后两者覆盖前面的同名声明。
    pub(super) fn props(&mut self, node: Node<'a, 'input>, parent: &Props) -> Props {
        let declared = self.declared(node);
        self.props_from(&declared, parent)
    }

    /// 与 [`Parser::props`] 相同，但声明 `declared` 已经取好。
    ///
    /// [`Parser::declared`] 需要匹配 CSS 规则，同一个元素的继承样式和自有属性共用同一份声明。
    pub(super) fn props_from(&mut self, declared: &[Declaration], parent: &Props) -> Props {
        let mut props = parent.clone();
        for (name, value) in declared {
            self.apply(&mut props, parent, name, value);
        }
        props
    }

    /// 返回 `node` 从根元素一路继承下来的样式。
    ///
    /// 裁剪、蒙版、滤镜的内容使用内容所在位置的样式，不使用引用这些内容的元素的样式。
    pub(super) fn inherited(&mut self, node: Node<'a, 'input>) -> Props {
        let mut chain: Vec<Node> = node.ancestors().filter(Node::is_element).collect();
        chain.reverse();
        let mut props = Props::default();
        for n in chain {
            props = self.props(n, &props);
        }
        props
    }

    /// 返回一个元素上写的所有样式声明，按优先级从低到高排列：后面的覆盖前面的。
    ///
    /// 每访问一次元素都要取一遍（`<use>` 每展开一次、渐变每被引用一次也是），而 `d`、
    /// 内嵌图片的 `href` 这类属性值可以达到数 MB，因此只引用原文，不复制。
    pub(super) fn declared(&self, node: Node<'a, 'input>) -> Vec<Declaration<'a>> {
        let mut out: Vec<Declaration<'a>> = node
            .attributes()
            .filter(|a| a.namespace().is_none())
            .map(|a| (Cow::Borrowed(a.name()), a.value()))
            .collect();
        out.extend(
            self.sheet
                .matching(node)
                .map(|(name, value)| (Cow::Borrowed(name), value)),
        );
        if let Some(style) = node.attribute("style") {
            out.extend(css::declarations(style));
        }
        out
    }

    /// 把一条声明 `name: value` 应用到 `props`；字体相关的属性交给 `text::apply_font` 处理。
    fn apply(&mut self, props: &mut Props, parent: &Props, name: &str, value: &str) {
        let value = value.trim();
        if value == "inherit" {
            return;
        }
        let diagonal = self.diagonal();
        match name {
            // 无法解析的声明不生效，保留优先级较低的声明给出的值。
            "fill" => props.fill = paint(value).unwrap_or_else(|| props.fill.clone()),
            "stroke" => props.stroke = paint(value).unwrap_or_else(|| props.stroke.clone()),
            "fill-opacity" => props.fill_opacity = opacity(value),
            "stroke-opacity" => props.stroke_opacity = opacity(value),
            "fill-rule" | "clip-rule" => {
                let rule = if value == "evenodd" {
                    Fill::EvenOdd
                } else {
                    Fill::NonZero
                };
                if name == "fill-rule" {
                    props.fill_rule = rule;
                } else {
                    props.clip_rule = rule;
                }
            }
            "stroke-width" => {
                if let Some(w) = length(value, diagonal).filter(|w| *w >= 0.0) {
                    props.stroke_width = w;
                }
            }
            "stroke-linecap" => {
                props.cap = match value {
                    "round" => Cap::Round,
                    "square" => Cap::Square,
                    _ => Cap::Butt,
                }
            }
            "stroke-linejoin" => {
                props.join = match value {
                    "round" => Join::Round,
                    "bevel" => Join::Bevel,
                    _ => Join::Miter,
                }
            }
            "stroke-miterlimit" => {
                if let Ok(m) = value.parse::<f32>() {
                    props.miter = m.max(1.0);
                }
            }
            "stroke-dasharray" => {
                let mut dashes: Vec<f32> = value
                    .split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .filter_map(|s| length(s, diagonal))
                    .collect();
                if dashes.iter().any(|d| *d < 0.0) || dashes.iter().sum::<f32>() <= 0.0 {
                    dashes.clear();
                } else if dashes.len() % 2 == 1 {
                    dashes.extend_from_within(..);
                }
                props.dashes = dashes.into();
            }
            "stroke-dashoffset" => props.dash_offset = length(value, diagonal).unwrap_or(0.0),
            "color" => {
                if let Some(c) = color(value) {
                    props.color = c;
                }
            }
            "visibility" => props.visible = value == "visible",
            "paint-order" => {
                let words: Vec<&str> = value.split_whitespace().collect();
                let at = |w: &str| words.iter().position(|x| *x == w);
                props.stroke_first = match (at("stroke"), at("fill")) {
                    (Some(s), Some(f)) => s < f,
                    (Some(_), None) => true,
                    _ => false,
                };
            }
            "color-interpolation-filters" => props.filters_linear = value != "sRGB",
            "image-rendering" => {
                props.pixelated = matches!(value, "pixelated" | "crisp-edges" | "optimizeSpeed")
            }
            "marker-start" | "marker-mid" | "marker-end" | "marker" if value != "none" => {
                self.miss(winc_i18n::t!("preview-svg-missing-markers"))
            }
            "animation" | "animation-name" if value != "none" => {
                self.miss(winc_i18n::t!("preview-svg-missing-animation"))
            }
            _ => text::apply_font(&mut props.font, &parent.font, name, value, diagonal),
        }
    }

    /// 返回当前元素自己的（不继承的）属性。
    pub(super) fn own(&self, node: Node<'a, 'input>) -> Own {
        Self::own_from(&self.declared(node))
    }

    /// 与 [`Parser::own`] 相同，但声明 `declared` 已经取好。
    pub(super) fn own_from(declared: &[Declaration]) -> Own {
        let mut own = Own {
            opacity: 1.0,
            shown: true,
            transform: Affine::IDENTITY,
            clip_path: None,
            mask: None,
            filter: None,
            overflow_visible: false,
            non_scaling: false,
            mask_alpha: false,
            blend: filter::Blend::Normal,
        };
        let url = |v: &str| {
            let v = v.trim();
            let inside = v.strip_prefix("url(")?.split_once(')')?.0;
            let id = inside.trim().trim_matches(['"', '\'']);
            Some(id.strip_prefix('#').unwrap_or(id).to_owned())
        };
        for &(ref name, value) in declared {
            match name.as_ref() {
                "opacity" => own.opacity = opacity(value),
                "display" => own.shown = value.trim() != "none",
                "transform" => own.transform = transform(value),
                "clip-path" => own.clip_path = url(value),
                "mask" => own.mask = url(value),
                "filter" => {
                    own.filter = match value.trim() {
                        "none" | "" => None,
                        v if v.starts_with("url(") => url(v).map(FilterRef::Url),
                        v => Some(FilterRef::Functions(v.to_owned())),
                    }
                }
                "overflow" => own.overflow_visible = matches!(value.trim(), "visible" | "auto"),
                "vector-effect" => own.non_scaling = value.trim() == "non-scaling-stroke",
                "mask-type" => own.mask_alpha = value.trim() == "alpha",
                "mix-blend-mode" => own.blend = filter::blend_mode(value),
                _ => {}
            }
        }
        own
    }
}
