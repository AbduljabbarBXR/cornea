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

/// Compute overlaps between distinct visible elements.
pub fn overlaps(model: &VisualModel) -> Vec<Overlap> {
    let vis: Vec<&ElementView> = model
        .elements
        .iter()
        .filter(|e| e.visible && e.id != 0 && e.bbox.w > 0.0 && e.bbox.h > 0.0)
        .collect();
    let mut out = Vec::new();
    for i in 0..vis.len() {
        for j in (i + 1)..vis.len() {
            let a = vis[i];
            let b = vis[j];
            // skip ancestor/descendant pairs (legitimate containment via the DOM tree)
            if is_ancestor(model, a.id, b.id) || is_ancestor(model, b.id, a.id) {
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

/// True if id `a` is an ancestor of id `b` by walking parent pointers.
fn is_ancestor(model: &VisualModel, a: usize, b: usize) -> bool {
    if a == b {
        return false;
    }
    let mut cur = b;
    let mut guard = 0;
    while cur != 0 && guard < model.elements.len() + 1 {
        if cur == a {
            return true;
        }
        cur = model
            .elements
            .iter()
            .find(|e| e.id == cur)
            .map(|e| e.parent)
            .unwrap_or(0);
        guard += 1;
    }
    false
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

/// Estimate WCAG contrast from hex colors.
/// Only reports rows when at least one color is actually declared in the page,
/// to avoid noise from default black-on-white.
pub fn contrast(model: &VisualModel) -> Vec<ContrastResult> {
    let mut out = Vec::new();
    for e in &model.elements {
        if e.id == 0 || !e.visible || e.text.is_none() {
            continue;
        }
        // skip when neither fg nor bg is declared anywhere (nothing to judge)
        if e.fg.is_none() && e.bg.is_none() {
            continue;
        }
        let fg = e.fg.clone().unwrap_or_else(|| "#000000".to_string());
        let bg = e.bg.clone().unwrap_or_else(|| "#ffffff".to_string());
        let ratio = parse_color(&fg)
            .and_then(|f| parse_color(&bg).map(|b| (f, b)))
            .map(|(f, b)| contrast_ratio(f, b));
        let (pass, note) = match ratio {
            Some(r) => (
                r >= 4.5,
                format!("contrast ratio {:.2}:1 (AA text requires 4.5)", r),
            ),
            None => (true, "unresolved color values".into()),
        };
        out.push(ContrastResult {
            id: e.id,
            selector: e.selector.clone(),
            fg: Some(fg),
            bg: Some(bg),
            ratio,
            pass_aa: pass,
            note,
        });
    }
    out
}

fn parse_color(s: &str) -> Option<(f64, f64, f64)> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            return Some((r as f64, g as f64, b as f64));
        }
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some((r as f64, g as f64, b as f64));
        }
    }
    // common named colors (else ratio is reported as unresolved)
    let named: [(&str, (f64, f64, f64)); 18] = [
        ("black", (0.0, 0.0, 0.0)),
        ("white", (255.0, 255.0, 255.0)),
        ("red", (255.0, 0.0, 0.0)),
        ("green", (0.0, 128.0, 0.0)),
        ("blue", (0.0, 0.0, 255.0)),
        ("gray", (128.0, 128.0, 128.0)),
        ("grey", (128.0, 128.0, 128.0)),
        ("yellow", (255.0, 255.0, 0.0)),
        ("cyan", (0.0, 255.0, 255.0)),
        ("magenta", (255.0, 0.0, 255.0)),
        ("orange", (255.0, 165.0, 0.0)),
        ("purple", (128.0, 0.0, 128.0)),
        ("brown", (165.0, 42.0, 42.0)),
        ("lightgray", (211.0, 211.0, 211.0)),
        ("lightgrey", (211.0, 211.0, 211.0)),
        ("darkgray", (169.0, 169.0, 169.0)),
        ("darkgrey", (169.0, 169.0, 169.0)),
        ("transparent", (255.0, 255.0, 255.0)),
    ];
    for (name, rgb) in named {
        if s.eq_ignore_ascii_case(name) {
            return Some(rgb);
        }
    }
    None
}

fn srgb_lin(c: f64) -> f64 {
    let c = c / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: (f64, f64, f64)) -> f64 {
    0.2126 * srgb_lin(c.0) + 0.7152 * srgb_lin(c.1) + 0.0722 * srgb_lin(c.2)
}

fn contrast_ratio(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
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
