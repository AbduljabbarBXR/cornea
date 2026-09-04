/// Deterministic box-model layout engine.
/// v1 scope (documented): block-flow, inline runs (text estimating), basic
/// flex (row/column, no wrap), box model, z-index, visibility.
/// Box-sizing border-box and content-box supported.
use crate::css::{CssSheet, Styles, default_display, px, resolve};
use crate::dom::{ElNode, Node};
use crate::model::{ElementView, Rect, VisualModel};

pub const DEFAULT_WIDTH: f64 = 360.0;

#[derive(Debug, Clone, Default)]
struct ComputedBox {
    margin: [f64; 4],  // top, right, bottom, left
    padding: [f64; 4], // top, right, bottom, left
    border: [f64; 4],  // top, right, bottom, left
    width: Option<f64>,
    height: Option<f64>,
    box_sizing: BoxSizing,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Display {
    Block,
    Inline,
    FlexRow,
    FlexCol,
    None,
    Grid,
}

pub struct LayoutEngine<'a> {
    root: &'a Node,
    sheet: &'a CssSheet,
    elements: Vec<ElementView>,
}

impl<'a> LayoutEngine<'a> {
    pub fn new(root: &'a Node, sheet: &'a CssSheet) -> Self {
        LayoutEngine {
            root,
            sheet,
            elements: Vec::new(),
        }
    }

    pub fn run(mut self, viewport_w: f64) -> VisualModel {
        let root_box = Rect::new(0.0, 0.0, viewport_w, 0.0);
        self.elements.push(ElementView {
            id: 0,
            parent: 0,
            tag: "#document".to_string(),
            selector: "#document".to_string(),
            role: "document".to_string(),
            display: "block".to_string(),
            visible: true,
            bbox: root_box,
            content: root_box,
            z: 0,
            fg: None,
            bg: None,
            text: None,
            fidelity: "exact".into(),
        });
        let root_el = match self.root {
            Node::Elem(e) => e,
            // The root is always the converted html5ever Document node (see
            // dom::parse), which produces a top-level element, never a Text
            // node. Safe by construction — kept unreachable! for intent.
            Node::Text(_) => unreachable!("cornea: root must be an element"),
        };
        self.layout_block(root_el, root_box, &mut String::from(""), 0);
        let element_count = self.elements.len();
        VisualModel {
            viewport: root_box,
            element_count,
            elements: self.elements,
        }
    }

    fn display_of(&self, el: &ElNode) -> Display {
        let s = resolve(el, self.sheet);
        let d = s.get("display").unwrap_or_else(|| default_display(&el.tag));
        match d {
            "none" => Display::None,
            "inline" => Display::Inline,
            "flex" | "inline-flex" => Display::FlexRow,
            "grid" | "inline-grid" => Display::Grid,
            _ => Display::Block,
        }
    }

    /// Lay out a block-level element inside `containing` (its content box),
    /// recursing into its children. Selector path is shared/mutated by callers.
    #[allow(clippy::ptr_arg)] // selector path is thread-and-build, needs String
    fn layout_block(
        &mut self,
        el: &ElNode,
        containing: Rect,
        selector: &mut String,
        parent_id: usize,
    ) {
        let disp = self.display_of(el);

        if disp == Display::None {
            let s = function_sel(selector, el);
            self.push(
                el,
                Rect::new(0.0, 0.0, 0.0, 0.0),
                &s,
                disp,
                false,
                0,
                false,
                parent_id,
            );
            return;
        }

        let cb = self.compute_box(el);
        let z = self.z_of(el, 0);
        let content_w = (containing.w).max(0.0);
        // explicit width is not clamped, so overflow past the container/viewport
        // can be detected by the inspect layer
        let own_w = cb.width.unwrap_or(content_w);
        let own_h = self.measure_height(el, own_w);

        let x = containing.x + cb.margin[3];
        let y = containing.y + cb.margin[0];
        let (cw_p, ch_p) = content_size(own_w, own_h, &cb);

        let rect = Rect::new(x, y, own_w, ch_p);
        let mut s = function_sel(selector, el);
        self.push(el, rect, &s, disp, true, z, true, parent_id);

        // document node: children lay out in block flow at viewport origin
        if el.tag == "#document" {
            self.block_flow_children(el, Rect::new(0.0, 0.0, containing.w, 0.0), &mut s);
            return;
        }

        let content = Rect::new(
            rect.x + cb.padding[3] + cb.border[3],
            rect.y + cb.padding[0] + cb.border[0],
            cw_p,
            ch_p,
        );

        match disp {
            Display::FlexRow | Display::FlexCol => self.layout_flex_children(el, content, &mut s),
            Display::Inline => {}
            _ => self.block_flow_children(el, content, &mut s),
        }
    }

    /// Stack block children vertically; run inline children in horizontal runs.
    #[allow(clippy::ptr_arg)] // selector path builder takes &mut String
    fn block_flow_children(&mut self, el: &ElNode, content: Rect, sel: &mut String) {
        let mut cursor = content.y;
        let mut x_cursor = content.x;
        let mut line_top = content.y;
        let mut line_height = 0.0;
        for child in el.children.iter() {
            let Node::Elem(c) = child else { continue };
            // absolutely/fixed-positioned children are taken out of flow
            if let Some(ar) = self.absolute_rect(c, content) {
                let z = self.z_of(c, 0);
                let s = function_sel(sel, c);
                self.push(c, ar, &s, Display::Block, true, z, true, el.id);
                continue;
            }
            match self.display_of(c) {
                Display::None => {
                    let s = function_sel(sel, c);
                    self.push(
                        c,
                        Rect::new(0.0, 0.0, 0.0, 0.0),
                        &s,
                        Display::None,
                        false,
                        0,
                        false,
                        el.id,
                    );
                }
                Display::Inline => {
                    // a block child ended the previous line: start a fresh line
                    if x_cursor == content.x && line_height == 0.0 {
                        line_top = line_top.max(cursor);
                    }
                    let m = self.inline_measure(c);
                    let needs_wrap = x_cursor + m.w > content.x + content.w && x_cursor > content.x;
                    if needs_wrap {
                        line_top += line_height;
                        x_cursor = content.x;
                        line_height = 0.0;
                    }
                    let r = Rect::new(x_cursor, line_top, m.w, m.h);
                    let s = function_sel(sel, c);
                    let z = self.z_of(c, 0);
                    self.push(c, r, &s, Display::Inline, true, z, true, el.id);
                    x_cursor += m.w;
                    line_height = line_height.max(m.h);
                }
                _ => {
                    // commit any pending inline line before the block child
                    if line_height > 0.0 {
                        cursor = (line_top + line_height).max(cursor);
                    }
                    self.layout_block(
                        c,
                        Rect::new(content.x, cursor, content.w, content.h),
                        sel,
                        el.id,
                    );
                    if let Some(last) = self.elements.iter().rev().find(|e| e.id == c.id) {
                        cursor = last.bbox.y + last.bbox.h;
                    }
                    x_cursor = content.x;
                    line_height = 0.0;
                    line_top = cursor;
                }
            }
        }
    }

    /// Resolve a rect for an absolutely/fixed-positioned element with left/top,
    /// relative to the containing block. Returns None if not positioned.
    fn absolute_rect(&self, el: &ElNode, containing: Rect) -> Option<Rect> {
        let s = resolve(el, self.sheet);
        let pos = s.get("position").unwrap_or("");
        if pos != "absolute" && pos != "fixed" {
            return None;
        }
        let left = s.get("left").and_then(|v| px(v, 16.0)).unwrap_or(0.0);
        let top = s.get("top").and_then(|v| px(v, 16.0)).unwrap_or(0.0);
        let w = px(s.get("width").unwrap_or("0"), 16.0).unwrap_or(0.0);
        let h = px(s.get("height").unwrap_or("0"), 16.0).unwrap_or(20.0);
        Some(Rect::new(
            containing.x + left,
            containing.y + top,
            w.max(0.0),
            h.max(0.0),
        ))
    }

    #[allow(clippy::ptr_arg)] // selector path builder takes &mut String
    fn layout_flex_children(&mut self, el: &ElNode, content: Rect, sel: &mut String) {
        let kids: Vec<&ElNode> = el
            .children
            .iter()
            .filter_map(|c| if let Node::Elem(e) = c { Some(e) } else { None })
            .collect();

        // simplified flex: column stacking (most common), row distributes evenly
        if self.display_of(el) == Display::FlexCol {
            let mut cursor = content.y;
            for k in &kids {
                if self.display_of(k) == Display::None {
                    let s = function_sel(sel, k);
                    self.push(
                        k,
                        Rect::new(0.0, 0.0, 0.0, 0.0),
                        &s,
                        Display::None,
                        false,
                        0,
                        false,
                        el.id,
                    );
                    continue;
                }
                let cb = self.compute_box(k);
                let w = cb.width.unwrap_or(content.w).min(content.w);
                let h = self.measure_height(k, w);
                let r = Rect::new(content.x + cb.margin[3], cursor + cb.margin[0], w, h);
                self.push(
                    k,
                    r,
                    &function_sel(sel, k),
                    self.display_of(k),
                    true,
                    self.z_of(k, 0),
                    true,
                    el.id,
                );
                cursor += h + cb.margin[0] + cb.margin[2];
            }
            return;
        }

        // flex row: divide width evenly across kids
        let available = content.w;
        let n = kids.len().max(1) as f64;
        let slot = available / n;
        for (i, k) in kids.iter().enumerate() {
            if self.display_of(k) == Display::None {
                let s = function_sel(sel, k);
                self.push(
                    k,
                    Rect::new(0.0, 0.0, 0.0, 0.0),
                    &s,
                    Display::None,
                    false,
                    0,
                    false,
                    el.id,
                );
                continue;
            }
            let cb = self.compute_box(k);
            let w = slot - cb.margin[3] - cb.margin[1];
            let h = self.measure_height(k, w);
            let r = Rect::new(
                content.x + i as f64 * slot + cb.margin[3],
                content.y,
                w.max(0.0),
                h,
            );
            self.push(
                k,
                r,
                &function_sel(sel, k),
                self.display_of(k),
                true,
                self.z_of(k, 0),
                true,
                el.id,
            );
        }
    }

    fn inline_measure(&self, el: &ElNode) -> Rect {
        // rough glyph estimate: ~13px per visible char on 16px font
        let mut t = String::new();
        collect_text(el, &mut t);
        let chars = t.chars().count() as f64;
        let s = resolve(el, self.sheet);
        let h = px(s.get("height").unwrap_or("1em"), 16.0).unwrap_or(16.0);
        Rect::new(0.0, 0.0, chars * 13.0, h)
    }

    fn compute_box(&self, el: &ElNode) -> ComputedBox {
        let s = resolve(el, self.sheet);
        let mut cb = ComputedBox::default();
        let sides = ["top", "right", "bottom", "left"];
        for (i, side) in sides.iter().enumerate() {
            cb.margin[i] = px(&prop(&s, &format!("margin-{}", side)), 16.0).unwrap_or(0.0);
            cb.padding[i] = px(&prop(&s, &format!("padding-{}", side)), 16.0).unwrap_or(0.0);
            cb.border[i] = border_px(&prop(&s, &format!("border-{}-width", side)), 16.0);
        }
        cb.width = s.get("width").and_then(|v| px(v, 16.0));
        if cb.width.is_none() {
            cb.width = s.get("max-width").and_then(|v| px(v, 16.0));
        }
        cb.height = s.get("height").and_then(|v| px(v, 16.0));
        cb.box_sizing = if s
            .get("box-sizing")
            .map(|v| v == "border-box")
            .unwrap_or(false)
        {
            BoxSizing::BorderBox
        } else {
            BoxSizing::ContentBox
        };
        cb
    }

    fn z_of(&self, el: &ElNode, parent_z: i32) -> i32 {
        let s = resolve(el, self.sheet);
        s.get("z-index")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(parent_z)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn measure_height(&self, el: &ElNode, w: f64) -> f64 {
        let s = resolve(el, self.sheet);
        if let Some(h) = s.get("height").and_then(|v| px(v, 16.0)) {
            return h;
        }
        if let Some(h) = el
            .attr("height")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|_| el.tag == "img" || el.tag == "svg")
        {
            return h;
        }
        // sum of block children + padding
        let own_pad = self.compute_box(el);
        let mut sum = 0.0;
        for child in el.children.iter() {
            if let Node::Elem(c) = child {
                match self.display_of(c) {
                    Display::None => {}
                    Display::Inline => sum += self.inline_measure(c).h,
                    _ => sum += self.measure_height(c, w) + 0.0,
                }
            }
        }
        if sum > 0.0 {
            sum + own_pad.padding[0] + own_pad.padding[2] + own_pad.border[0] + own_pad.border[2]
        } else if has_significant_text(el) {
            20.0 + own_pad.padding[0] + own_pad.padding[2]
        } else {
            0.0
        }
    }

    #[allow(clippy::too_many_arguments)] // element-view builder
    fn push(
        &mut self,
        el: &ElNode,
        rect: Rect,
        selector: &str,
        display: Display,
        visible: bool,
        z: i32,
        include_content: bool,
        parent_id: usize,
    ) {
        let s = resolve(el, self.sheet);
        let cb = self.compute_box(el);
        let visible = visible
            && rect.w > 0.0
            && rect.h > 0.0
            && !(rect.x + rect.w < 0.0 || rect.y + rect.h < 0.0);
        let content = if include_content {
            Rect::new(
                rect.x + cb.padding[3] + cb.border[3],
                rect.y + cb.padding[0] + cb.border[0],
                (rect.w - cb.padding[3] - cb.padding[1] - cb.border[3] - cb.border[1]).max(0.0),
                (rect.h - cb.padding[0] - cb.padding[2] - cb.border[0] - cb.border[2]).max(0.0),
            )
        } else {
            rect
        };
        self.elements.push(ElementView {
            id: el.id,
            parent: parent_id,
            tag: el.tag.clone(),
            selector: selector.to_string(),
            role: role(el),
            display: display_str(display),
            visible,
            bbox: rect,
            content,
            z,
            fg: s.get("color").map(|v| v.to_string()),
            bg: s.get("background-color").map(|v| v.to_string()),
            text: text_if_small(el),
            fidelity: "exact".into(),
        });
    }
}

fn prop(s: &Styles, key: &str) -> String {
    s.get(key).unwrap_or("0").to_string()
}

fn function_sel(selector: &str, el: &ElNode) -> String {
    if let Some(id) = el.attr("id") {
        if id.contains(char::is_whitespace) {
            return format!("{}[{}#{}]", selector, el.tag, id);
        }
        return format!("{}#{}", selector, id);
    }
    if let Some(cls) = el.attr("class")
        && let Some(first) = cls.split_whitespace().next()
    {
        return format!("{}>{}.{}", selector, el.tag, first);
    }
    format!("{}>{}", selector, el.tag)
}

fn content_size(w: f64, own_h: f64, cb: &ComputedBox) -> (f64, f64) {
    match cb.box_sizing {
        BoxSizing::BorderBox => {
            let cw = (w - cb.padding[3] - cb.padding[1] - cb.border[3] - cb.border[1]).max(0.0);
            let ch = (own_h - cb.padding[0] - cb.padding[2] - cb.border[0] - cb.border[2]).max(0.0);
            (cw, ch)
        }
        BoxSizing::ContentBox => (w, own_h),
    }
}

fn border_px(v: &str, fs: f64) -> f64 {
    let t = v.trim();
    if t == "0" || t.is_empty() || t == "none" || t == "medium" || t == "thin" || t == "thick" {
        return 0.0;
    }
    px(t, fs).unwrap_or(0.0)
}

fn display_str(d: Display) -> String {
    match d {
        Display::Block => "block".into(),
        Display::Inline => "inline".into(),
        Display::FlexRow => "flex".into(),
        Display::FlexCol => "flex".into(),
        Display::Grid => "grid".into(),
        Display::None => "none".into(),
    }
}

fn role(el: &ElNode) -> String {
    if let Some(r) = el.attr("role") {
        return r.to_string();
    }
    match el.tag.as_str() {
        "button" | "input" | "select" | "textarea" | "a" | "link" => "interactive".into(),
        "img" | "picture" | "svg" => "image".into(),
        "h1" | "h2" | "h3" | "h4" | "p" | "span" | "label" | "li" | "strong" | "em" => {
            "text".into()
        }
        "header" | "nav" | "main" | "footer" | "section" | "aside" | "article" | "div" => {
            "container".into()
        }
        _ => "element".into(),
    }
}

fn text_if_small(el: &ElNode) -> Option<String> {
    let mut t = String::new();
    collect_text(el, &mut t);
    let t = t.trim().to_string();
    if t.is_empty() || t.chars().count() > 60 {
        None
    } else {
        Some(t)
    }
}

fn collect_text(el: &ElNode, out: &mut String) {
    for c in &el.children {
        match c {
            Node::Text(s) => out.push_str(s),
            Node::Elem(e) => collect_text(e, out),
        }
    }
}

fn has_significant_text(el: &ElNode) -> bool {
    let mut t = String::new();
    collect_text(el, &mut t);
    !t.trim().is_empty()
}
