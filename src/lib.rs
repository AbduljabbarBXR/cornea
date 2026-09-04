//! Cornea — deterministic visual inspection for AI agents.
//!
//! Core library: parse HTML (+ minimal CSS), compute a deterministic visual
//! geometry model, and derive inspection conclusions (overlap, overflow,
//! contrast, quality).

pub mod css;
pub mod dom;
pub mod fetch;
pub mod inspect;
pub mod js;
pub mod layout;
pub mod model;
pub mod rest;

use model::VisualModel;

/// Full pipeline: HTML + viewport width -> visual model.
/// Viewport height 0 (default) means an unbounded scrolling page.
pub fn build_model(html: &str, viewport_w: f64) -> VisualModel {
    build_model_vh(html, viewport_w, 0.0)
}

/// Full pipeline with an explicit viewport height. A positive height enables
/// the below-the-fold clipping check; 0 keeps the unbounded-page semantics.
pub fn build_model_vh(html: &str, viewport_w: f64, viewport_h: f64) -> VisualModel {
    let parsed = dom::parse(html);
    let sheet = css::CssSheet::from_html(html);
    let engine = layout::LayoutEngine::new(&parsed.root, &sheet);
    engine.run_with_height(viewport_w, viewport_h)
}

/// Full pipeline producing both the visual model and the inspection report.
pub fn analyze(html: &str, viewport_w: f64) -> (VisualModel, inspect::InspectReport) {
    analyze_vh(html, viewport_w, 0.0)
}

/// Analyze with an explicit viewport height (see `build_model_vh`).
pub fn analyze_vh(
    html: &str,
    viewport_w: f64,
    viewport_h: f64,
) -> (VisualModel, inspect::InspectReport) {
    let model = build_model_vh(html, viewport_w, viewport_h);
    let mut report = inspect::inspect(&model);
    report.warnings = inspect::warnings(html, &report);
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

    fn contrast_row<'a>(
        r: &'a inspect::InspectReport,
        sel_suffix: &str,
    ) -> Option<&'a inspect::ContrastResult> {
        r.contrast.iter().find(|c| c.selector.ends_with(sel_suffix))
    }

    #[test]
    fn contrast_uses_painted_background_from_ancestors() {
        // white text on a black body: the p itself declares no background,
        // so the engine must walk up to body instead of assuming white
        let (_m, r) = analyze(
            "<html><body style=\"background-color:#000000\"><p style=\"color:#ffffff\">hi</p></body></html>",
            360.0,
        );
        let row = contrast_row(&r, ">p").expect("p contrast row present");
        assert!(
            row.pass_aa,
            "white on inherited black must pass AA: {}",
            row.note
        );
        assert_eq!(row.bg.as_deref(), Some("#000000"));
    }

    #[test]
    fn contrast_inherits_text_color_from_ancestors() {
        // body declares the text color; the p inherits it
        let (_m, r) = analyze(
            "<html><body style=\"color:#cccccc\"><p>x</p></body></html>",
            360.0,
        );
        let row = contrast_row(&r, ">p").expect("p contrast row present");
        assert_eq!(row.fg.as_deref(), Some("#cccccc"));
        assert!(!row.pass_aa, "inherited light grey on white must fail AA");
    }

    #[test]
    fn contrast_unresolved_color_never_passes() {
        let (_m, r) = analyze(
            "<html><body><p style=\"color:var(--brand)\">x</p></body></html>",
            360.0,
        );
        let row = contrast_row(&r, ">p").expect("p contrast row present");
        assert!(
            !row.pass_aa,
            "an unresolved color (var) must not silently pass AA"
        );
        assert!(row.ratio.is_none());
        assert!(row.note.contains("unresolved"), "note: {}", row.note);
    }

    #[test]
    fn contrast_parses_modern_color_formats() {
        let (_m, r) = analyze(
            "<html><body style=\"background:rgb(0,0,0)\"><p style=\"color:rgba(255,255,255,1)\">a</p><p style=\"color:hsl(0,0%,90%)\">b</p></body></html>",
            360.0,
        );
        let rows: Vec<&inspect::ContrastResult> = r
            .contrast
            .iter()
            .filter(|c| c.selector.ends_with(">p"))
            .collect();
        assert_eq!(
            rows.len(),
            2,
            "both p elements judged against body background"
        );
        let row = |fg: &str| rows.iter().find(|c| c.fg.as_deref() == Some(fg)).expect(fg);
        let white = row("rgba(255,255,255,1)");
        assert_eq!(
            white.ratio.map(|v| v.round()),
            Some(21.0),
            "white on black is 21:1"
        );
        assert!(white.pass_aa);
        let grey = row("hsl(0,0%,90%)");
        assert!(grey.ratio.unwrap() > 1.0);
        assert!(grey.pass_aa, "90% grey on black passes AA");
    }

    #[test]
    fn report_carries_page_level_warnings() {
        // a real-world page shape: external css + external script + media
        // queries + an unresolved var color; the report must say so itself
        let html = r#"<html><head>
            <link rel="stylesheet" href="/app.css">
            <style>@media (max-width: 600px) { .x { width: 50%; } }</style>
        </head><body>
            <script src="/bundle.js"></script>
            <p style="color:var(--brand)">hi</p>
        </body></html>"#;
        let (_m, r) = analyze(html, 360.0);
        let joined = r.warnings.join("\n");
        assert!(
            joined.contains("external stylesheet"),
            "warnings must flag <link> css: {}",
            joined
        );
        assert!(
            joined.contains("<script src>"),
            "warnings must flag external scripts: {}",
            joined
        );
        assert!(
            joined.contains("@media"),
            "warnings must flag media queries: {}",
            joined
        );
        assert!(
            joined.contains("unresolved color"),
            "warnings must flag unresolved colors: {}",
            joined
        );
        // a clean page stays quiet
        let (_m2, r2) = analyze("<html><body><p>plain</p></body></html>", 360.0);
        assert!(r2.warnings.is_empty(), "clean page must have no warnings");
    }
}
