/// Minimal CSS resolution for v1.
/// Sources, in precedence order:
///   1. inline `style` attribute (highest)
///   2. stylesheet rules matched by class / id / tag (specificity ranked)
///   3. presentational attributes
///
/// Stylesheet support: `<style>` blocks parsed for simple selectors:
/// `.class`, `#id`, `tag`, `tag.class`, `tag#id`, and comma-separated groups.
/// Combinators (space `>`, `+`, `~`), pseudo-classes and @media are deferred.
use crate::dom::ElNode;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub enum SelectorPart {
    #[default]
    Empty,
    Class(String),
    Id(String),
    Tag(String),
}

#[derive(Debug, Clone, Default)]
pub struct Selector {
    pub parts: Vec<SelectorPart>,
}

impl Selector {
    pub fn specificity(&self) -> u32 {
        let mut classes = 0u32;
        let mut ids = 0u32;
        let mut tags = 0u32;
        for p in &self.parts {
            match p {
                SelectorPart::Class(_) => classes += 1,
                SelectorPart::Id(_) => ids += 1,
                SelectorPart::Tag(_) => tags += 1,
                SelectorPart::Empty => {}
            }
        }
        ids * 1000 + classes * 100 + tags
    }

    pub fn matches(&self, el: &ElNode) -> bool {
        if self.parts.is_empty() {
            return false;
        }
        let mut class_ok = true;
        let mut id_ok = true;
        let mut tag_ok = true;
        let mut any_class = false;
        let mut any_id = false;
        let mut any_tag = false;
        let mut req_class: Option<&str> = None;
        let mut req_id: Option<&str> = None;
        let mut req_tag: Option<&str> = None;

        for p in &self.parts {
            match p {
                SelectorPart::Class(c) => {
                    any_class = true;
                    req_class = Some(c);
                }
                SelectorPart::Id(i) => {
                    any_id = true;
                    req_id = Some(i);
                }
                SelectorPart::Tag(t) => {
                    any_tag = true;
                    req_tag = Some(t);
                }
                SelectorPart::Empty => {}
            }
        }
        if let Some(c) = req_class {
            class_ok = el
                .attr("class")
                .map(|cls| cls.split_whitespace().any(|x| x == c))
                .unwrap_or(false);
        }
        if let Some(i) = req_id {
            id_ok = el.attr("id").map(|x| x == i).unwrap_or(false);
        }
        if let Some(t) = req_tag {
            tag_ok = el.tag == t;
        }
        let _ = (any_class, any_id, any_tag);
        class_ok && id_ok && tag_ok
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub block: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct CssSheet {
    pub rules: Vec<Rule>,
}

impl CssSheet {
    /// Extract `<style>` blocks from raw HTML and parse them.
    pub fn from_html(html: &str) -> Self {
        let mut sheet = CssSheet::default();
        let mut rest = html;
        while let Some(start) = rest.find("<style") {
            let after_tag = &rest[start..];
            let gt = after_tag.find('>').map(|i| start + i + 1);
            let Some(content_start) = gt else { break };
            let content = &rest[content_start..];
            let close = content.find("</style").unwrap_or(content.len());
            let css_text = &content[..close];
            sheet.parse_css(css_text);
            rest = &content[close..];
        }
        sheet
    }

    /// Parse a raw CSS string into rules.
    pub fn parse_css(&mut self, css: &str) {
        let mut rest = css;
        while let Some(open) = rest.find('{') {
            let selector_part = rest[..open].trim();
            let close = rest[open..].find('}').map(|i| open + i);
            let Some(close) = close else { break };
            let block = &rest[open + 1..close];
            rest = &rest[close + 1..];

            let selectors: Vec<Selector> = selector_part
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| parse_selector(s.trim()))
                .filter(|sel| !sel.parts.is_empty())
                .collect();

            if selectors.is_empty() {
                continue;
            }

            let mut props = HashMap::new();
            for decl in block.split(';') {
                let decl = decl.trim();
                if let Some((p, v)) = decl.split_once(':') {
                    let p = p.trim().to_lowercase();
                    let v = v.trim();
                    if !p.is_empty() && !v.is_empty() {
                        props.insert(p, v.to_string());
                    }
                }
            }
            if props.is_empty() {
                continue;
            }
            self.rules.push(Rule {
                selectors,
                block: props,
            });
        }
    }

    /// Collect stylesheet properties that match an element, ranked by specificity.
    fn sheet_props_for(&self, el: &ElNode) -> HashMap<String, (u32, String)> {
        let mut gathered: HashMap<String, (u32, String)> = HashMap::new();
        for rule in &self.rules {
            for sel in &rule.selectors {
                if sel.matches(el) {
                    let spec = sel.specificity();
                    for (k, v) in &rule.block {
                        let cur = gathered.get(k).map(|(c, _)| *c).unwrap_or(0);
                        if spec > cur {
                            gathered.insert(k.clone(), (spec, v.clone()));
                        }
                    }
                }
            }
        }
        gathered
    }
}

fn parse_selector(s: &str) -> Selector {
    let mut parts = Vec::new();
    let mut i = 0usize;
    let chars: Vec<char> = s.chars().collect();
    while i < chars.len() {
        match chars[i] {
            '.' => {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && is_ident_char(chars[j]) {
                    j += 1;
                }
                if j > start {
                    parts.push(SelectorPart::Class(chars[start..j].iter().collect()));
                }
                i = j;
            }
            '#' => {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && is_ident_char(chars[j]) {
                    j += 1;
                }
                if j > start {
                    parts.push(SelectorPart::Id(chars[start..j].iter().collect()));
                }
                i = j;
            }
            c if c.is_ascii_alphabetic() || c == '-' || c == '_' => {
                let start = i;
                let mut j = i;
                while j < chars.len() && is_ident_char(chars[j]) {
                    j += 1;
                }
                parts.push(SelectorPart::Tag(chars[start..j].iter().collect()));
                i = j;
            }
            _ => {
                i += 1;
            }
        }
    }
    Selector { parts }
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

#[derive(Debug, Clone, Default)]
pub struct Styles {
    pub props: HashMap<String, String>,
}

impl Styles {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.props.get(key).map(|s| s.as_str())
    }

    pub fn has(&self, key: &str) -> bool {
        self.props.contains_key(key)
    }

    fn parse_block(&mut self, block: &str) {
        for decl in block.split(';') {
            let decl = decl.trim();
            if decl.is_empty() {
                continue;
            }
            if let Some((prop, value)) = decl.split_once(':') {
                let prop = prop.trim().to_lowercase();
                let value = value.trim();
                if !prop.is_empty() && !value.is_empty() {
                    self.props.insert(prop, value.to_string());
                }
            }
        }
    }
}

/// Normalize a length value to px. Handles px, em, rem, bare numbers, "0",
/// "auto"/"inherit" -> None. Percentages return None (resolved by callers).
pub fn px(value: &str, font_size_px: f64) -> Option<f64> {
    let v = value.trim();
    if v.is_empty()
        || v.eq_ignore_ascii_case("auto")
        || v.eq_ignore_ascii_case("inherit")
        || v.ends_with('%')
    {
        return None;
    }
    if v == "0" {
        return Some(0.0);
    }
    let stripped: String = v
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    let num: f64 = match stripped.parse() {
        Ok(n) => n,
        Err(_) => return None,
    };
    if v.ends_with("em") {
        Some(num * font_size_px)
    } else if v.ends_with("rem") {
        Some(num * 16.0)
    } else {
        Some(num)
    }
}

/// Default display for a tag when not otherwise specified.
pub fn default_display(tag: &str) -> &'static str {
    match tag {
        "span" | "a" | "b" | "i" | "em" | "strong" | "small" | "code" | "label" | "button"
        | "img" | "input" | "select" | "textarea" | "svg" | "time" | "mark" | "sub" | "sup" => {
            "inline"
        }
        "li" => "list-item",
        "script" | "style" | "head" | "meta" | "link" | "title" | "template" | "noscript" => "none",
        _ => "block",
    }
}

/// Build the effective styles for an element from inline, sheet rules, and
/// presentation attrs. Inline style and presentation attrs override the sheet.
pub fn resolve(el: &ElNode, sheet: &CssSheet) -> Styles {
    let mut s = Styles::default();

    // stylesheet rules first (lowest precedence)
    for (k, (_, v)) in sheet.sheet_props_for(el) {
        s.props.insert(k, v);
    }

    // inline style (overrides sheet)
    if let Some(style) = el.attr("style") {
        s.parse_block(style);
    }

    // presentational attributes (only when not already set)
    for (attr, key) in [
        ("width", "width"),
        ("height", "height"),
        ("color", "color"),
        ("bgcolor", "background-color"),
        ("align", "text-align"),
    ] {
        if !s.has(key)
            && let Some(v) = el.attr(attr)
        {
            s.props.insert(key.to_string(), v.to_string());
        }
    }
    s
}
