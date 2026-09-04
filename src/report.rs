//! Page view reporting: deterministic structure summary of a rendered page.
//!
//! The engine cannot judge aesthetics and never pretends to. What it CAN do
//! deterministically is describe how a page is put together: which semantic
//! sections exist, what the hero block looks like (box, fold position,
//! heading, CTAs, contrast), and simple page stats. The agent that reads
//! this JSON is the one that decides whether the hero needs adjustment.

use crate::analyze_vh;
use crate::dom::{self, Node};
use crate::inspect::InspectReport;
use crate::model::{Rect, VisualModel};
use serde_json::json;
use std::collections::HashMap;

const SECTION_TAGS: &[&str] = &[
    "header", "nav", "main", "section", "article", "aside", "footer", "form",
];
const SKIP_TEXT_TAGS: &[&str] = &["script", "style", "noscript", "template"];

/// Build the page view JSON for a document. `width` and `height` mirror the
/// inspection viewport (height 0 = unbounded page).
pub fn page_view(html: &str, width: f64, height: f64) -> serde_json::Value {
    let (model, irep) = analyze_vh(html, width, height);
    build_view(html, &model, &irep, width, height)
}

fn build_view(
    html: &str,
    model: &VisualModel,
    irep: &InspectReport,
    width: f64,
    height: f64,
) -> serde_json::Value {
    let parsed = dom::parse(html);
    let boxes: HashMap<usize, (Rect, bool)> = model
        .elements
        .iter()
        .map(|e| (e.id, (e.bbox, e.visible)))
        .collect();

    // one pass to collect semantic sections and full text
    let mut sections: Vec<serde_json::Value> = Vec::new();
    let mut hero: Option<serde_json::Value> = None;
    let mut hero_ids: Vec<usize> = Vec::new();
    let mut stats = DocStats::default();

    if let Node::Elem(doc) = &parsed.root {
        walk_semantics(
            doc,
            &boxes,
            &mut stats,
            &mut sections,
            &mut hero,
            &mut hero_ids,
        );
    }

    let contrast_failed = irep
        .contrast
        .iter()
        .filter(|c| !c.pass_aa)
        .filter(|c| hero_ids.is_empty() || hero_ids.contains(&c.id))
        .count();
    let hero_fold = if let Some(h) = &hero {
        let b = h.get("box").and_then(|b| b.as_object());
        let top = b
            .and_then(|b| b.get("y"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let bottom = b
            .and_then(|b| b.get("h"))
            .and_then(|v| v.as_f64())
            .map(|h| top + h)
            .unwrap_or(top);
        if height <= 0.0 {
            Some("unbounded".to_string())
        } else if bottom <= height {
            Some("above".to_string())
        } else if top < height {
            Some("partial".to_string())
        } else {
            Some("below".to_string())
        }
    } else {
        None
    };
    if let Some(h) = hero.as_mut() {
        if let Some(f) = hero_fold {
            h["fold_fit"] = json!(f);
        }
        h["contrast_failed_in_hero"] = json!(contrast_failed);
        if let Some(b) = h.get("box").and_then(|b| b.as_object()) {
            let x = b.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let w = b.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if width > 0.0 && w > 0.0 {
                let center = x + w / 2.0;
                h["full_bleed"] = json!(w >= width * 0.95);
                h["centered"] =
                    json!(w >= width * 0.5 && (center - width / 2.0).abs() <= width * 0.05);
            }
        }
    }

    json!({
        "sections": sections,
        "hero": hero,
        "stats": {
            "total_elements": irep.total_elements,
            "visible_elements": irep.visible_elements,
            "text_chars": stats.text_chars,
            "images": stats.images,
            "links": stats.links,
            "buttons": stats.buttons,
            "headings": {
                "h1": stats.h1, "h2": stats.h2, "h3": stats.h3,
                "h4": stats.h4, "h5": stats.h5, "h6": stats.h6
            },
            "contrast_passed": irep.contrast.iter().filter(|c| c.pass_aa).count(),
            "contrast_failed": irep.contrast.iter().filter(|c| !c.pass_aa).count(),
            "quality_score": irep.quality.score,
            "warnings": irep.warnings
        },
        "viewport": { "width": width, "height": height }
    })
}

#[derive(Default)]
struct DocStats {
    text_chars: usize,
    images: usize,
    links: usize,
    buttons: usize,
    h1: usize,
    h2: usize,
    h3: usize,
    h4: usize,
    h5: usize,
    h6: usize,
}

/// Recursive walk collecting section entries, hero detection and page stats.
#[allow(clippy::too_many_arguments)]
fn walk_semantics(
    el: &dom::ElNode,
    boxes: &HashMap<usize, (Rect, bool)>,
    stats: &mut DocStats,
    sections: &mut Vec<serde_json::Value>,
    hero: &mut Option<serde_json::Value>,
    hero_ids: &mut Vec<usize>,
) {
    if SKIP_TEXT_TAGS.contains(&el.tag.as_str()) {
        return;
    }

    let text = collapse_text(el);
    if !text.is_empty() {
        stats.text_chars += text.chars().count().min(400);
    }
    match el.tag.as_str() {
        "img" => stats.images += 1,
        "a" if has_attr(el, "href") => stats.links += 1,
        "button" | "input" => stats.buttons += 1,
        "h1" => stats.h1 += 1,
        "h2" => stats.h2 += 1,
        "h3" => stats.h3 += 1,
        "h4" => stats.h4 += 1,
        "h5" => stats.h5 += 1,
        "h6" => stats.h6 += 1,
        _ => {}
    }

    let is_semantic = SECTION_TAGS.contains(&el.tag.as_str());
    let is_hero_named = el
        .attr("class")
        .map(|c| c.to_ascii_lowercase().contains("hero"))
        .unwrap_or(false);

    if is_semantic {
        if let Some((bbox, visible)) = boxes.get(&el.id).copied() {
            if visible && bbox.w > 0.0 && bbox.h > 0.0 {
                sections.push(json!({
                    "kind": el.tag,
                    "selector": selector_of(el),
                    "box": box_json(bbox),
                    "text": text.chars().take(140).collect::<String>(),
                    "children": el.children.len(),
                }));
            }
        }
    }

    // hero: an explicit class wins, else the first large top block
    if hero.is_none() {
        let geometric = !el.tag.is_empty()
            && matches!(
                el.tag.as_str(),
                "section" | "div" | "article" | "main" | "header"
            )
            && boxes
                .get(&el.id)
                .map(|(b, v)| *v && b.w >= 200.0 && b.h >= 120.0 && b.y >= 0.0 && b.y < 800.0)
                .unwrap_or(false);
        if is_hero_named && boxes.get(&el.id).map(|(_, v)| *v).unwrap_or(false) {
            *hero = Some(hero_entry(el, bbox_of(boxes, el.id), text.clone()));
            collect_ids(el, hero_ids);
        } else if geometric {
            if let Some((bbox, _)) = boxes.get(&el.id) {
                *hero = Some(hero_entry(el, *bbox, text.clone()));
                collect_ids(el, hero_ids);
            }
        }
    }

    for child in &el.children {
        if let Node::Elem(c) = child {
            walk_semantics(c, boxes, stats, sections, hero, hero_ids);
        }
    }
}

fn hero_entry(el: &dom::ElNode, bbox: Rect, text: String) -> serde_json::Value {
    let heading = first_heading(el);
    let ctas = cta_samples(el);
    json!({
        "tag": el.tag,
        "selector": selector_of(el),
        "box": box_json(bbox),
        "text": text.chars().take(200).collect::<String>(),
        "heading": heading,
        "ctas": ctas,
    })
}

fn box_json(b: Rect) -> serde_json::Value {
    json!({ "x": round2(b.x), "y": round2(b.y), "w": round2(b.w), "h": round2(b.h) })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn bbox_of(boxes: &HashMap<usize, (Rect, bool)>, id: usize) -> Rect {
    boxes
        .get(&id)
        .map(|(b, _)| *b)
        .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
}

fn has_attr(el: &dom::ElNode, name: &str) -> bool {
    el.attrs.iter().any(|(k, _)| k == name)
}

fn selector_of(el: &dom::ElNode) -> String {
    if let Some(id) = el.attr("id") {
        return format!("#{}", id);
    }
    match el.attr("class").and_then(|c| c.split_whitespace().next()) {
        Some(first) => format!("{}.{}", el.tag, first),
        None => el.tag.clone(),
    }
}

/// Collapse whitespace in an element's descendant text.
fn collapse_text(el: &dom::ElNode) -> String {
    let mut out = String::new();
    collect_text(el, &mut out);
    let mut parts: Vec<&str> = out.split_whitespace().collect();
    parts.truncate(400);
    parts.join(" ")
}

fn collect_text(el: &dom::ElNode, out: &mut String) {
    for child in &el.children {
        match child {
            Node::Text(s) => out.push_str(s),
            Node::Elem(c) => {
                if !SKIP_TEXT_TAGS.contains(&c.tag.as_str()) {
                    collect_text(c, out);
                }
            }
        }
    }
}

fn first_heading(el: &dom::ElNode) -> Option<String> {
    let mut hit: Option<String> = None;
    collect_heading(el, &mut hit);
    hit
}

fn collect_heading(el: &dom::ElNode, hit: &mut Option<String>) {
    for child in &el.children {
        let Node::Elem(c) = child else { continue };
        if hit.is_none() && matches!(c.tag.as_str(), "h1" | "h2" | "h3") {
            let mut t = String::new();
            collect_text(c, &mut t);
            *hit = Some(collapse_str(&t).chars().take(120).collect());
            return;
        }
        collect_heading(c, hit);
        if hit.is_some() {
            return;
        }
    }
}

fn collapse_str(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// First five interactive descendants with their text and href.
fn cta_samples(el: &dom::ElNode) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    collect_ctas(el, &mut out);
    out.truncate(5);
    out
}

fn collect_ctas(el: &dom::ElNode, out: &mut Vec<serde_json::Value>) {
    for child in &el.children {
        let Node::Elem(c) = child else { continue };
        let is_button = c.tag == "button"
            || (c.tag == "input"
                && c.attr("type")
                    .map(|t| t == "submit" || t == "button")
                    .unwrap_or(false))
            || (c.tag == "a" && has_attr(c, "href"));
        if is_button {
            let mut t = String::new();
            collect_text(c, &mut t);
            out.push(json!({
                "tag": c.tag,
                "text": collapse_str(&t).chars().take(40).collect::<String>(),
                "href": c.attr("href").map(|h| h.chars().take(80).collect::<String>()),
            }));
        }
        collect_ctas(c, out);
    }
}

fn collect_ids(el: &dom::ElNode, ids: &mut Vec<usize>) {
    ids.push(el.id);
    for child in &el.children {
        if let Node::Elem(c) = child {
            collect_ids(c, ids);
        }
    }
}
