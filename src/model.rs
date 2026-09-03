use serde::Serialize;

/// A resolved bounding box in viewport space (CSS px, top-left origin).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }
    /// Intersection area of two rects (0 = no overlap).
    pub fn overlap_area(&self, o: &Rect) -> f64 {
        let inter_w = (self.x + self.w).min(o.x + o.w) - self.x.max(o.x);
        let inter_h = (self.y + self.h).min(o.y + o.h) - self.y.max(o.y);
        if inter_w > 0.0 && inter_h > 0.0 {
            inter_w * inter_h
        } else {
            0.0
        }
    }
    pub fn intersection(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let w = (self.x + self.w).min(o.x + o.w) - x;
        let h = (self.y + self.h).min(o.y + o.h) - y;
        Rect::new(x, y, w.max(0.0), h.max(0.0))
    }
}

/// One visible element's resolved geometry + computed styles.
#[derive(Debug, Clone, Serialize)]
pub struct ElementView {
    pub id: usize,
    pub parent: usize,
    pub tag: String,
    pub selector: String,
    pub role: String,
    pub display: String,
    pub visible: bool,
    #[serde(rename = "box")]
    pub bbox: Rect,
    pub content: Rect,
    pub z: i32,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub text: Option<String>,
    pub fidelity: String,
}

/// The full visual geometry model.
#[derive(Debug, Clone, Serialize)]
pub struct VisualModel {
    pub viewport: Rect,
    pub element_count: usize,
    pub elements: Vec<ElementView>,
}
