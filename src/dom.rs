use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

#[derive(Debug, Clone)]
pub struct ElNode {
    pub id: usize,
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Elem(ElNode),
    Text(String),
}

pub struct ParsedHtml {
    pub root: Node,
}

pub fn parse(html: &str) -> ParsedHtml {
    let opts = html5ever::ParseOpts::default();
    let dom: RcDom = parse_document(RcDom::default(), opts).one(html);

    let mut counter = 0usize;
    let root = convert(&dom.document, &mut counter);

    ParsedHtml { root }
}

fn alloc_id(counter: &mut usize) -> usize {
    let id = *counter;
    *counter += 1;
    id
}

fn convert(handle: &Handle, counter: &mut usize) -> Node {
    match &handle.data {
        NodeData::Document => {
            let mut children = Vec::new();
            for child in handle.children.borrow().iter() {
                let n = convert(child, counter);
                if !is_whitespace_only(&n) {
                    children.push(n);
                }
            }
            Node::Elem(ElNode {
                id: alloc_id(counter),
                tag: "#document".to_string(),
                attrs: Vec::new(),
                children,
            })
        }

        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.as_ref().to_string();
            let attrs = attrs
                .borrow()
                .iter()
                .map(|a| (a.name.local.as_ref().to_string(), a.value.to_string()))
                .collect::<Vec<_>>();

            let mut children = Vec::new();
            for child in handle.children.borrow().iter() {
                let n = convert(child, counter);
                if !is_whitespace_only(&n) {
                    children.push(n);
                }
            }

            Node::Elem(ElNode {
                id: alloc_id(counter),
                tag,
                attrs,
                children,
            })
        }

        NodeData::Text { contents } => Node::Text(contents.borrow().to_string()),

        _ => Node::Text(String::new()),
    }
}

fn is_whitespace_only(n: &Node) -> bool {
    match n {
        Node::Text(s) => s.trim().is_empty(),
        _ => false,
    }
}

impl ElNode {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}
