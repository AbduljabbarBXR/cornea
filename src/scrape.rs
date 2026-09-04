//! Web scraping surface: structured extraction from an HTML document.
//!
//! Honest scope: static content only. Anything that needs clicks, scrolls,
//! login or client side rendering is outside this function's reach (pair it
//! with a headless capture when a browser exists on the machine).

use crate::dom::{self, Node};
use serde_json::json;

const SKIP_TAGS: &[&str] = &["script", "style", "noscript", "template"];

/// Extract the interesting parts of a document as lean JSON: metadata,
/// headings, links, images, tables, and collapsed text.
pub fn scrape(html: &str) -> serde_json::Value {
    let parsed = dom::parse(html);
    let root = match &parsed.root {
        Node::Elem(e) => e,
        _ => return json!({}),
    };

    let mut title: Option<String> = None;
    let mut meta_desc: Option<String> = None;
    let mut lang: Option<String> = None;
    let mut h1: Vec<String> = Vec::new();
    let mut links: Vec<serde_json::Value> = Vec::new();
    let mut images: Vec<serde_json::Value> = Vec::new();
    let mut tables = 0usize;
    let mut text = String::new();
    let mut headings = [0usize; 6];

    walk(
        root,
        &mut title,
        &mut meta_desc,
        &mut lang,
        &mut h1,
        &mut links,
        &mut images,
        &mut tables,
        &mut text,
        &mut headings,
    );

    json!({
        "title": title,
        "meta_description": meta_desc,
        "lang": lang,
        "h1": h1.iter().take(10).cloned().collect::<Vec<_>>(),
        "headings": {
            "h1": headings[0], "h2": headings[1], "h3": headings[2],
            "h4": headings[3], "h5": headings[4], "h6": headings[5]
        },
        "links": links.iter().take(200).cloned().collect::<Vec<_>>(),
        "images": images.iter().take(100).cloned().collect::<Vec<_>>(),
        "tables": tables,
        "text": text.chars().take(20_000).collect::<String>(),
    })
}

#[allow(clippy::too_many_arguments)]
fn walk(
    el: &dom::ElNode,
    title: &mut Option<String>,
    meta_desc: &mut Option<String>,
    lang: &mut Option<String>,
    h1: &mut Vec<String>,
    links: &mut Vec<serde_json::Value>,
    images: &mut Vec<serde_json::Value>,
    tables: &mut usize,
    text: &mut String,
    headings: &mut [usize; 6],
) {
    match el.tag.as_str() {
        "html" => {
            if lang.is_none() {
                *lang = el.attr("lang").map(|v| v.to_string());
            }
        }
        "title" => {
            if title.is_none() {
                *title = Some(node_text(el));
            }
        }
        "meta" => {
            let name = el
                .attr("name")
                .or_else(|| el.attr("property"))
                .map(|v| v.to_ascii_lowercase());
            let is_desc = matches!(
                name.as_deref(),
                Some("description") | Some("og:description") | Some("twitter:description")
            );
            if is_desc && meta_desc.is_none() {
                *meta_desc = el.attr("content").map(|v| v.chars().take(300).collect());
            }
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = el.tag.as_bytes()[1] - b'0';
            headings[(level - 1) as usize] += 1;
            if el.tag == "h1" && h1.len() < 10 {
                h1.push(cut(&node_text(el), 200));
            }
        }
        "a" => {
            if let Some(href) = el.attr("href") {
                let t = cut(&node_text(el), 60);
                if links.len() < 200 {
                    links.push(json!({ "text": t, "href": cut(href, 200) }));
                }
            }
        }
        "img" => {
            if images.len() < 100 {
                images.push(json!({
                    "src": el.attr("src").map(|s| cut(s, 200)),
                    "alt": el.attr("alt").map(|a| cut(a, 120)),
                }));
            }
        }
        "table" => *tables += 1,
        _ => {}
    }

    if SKIP_TAGS.contains(&el.tag.as_str()) {
        return;
    }
    for child in &el.children {
        match child {
            Node::Text(s) => {
                if text.len() < 20_000 {
                    text.push_str(s);
                    text.push(' ');
                }
            }
            Node::Elem(c) => walk(
                c, title, meta_desc, lang, h1, links, images, tables, text, headings,
            ),
        }
    }
}

fn node_text(el: &dom::ElNode) -> String {
    let mut out = String::new();
    collect(el, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect(el: &dom::ElNode, out: &mut String) {
    for child in &el.children {
        match child {
            Node::Text(s) => out.push_str(s),
            Node::Elem(c) => collect(c, out),
        }
    }
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
