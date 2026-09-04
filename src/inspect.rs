/// Inspection: derive high-signal conclusions from the visual geometry model.
use crate::model::{ElementView, Rect, VisualModel};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Overlap {
    pub a_id: usize,
    pub a_sel: String,
    pub b_id: usize,
    pub b_sel: String,
    pub intersect: Rect,
    pub area: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overflow {
    pub id: usize,
    pub selector: String,
    pub kind: String, // "clipped" | "collapsed" | "offscreen" | "negative"
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContrastResult {
    pub id: usize,
    pub selector: String,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub ratio: Option<f64>,
    pub pass_aa: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectReport {
    pub viewport: Rect,
    pub total_elements: usize,
    pub visible_elements: usize,
    pub overlaps: Vec<Overlap>,
    pub overflows: Vec<Overflow>,
    pub contrast: Vec<ContrastResult>,
    pub quality: Quality,
    /// Notes from the JS engine (--js/`js:true`): script errors, unsupported
    /// async APIs, or the list of scripts that ran. Empty when JS is off.
    #[serde(default)]
    pub js_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Quality {
    pub score: f64, // 0..1
    pub label: String,
    pub issues: Vec<String>,
}

const MIN_OVERLAP_PX: f64 = 4.0;

/// Element id -> parent id map (built once per pass, O(n) lookups after).
fn parent_map(model: &VisualModel) -> std::collections::HashMap<usize, usize> {
    model.elements.iter().map(|e| (e.id, e.parent)).collect()
}

/// True if id `a` is an ancestor of id `b` by walking parent pointers.
fn is_ancestor(map: &std::collections::HashMap<usize, usize>, a: usize, b: usize) -> bool {
    if a == b {
        return false;
    }
    let mut cur = b;
    let mut guard = 0;
    while cur != 0 && guard <= map.len() {
        if cur == a {
            return true;
        }
        cur = map.get(&cur).copied().unwrap_or(0);
        guard += 1;
    }
    false
}

/// Compute overlaps between distinct visible elements.
/// O(n log n + k): candidates are sorted by x and only pairs whose x ranges
/// can intersect are examined, and ancestor checks use a prebuilt parent map
/// instead of rescanning the element list.
pub fn overlaps(model: &VisualModel) -> Vec<Overlap> {
    let parents = parent_map(model);
    let mut vis: Vec<&ElementView> = model
        .elements
        .iter()
        .filter(|e| e.visible && e.id != 0 && e.bbox.w > 0.0 && e.bbox.h > 0.0)
        .collect();
    vis.sort_by(|x, y| {
        x.bbox
            .x
            .partial_cmp(&y.bbox.x)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out = Vec::new();
    for i in 0..vis.len() {
        let a = vis[i];
        let a_right = a.bbox.x + a.bbox.w;
        for j in (i + 1)..vis.len() {
            let b = vis[j];
            if b.bbox.x >= a_right {
                break; // sorted: no later box can reach a
            }
            if !(b.bbox.y < a.bbox.y + a.bbox.h && a.bbox.y < b.bbox.y + b.bbox.h) {
                continue; // no y overlap
            }
            // skip ancestor/descendant pairs (legitimate containment via the DOM tree)
            if is_ancestor(&parents, a.id, b.id) || is_ancestor(&parents, b.id, a.id) {
                continue;
            }
            let area = a.bbox.overlap_area(&b.bbox);
            if area >= MIN_OVERLAP_PX {
                out.push(Overlap {
                    a_id: a.id,
                    a_sel: a.selector.clone(),
                    b_id: b.id,
                    b_sel: b.selector.clone(),
                    intersect: a.bbox.intersection(&b.bbox),
                    area,
                });
            }
        }
    }
    out.sort_by(|x, y| {
        y.area
            .partial_cmp(&x.area)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Detect clipped / collapsed / off-screen elements.
pub fn overflow(model: &VisualModel) -> Vec<Overflow> {
    let mut out = Vec::new();
    let vp = &model.viewport;
    for e in &model.elements {
        if e.id == 0 || !e.visible {
            continue;
        }
        let b = &e.bbox;
        if b.w <= 0.0 && b.h <= 0.0 {
            out.push(Overflow {
                id: e.id,
                selector: e.selector.clone(),
                kind: "collapsed".into(),
                detail: format!("zero-sized {} at ({} , {})", e.tag, b.x, b.y),
            });
            continue;
        }
        if b.w < 0.0 || b.h < 0.0 {
            out.push(Overflow {
                id: e.id,
                selector: e.selector.clone(),
                kind: "negative".into(),
                detail: format!("negative dimension {}", e.tag),
            });
            continue;
        }
        if b.x + b.w > vp.w + 1.0 {
            out.push(Overflow {
                id: e.id,
                selector: e.selector.clone(),
                kind: "clipped".into(),
                detail: format!("right edge {} exceeds viewport {}", round2(b.x + b.w), vp.w),
            });
        }
        if b.x < -1.0 {
            out.push(Overflow {
                id: e.id,
                selector: e.selector.clone(),
                kind: "offscreen".into(),
                detail: format!("left edge {} negative", round2(b.x)),
            });
        }
        if b.y + b.h > vp.h + 1.0 && vp.h > 0.0 {
            out.push(Overflow {
                id: e.id,
                selector: e.selector.clone(),
                kind: "clipped".into(),
                detail: format!(
                    "bottom edge {} exceeds viewport height {}",
                    round2(b.y + b.h),
                    vp.h
                ),
            });
        }
    }
    out
}

/// Resolved (r,g,b) each 0..255 with alpha 0..1, plus whether the value came
/// from an actual declaration (as opposed to the canvas defaults).
type Rgba = (f64, f64, f64, f64);

/// Estimate WCAG contrast. Text color is inherited down the tree (CSS color),
/// and the painted background is the nearest ancestor that declares a
/// background color, falling back to the white canvas. Only rows where at
/// least one color is actually declared are reported, so default black on
/// white does not become noise. Colors the engine cannot resolve are
/// reported as failing AA, never silently passed.
pub fn contrast(model: &VisualModel) -> Vec<ContrastResult> {
    let idx = id_index(model);
    let mut out = Vec::new();
    for e in &model.elements {
        if e.id == 0 || !e.visible || e.text.is_none() {
            continue;
        }
        let (fg_declared, fg) = chain_color(model, &idx, e, true);
        let (bg_declared, bg) = chain_color(model, &idx, e, false);
        if !fg_declared && !bg_declared {
            continue; // nothing declared anywhere along the chain: nothing to judge
        }
        let fg_s = fg.clone().unwrap_or_else(|| "#000000".to_string());
        let bg_s = bg.clone().unwrap_or_else(|| "#ffffff".to_string());
        let pf = parse_color(&fg_s);
        let pb = parse_color(&bg_s);
        let (pass, ratio, note) = match (pf, pb) {
            (Some(f), Some(b)) => {
                // composite alpha over the resolved background (fg over bg,
                // bg over the white canvas), then measure the pair
                let fg_c = composite_over(f, b);
                let bg_c = composite_over(b, (255.0, 255.0, 255.0, 1.0));
                let r = contrast_ratio(fg_c, bg_c);
                (
                    r >= 4.5,
                    Some(r),
                    format!("contrast ratio {:.2}:1 (AA text requires 4.5)", r),
                )
            }
            _ => {
                let unresolved = if pf.is_none() {
                    format!("color '{}'", fg_s)
                } else {
                    format!("background '{}'", bg_s)
                };
                (
                    false,
                    None,
                    format!("unresolved color, cannot certify AA: {}", unresolved),
                )
            }
        };
        out.push(ContrastResult {
            id: e.id,
            selector: e.selector.clone(),
            fg: Some(fg_s),
            bg: Some(bg_s),
            ratio,
            pass_aa: pass,
            note,
        });
    }
    out
}

/// Map element id -> index in the model's element vec (built once per pass).
fn id_index(model: &VisualModel) -> std::collections::HashMap<usize, usize> {
    model
        .elements
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id, i))
        .collect()
}

/// Walk from `el` up the parent chain looking for a declared color.
/// `for_fg=true` treats the element's own color first (CSS inheritance),
/// `for_fg=false` treats background-color as not inherited: the element's own
/// declaration, else the first ancestor that painted a background.
/// Returns (declared_anywhere, resolved_value).
fn chain_color(
    model: &VisualModel,
    idx: &std::collections::HashMap<usize, usize>,
    e: &ElementView,
    for_fg: bool,
) -> (bool, Option<String>) {
    let mut cur = e;
    let mut guard = 0;
    while guard <= model.elements.len() {
        let own = if for_fg {
            cur.fg.as_ref()
        } else {
            cur.bg.as_ref()
        };
        if let Some(c) = own {
            return (true, Some(c.clone()));
        }
        if cur.parent == 0 || cur.parent == cur.id {
            return (false, None);
        }
        match idx.get(&cur.parent).and_then(|&i| model.elements.get(i)) {
            Some(p) => cur = p,
            None => return (false, None),
        }
        guard += 1;
    }
    (false, None)
}

/// Alpha-composite `front` over `back` (both 0..255 rgb, alpha 0..1).
fn composite_over(front: Rgba, back: Rgba) -> (f64, f64, f64, f64) {
    let a = front.3 + back.3 * (1.0 - front.3);
    if a <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let comp = |f: f64, b: f64| (f * front.3 + b * back.3 * (1.0 - front.3)) / a;
    (
        comp(front.0, back.0),
        comp(front.1, back.1),
        comp(front.2, back.2),
        a,
    )
}

fn clamp255(v: f64) -> f64 {
    v.clamp(0.0, 255.0)
}

/// Parse a CSS color value. Supports #rgb, #rgba, #rrggbb, #rrggbbaa,
/// rgb()/rgba() with commas or spaces plus optional slash alpha, percentages,
/// hsl()/hsla(), and a common named-color table. Anything else (var(),
/// currentColor, gradients, system colors) returns None so the caller can
/// report it as unresolved rather than guessing.
fn parse_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if let Some(c) = parse_color_primary(s) {
        return Some(c);
    }
    // shorthand fallback: "background: url(x) no-repeat #fff center" carries
    // the color as one token; pick the last token that parses as a color
    let mut last: Option<Rgba> = None;
    for tok in s.split_whitespace() {
        if let Some(c) = parse_color_primary(tok) {
            last = Some(c);
        }
    }
    last
}

fn parse_color_primary(s: &str) -> Option<Rgba> {
    if let Some(hex) = s.strip_prefix('#') {
        let expanded: String = match hex.len() {
            3 | 4 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
            6 | 8 => hex.to_string(),
            _ => return None,
        };
        if expanded.len() == 6 || expanded.len() == 8 {
            let alpha = if expanded.len() == 8 {
                u8::from_str_radix(&expanded[6..8], 16).ok()? as f64 / 255.0
            } else {
                1.0
            };
            let r = u8::from_str_radix(&expanded[0..2], 16).ok()? as f64;
            let g = u8::from_str_radix(&expanded[2..4], 16).ok()? as f64;
            let b = u8::from_str_radix(&expanded[4..6], 16).ok()? as f64;
            return Some((r, g, b, alpha));
        }
        return None;
    }
    if s.eq_ignore_ascii_case("transparent") {
        return Some((0.0, 0.0, 0.0, 0.0));
    }
    // rgb(...) / rgba(...) / hsl(...) / hsla(...) with commas or spaces.
    for (name, is_hsl) in [("rgb", false), ("hsl", true)] {
        let lower = s.to_ascii_lowercase();
        if !lower.starts_with(name) {
            continue;
        }
        let open = s.find('(')?;
        let close = s.rfind(')')?;
        if open > close {
            return None;
        }
        let body = s[open + 1..close].trim();
        // split on commas when present, else whitespace, else slash groups
        let mut parts: Vec<&str> = Vec::new();
        if body.contains(',') {
            parts = body.split(',').map(|p| p.trim()).collect();
        } else {
            for tok in body.split_whitespace() {
                if tok == "/" {
                    continue;
                }
                parts.push(tok.trim());
            }
        }
        let mut it = parts.into_iter();
        let mut rgb: (f64, f64, f64);
        if is_hsl {
            let h_raw = parse_angle(it.next()?)?;
            let s_pct = parse_pct_or_num(it.next()?)?;
            let l_pct = parse_pct_or_num(it.next()?)?;
            rgb = hsl_to_rgb(h_raw, s_pct, l_pct);
        } else {
            let r = parse_channel(it.next()?)?;
            let g = parse_channel(it.next()?)?;
            let b = parse_channel(it.next()?)?;
            rgb = (r, g, b);
        }
        let alpha = match it.next() {
            Some(a) => parse_alpha(a)?,
            None => 1.0,
        };
        if !is_hsl {
            rgb.0 = clamp255(rgb.0);
            rgb.1 = clamp255(rgb.1);
            rgb.2 = clamp255(rgb.2);
        }
        return Some((rgb.0, rgb.1, rgb.2, alpha));
    }
    // common named colors (else the caller reports the row as unresolved)
    let named: &[(&str, (f64, f64, f64))] = &[
        ("black", (0.0, 0.0, 0.0)),
        ("white", (255.0, 255.0, 255.0)),
        ("red", (255.0, 0.0, 0.0)),
        ("lime", (0.0, 255.0, 0.0)),
        ("blue", (0.0, 0.0, 255.0)),
        ("green", (0.0, 128.0, 0.0)),
        ("gray", (128.0, 128.0, 128.0)),
        ("grey", (128.0, 128.0, 128.0)),
        ("silver", (192.0, 192.0, 192.0)),
        ("yellow", (255.0, 255.0, 0.0)),
        ("cyan", (0.0, 255.0, 255.0)),
        ("aqua", (0.0, 255.0, 255.0)),
        ("magenta", (255.0, 0.0, 255.0)),
        ("fuchsia", (255.0, 0.0, 255.0)),
        ("orange", (255.0, 165.0, 0.0)),
        ("purple", (128.0, 0.0, 128.0)),
        ("navy", (0.0, 0.0, 128.0)),
        ("teal", (0.0, 128.0, 128.0)),
        ("olive", (128.0, 128.0, 0.0)),
        ("maroon", (128.0, 0.0, 0.0)),
        ("aquamarine", (127.0, 255.0, 212.0)),
        ("brown", (165.0, 42.0, 42.0)),
        ("coral", (255.0, 127.0, 80.0)),
        ("darkblue", (0.0, 0.0, 139.0)),
        ("darkgray", (169.0, 169.0, 169.0)),
        ("darkgrey", (169.0, 169.0, 169.0)),
        ("darkgreen", (0.0, 100.0, 0.0)),
        ("darkorange", (255.0, 140.0, 0.0)),
        ("darkred", (139.0, 0.0, 0.0)),
        ("gold", (255.0, 215.0, 0.0)),
        ("indigo", (75.0, 0.0, 130.0)),
        ("ivory", (255.0, 255.0, 240.0)),
        ("khaki", (240.0, 230.0, 140.0)),
        ("lightgray", (211.0, 211.0, 211.0)),
        ("lightgrey", (211.0, 211.0, 211.0)),
        ("lightblue", (173.0, 216.0, 230.0)),
        ("lightgreen", (144.0, 238.0, 144.0)),
        ("lightpink", (255.0, 182.0, 193.0)),
        ("pink", (255.0, 192.0, 203.0)),
        ("rebeccapurple", (102.0, 51.0, 153.0)),
        ("salmon", (250.0, 128.0, 114.0)),
        ("skyblue", (135.0, 206.0, 235.0)),
        ("tan", (210.0, 180.0, 140.0)),
        ("violet", (238.0, 130.0, 238.0)),
        ("darkviolet", (148.0, 0.0, 211.0)),
        ("crimson", (220.0, 20.0, 60.0)),
        ("dodgerblue", (30.0, 144.0, 255.0)),
        ("forestgreen", (34.0, 139.0, 34.0)),
        ("goldenrod", (218.0, 165.0, 32.0)),
        ("hotpink", (255.0, 105.0, 180.0)),
        ("limegreen", (50.0, 205.0, 50.0)),
        ("mediumblue", (0.0, 0.0, 205.0)),
        ("midnightblue", (25.0, 25.0, 112.0)),
        ("sandybrown", (244.0, 164.0, 96.0)),
        ("seagreen", (46.0, 139.0, 87.0)),
        ("tomato", (255.0, 99.0, 71.0)),
        ("turquoise", (64.0, 224.0, 208.0)),
        ("wheat", (245.0, 222.0, 179.0)),
    ];
    for (name, rgb) in named {
        if s.eq_ignore_ascii_case(name) {
            return Some((rgb.0, rgb.1, rgb.2, 1.0));
        }
    }
    None
}

/// A single rgb channel: plain number 0..255 or percentage 0..100%.
fn parse_channel(v: &str) -> Option<f64> {
    if v.ends_with('%') {
        Some(parse_pct_or_num(v)? * 2.55)
    } else {
        parse_pct_or_num(v)
    }
}

fn parse_pct_or_num(v: &str) -> Option<f64> {
    let t = v.trim();
    let t = t.strip_suffix('%').unwrap_or(t);
    t.parse::<f64>().ok()
}

fn parse_alpha(v: &str) -> Option<f64> {
    let a = parse_pct_or_num(v)?;
    Some(a.clamp(0.0, 1.0))
}

/// Angle in degrees (also accepts deg/rad/grad/turn, bare number = deg).
fn parse_angle(v: &str) -> Option<f64> {
    let t = v.trim().to_ascii_lowercase();
    for (suffix, mult) in [
        ("deg", 1.0),
        ("grad", 0.9),
        ("rad", 180.0 / std::f64::consts::PI),
        ("turn", 360.0),
    ] {
        if let Some(num) = t.strip_suffix(suffix) {
            return num.trim().parse::<f64>().ok().map(|n| n * mult);
        }
    }
    t.parse::<f64>().ok()
}

/// Standard hsl(h, s%, l%) to rgb, channels 0..255.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let h = ((h % 360.0) + 360.0) % 360.0 / 360.0;
    let s = (s / 100.0).clamp(0.0, 1.0);
    let l = (l / 100.0).clamp(0.0, 1.0);
    if s == 0.0 {
        let g = l * 255.0;
        return (g, g, g);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |t: f64| -> f64 {
        let t = ((t % 1.0) + 1.0) % 1.0;
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        v * 255.0
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

fn srgb_lin(c: f64) -> f64 {
    let c = c / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Rgba) -> f64 {
    0.2126 * srgb_lin(c.0) + 0.7152 * srgb_lin(c.1) + 0.0722 * srgb_lin(c.2)
}

fn contrast_ratio(a: Rgba, b: Rgba) -> f64 {
    let la = luminance(a);
    let lb = luminance(b);
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Combined quality report (0..1 score).
pub fn quality(model: &VisualModel) -> Quality {
    let ov = overlaps(model);
    let of = overflow(model);
    let issues = ov.len() + of.len();
    let score = if issues == 0 {
        1.0
    } else {
        (1.0 - (issues as f64 / (model.element_count.max(1) as f64) * 2.0)).clamp(0.0, 1.0)
    };
    let mut strs = Vec::new();
    for o in &ov {
        strs.push(format!("overlap: {} ⇄ {}", o.a_sel, o.b_sel));
    }
    for o in &of {
        strs.push(format!("{}: {}", o.kind, o.selector));
    }
    strs.truncate(12);
    let label = if score >= 0.9 {
        "good".into()
    } else if score >= 0.6 {
        "acceptable".into()
    } else {
        "broken".into()
    };
    Quality {
        score,
        label,
        issues: strs,
    }
}

/// Build the full inspection report.
pub fn inspect(model: &VisualModel) -> InspectReport {
    let visible_elements = model
        .elements
        .iter()
        .filter(|e| e.visible && e.id != 0)
        .count();
    InspectReport {
        viewport: model.viewport,
        total_elements: model.element_count,
        visible_elements,
        overlaps: overlaps(model),
        overflows: overflow(model),
        contrast: contrast(model),
        quality: quality(model),
        js_notes: Vec::new(),
    }
}

pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
