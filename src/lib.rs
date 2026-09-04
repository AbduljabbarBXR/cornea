//! Cornea — deterministic visual inspection for AI agents.
//!
//! Core library: parse HTML (+ minimal CSS), compute a deterministic visual
//! geometry model, and derive inspection conclusions (overlap, overflow,
//! contrast, quality).

pub mod css;
pub mod dom;
pub mod inspect;
pub mod js;
pub mod layout;
pub mod model;
pub mod rest;

use model::VisualModel;

/// Full pipeline: HTML + viewport width -> visual model.
pub fn build_model(html: &str, viewport_w: f64) -> VisualModel {
    let parsed = dom::parse(html);
    let sheet = css::CssSheet::from_html(html);
    let engine = layout::LayoutEngine::new(&parsed.root, &sheet);
    engine.run(viewport_w)
}

/// Full pipeline producing both the visual model and the inspection report.
pub fn analyze(html: &str, viewport_w: f64) -> (VisualModel, inspect::InspectReport) {
    let model = build_model(html, viewport_w);
    let report = inspect::inspect(&model);
    (model, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
    <!DOCTYPE html><html><head><style>
      .low { color: #cccccc; }
    </style></head><body>
      <header>Title</header>
      <main>
        <div style="position:absolute;left:20px;top:20px;width:200px;height:100px;background:red">a</div>
        <div style="position:absolute;left:150px;top:40px;width:200px;height:100px;background:blue">b</div>
        <div style="width:600px;height:50px;background:yellow">wide</div>
        <p class="low">hard to read</p>
      </main>
    </body></html>"#;

    #[test]
    fn determinism_byte_identical_output() {
        let (_m1, r1) = analyze(SAMPLE, 360.0);
        let (_m2, r2) = analyze(SAMPLE, 360.0);
        assert_eq!(
            serde_json::to_string(&r1).unwrap(),
            serde_json::to_string(&r2).unwrap(),
            "same input must produce byte-identical inspection output"
        );
    }

    #[test]
    fn detects_absolute_overlap() {
        let (_m, r) = analyze(SAMPLE, 360.0);
        assert!(
            !r.overlaps.is_empty(),
            "expected the two absolutely-positioned divs to overlap"
        );
        // left: x20..220 y20..120, right: x150..350 y40..140 -> 70x80 = 5600
        assert!(
            r.overlaps.iter().any(|o| (o.area - 5600.0).abs() < 1.0),
            "expected the abs-overlap pair (area ~5600), got {:?}",
            r.overlaps.iter().map(|o| o.area).collect::<Vec<_>>()
        );
    }

    #[test]
    fn detects_right_edge_overflow() {
        let (_m, r) = analyze(SAMPLE, 360.0);
        assert!(
            r.overflows
                .iter()
                .any(|o| { o.kind == "clipped" && o.detail.contains("600") }),
            "expected width:600px element to overflow the 360px viewport"
        );
    }

    #[test]
    fn contrast_flags_low_contrast() {
        let (_m, r) = analyze(SAMPLE, 360.0);
        assert!(
            r.contrast.iter().any(|c| !c.pass_aa),
            "expected at least one WCAG AA contrast failure"
        );
    }

    #[test]
    fn explicit_width_wins_and_is_not_clamped() {
        let (m, _r) = analyze(SAMPLE, 360.0);
        let wide = m
            .elements
            .iter()
            .find(|e| e.text.as_deref() == Some("wide"))
            .expect("wide element should exist");
        assert!(wide.bbox.w >= 600.0, "explicit width must not be clamped");
    }

    #[test]
    fn inline_siblings_flow_horizontally_not_stacked() {
        let (m, _r) = analyze(
            "<html><body><span>Hello</span><span>World</span></body></html>",
            360.0,
        );
        let spans: Vec<_> = m.elements.iter().filter(|e| e.tag == "span").collect();
        assert_eq!(spans.len(), 2, "expected two inline spans");
        let (a, b) = (spans[0], spans[1]);
        assert!(
            b.bbox.x > a.bbox.x,
            "second inline should sit to the right of the first (horizontal flow), got a.x={} b.x={}",
            a.bbox.x,
            b.bbox.x
        );
        assert!(
            (a.bbox.y - b.bbox.y).abs() < 1.0,
            "inline siblings should share a line (same y)"
        );
    }

    #[test]
    fn empty_html_produces_no_errors() {
        let (m, r) = analyze("", 360.0);
        assert!(!m.elements.is_empty());
        assert!(serde_json::to_string(&r).is_ok());
    }

    #[test]
    fn deeply_nested_html_does_not_crash() {
        let depth = 100;
        let mut html = String::from("<body>");
        for _ in 0..depth {
            html.push_str("<div>");
        }
        html.push('x');
        for _ in 0..depth {
            html.push_str("</div>");
        }
        html.push_str("</body>");
        let (_m, r) = analyze(&html, 360.0);
        assert!(r.total_elements >= depth, "all nested divs counted");
    }

    #[test]
    fn flex_row_distributes_siblings_evenly() {
        let (m, _r) = analyze(
            "<html><body><div style=\"display:flex;width:200\"><div>A</div><div>B</div></div></body></html>",
            360.0,
        );
        let kids: Vec<_> = m
            .elements
            .iter()
            .filter(|e| e.text.as_deref() == Some("A") || e.text.as_deref() == Some("B"))
            .collect();
        assert_eq!(kids.len(), 2);
        assert!(
            kids[1].bbox.x > kids[0].bbox.x,
            "flex row children should sit side by side"
        );
    }

    #[test]
    fn display_none_is_invisible_and_zero_sized() {
        let (m, _r) = analyze(
            "<html><body><div style=\"display:none\">hidden</div><div>shown</div></body></html>",
            360.0,
        );
        let hidden = m
            .elements
            .iter()
            .find(|e| e.text.as_deref() == Some("hidden"))
            .expect("hidden element exists");
        assert!(!hidden.visible, "display:none must be invisible");
        assert_eq!(hidden.bbox.w, 0.0);
        assert_eq!(hidden.bbox.h, 0.0);
    }

    #[test]
    fn long_text_is_not_inlined_into_report() {
        let long = "x".repeat(100);
        let (m, _r) = analyze(&format!("<html><body><p>{}</p></body></html>", long), 360.0);
        let p = m.elements.iter().find(|e| e.tag == "p").expect("p exists");
        assert!(
            p.text.is_none(),
            "text over 60 chars should be omitted to keep the report lean"
        );
    }
}
