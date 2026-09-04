// Phase A JS rendering: execute inline <script> through boA against a minimal
// DOM shim, then re-serialize to HTML so the existing deterministic layout
// engine consumes the script-built DOM unchanged.
//
// Contract: when --js is on, the static HTML is first mirrored into the shim
// (head and body, scripts excluded), then page scripts run against the seeded
// tree and may attach to it (getElementById etc.), then the merged DOM is
// serialized. Static content is preserved, not dropped.
//
// Determinism policy: synchronous execution only. Async/timer/network APIs are
// absent from the shim, so calling them throws, which we surface as a report
// note rather than hanging.

use crate::dom::{self, Node};
use boa_engine::Context;
use boa_engine::Source;

/// Minimal DOM shim written in ES. Backs a small tree and produces a canonical
/// HTML string via __cornea_serialize() so Cornea's static engine re-parses it.
const DOM_SHIM: &str = r#"
"use strict";
// --- node list helper ---
function _html_escape(s){ return String(s)
    .replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;")
    .replace(/"/g,"&quot;"); }
function _attr_escape(s){ return String(s)
    .replace(/&/g,"&amp;").replace(/"/g,"&quot;"); }

function _Element(tag){
    this.tag      = tag;
    this.attrs    = {};       // name -> value
    this.children = [];       // Element | string (text)
    this.text     = null;     // textContent override
}
_Element.prototype.setAttribute = function(name,value){
    this.attrs[String(name)] = String(value); return undefined;
};
_Element.prototype.getAttribute = function(name){
    return Object.prototype.hasOwnProperty.call(this.attrs,name) ? this.attrs[name] : null;
};
_Element.prototype.appendChild = function(child){
    this.children.push(child); return child;
};
_Element.prototype.insertBefore = function(child){
    this.children.unshift(child); return child;
};
_Element.prototype.removeChild = function(child){
    var i = this.children.indexOf(child);
    if (i >= 0) this.children.splice(i,1); return child;
};
Object.defineProperty(_Element.prototype,"textContent",{
  get:function(){ return this.text !== null ? this.text : this.children.map(function(c){
      return typeof c === "string" ? c : c.textContent; }).join(""); },
  set:function(v){ this.text = String(v); }
});
Object.defineProperty(_Element.prototype,"innerHTML",{
  get:function(){ return this.serializeChildren(); },
  set:function(v){ /* parse real markup into element children */
      this.text = null; this.children = _parseFragment(String(v));
  }
});
_Element.prototype.serializeChildren = function(){
  var out="";
  for (var i=0;i<this.children.length;i++){
    var c=this.children[i];
    if (typeof c === "string"){ out += _html_escape(c); continue; }
    out += c.serialize();
  }
  return out;
};
_Element.prototype.serialize = function(){
  if (this.text !== null){
    // a leaf whose children were superseded by textContent
    return "<"+this.tag+this._attrsStr()+">"+_html_escape(this.text)+"</"+this.tag+">";
  }
  var as = this._attrsStr();
  var inner = this.serializeChildren();
  return "<"+this.tag+as+">"+inner+"</"+this.tag+">";
};
_Element.prototype._attrsStr = function(){
  var s="";
  for (var k in this.attrs){
    if (Object.prototype.hasOwnProperty.call(this.attrs,k)){
      s += " "+k+'="'+_attr_escape(this.attrs[k])+'"';
    }
  }
  if (this._styleProps !== undefined && this._styleProps !== null){
    var styleParts=[];
    for (var p in this._styleProps){
      if (Object.prototype.hasOwnProperty.call(this._styleProps,p) && this._styleProps[p] !== ""){
        // camelCase -> kebab-case for CSS
        var kebab = p.replace(/[A-Z]/g, function(m){ return "-"+m.toLowerCase(); });
        styleParts.push(kebab+": "+this._styleProps[p]+";");
      }
    }
    if (styleParts.length){
      s += ' style="'+_attr_escape(styleParts.join(" "))+'"';
    }
  }
  return s;
};
// style object: style.background etc map to style="..."
Object.defineProperty(_Element.prototype,"style",{ get:function(){
  var el=this;
  return new Proxy({}, { get:function(t,p){
      if (typeof p==="symbol") return undefined;
      return el._styleProps[p] !== undefined ? el._styleProps[p] : "";
    }, set:function(t,p,v){
      if (typeof p!=="symbol"){ if (el._styleProps===undefined) el._styleProps={}; el._styleProps[p]=String(v); }
      return true;
    }});
}});
_Element.prototype._attrsStr_injectStyle = function(){ return ""; };

// --- common element surface the seed tree and scripts rely on ---
Object.defineProperty(_Element.prototype,"tagName",{ get:function(){ return this.tag.toUpperCase(); }});
Object.defineProperty(_Element.prototype,"id",{
  get:function(){ return this.attrs["id"] !== undefined ? this.attrs["id"] : ""; },
  set:function(v){ this.attrs["id"] = String(v); }
});
Object.defineProperty(_Element.prototype,"className",{
  get:function(){ return this.attrs["class"] !== undefined ? this.attrs["class"] : ""; },
  set:function(v){ this.attrs["class"] = String(v); }
});
Object.defineProperty(_Element.prototype,"classList",{ get:function(){
  var el = this;
  function classes(){ var c = el.attrs["class"]; return c ? c.split(/\s+/).filter(Boolean) : []; }
  return {
    add:function(){ var cur = classes(); for (var i=0;i<arguments.length;i++){ if (cur.indexOf(arguments[i])<0) cur.push(arguments[i]); } el.attrs["class"]=cur.join(" "); },
    remove:function(){ var cur = classes().filter(function(c){ for (var i=0;i<arguments.length;i++){ if (c===arguments[i]) return false; } return true; }); el.attrs["class"]=cur.join(" "); },
    contains:function(c){ return classes().indexOf(c) >= 0; },
    toggle:function(c){ var cur = classes(); var i = cur.indexOf(c); if (i>=0){ cur.splice(i,1); } else { cur.push(c); } el.attrs["class"]=cur.join(" "); }
  };
}});

// --- minimal innerHTML fragment parser (deterministic, tag soup subset) ---
function _isVoid(tag){ return /^(area|base|br|col|embed|hr|img|input|link|meta|param|source|track|wbr)$/.test(tag); }
function _parseAttrs(str){
  var m = {}; var re = /([a-zA-Z_:][-a-zA-Z0-9_:.]*)(?:\s*=\s*("([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g; var mm;
  while ((mm = re.exec(str)) !== null){
    m[mm[1].toLowerCase()] = mm[3] !== undefined ? mm[3] : (mm[4] !== undefined ? mm[4] : (mm[5] !== undefined ? mm[5] : ""));
  }
  return m;
}
function _decodeEntities(str){
  var named = { amp:"&", lt:"<", gt:">", quot:'"', apos:"'", nbsp:"\u00a0" };
  return str.replace(/&(#x[0-9a-fA-F]+|#[0-9]+|[a-zA-Z]+);/g, function(m, body){
    if (body[0] === '#'){
      var code = body[1] === "x" || body[1] === "X" ? parseInt(body.slice(2), 16) : parseInt(body.slice(1), 10);
      if (!isNaN(code)) { try { return String.fromCharCode(code); } catch(e){} }
      return m;
    }
    return named[body.toLowerCase()] !== undefined ? named[body.toLowerCase()] : m;
  });
}
function _parseFragment(src){
  var root = { children: [] };
  var stack = [root];
  var i = 0;
  while (i < src.length){
    var lt = src.indexOf("<", i);
    if (lt < 0){ stack[stack.length-1].children.push(_decodeEntities(src.slice(i))); break; }
    if (lt > i){ stack[stack.length-1].children.push(_decodeEntities(src.slice(i, lt))); }
    if (src.startsWith("<!--", lt)){
      var ce = src.indexOf("-->", lt + 4);
      i = ce < 0 ? src.length : ce + 3;
      continue;
    }
    if (src.charAt(lt+1) === "!" || src.charAt(lt+1) === "?"){
      var ce2 = src.indexOf(">", lt);
      i = ce2 < 0 ? src.length : ce2 + 1;
      continue;
    }
    if (src.charAt(lt+1) === "/"){
      var gt2 = src.indexOf(">", lt);
      i = gt2 < 0 ? src.length : gt2 + 1;
      if (stack.length > 1) stack.pop();
      continue;
    }
    var gt = src.indexOf(">", lt);
    if (gt < 0){ stack[stack.length-1].children.push(_decodeEntities(src.slice(lt))); break; }
    var raw = src.slice(lt + 1, gt);
    var selfClose = raw.charAt(raw.length - 1) === "/";
    var tagM = /^([a-zA-Z][a-zA-Z0-9-]*)/.exec(raw);
    if (!tagM){ i = gt + 1; continue; }
    var el = new _Element(tagM[1].toLowerCase());
    var parsed = _parseAttrs(raw.slice(tagM[0].length));
    for (var k in parsed){ if (parsed.hasOwnProperty(k)) el.attrs[k] = parsed[k]; }
    stack[stack.length - 1].children.push(el);
    if (!selfClose && !_isVoid(el.tag)) stack.push(el);
    i = gt + 1;
  }
  return root.children;
}

// --- deterministic tree lookups ---
function _findById(el, id){
  if (el.tag !== "_fragment" && el.attrs["id"] !== undefined && el.attrs["id"] === id) return el;
  for (var i = 0; i < el.children.length; i++){
    var c = el.children[i];
    if (typeof c === "string") continue;
    var f = _findById(c, id);
    if (f !== null) return f;
  }
  return null;
}
function _findByTag(el, tag, out){
  if (el.tag === tag) out.push(el);
  for (var i = 0; i < el.children.length; i++){
    var c = el.children[i];
    if (typeof c === "string") continue;
    _findByTag(c, tag, out);
  }
  return out;
}

// --- seeding: static HTML is mirrored into the shim before scripts run ---
function __seedInto(container, items){
  for (var i = 0; i < items.length; i++){
    var it = items[i];
    if (typeof it === "string"){ container.children.push(it); continue; }
    var el = new _Element(it.t);
    var a = it.a || {};
    for (var k in a){ if (a.hasOwnProperty(k)) el.attrs[k] = String(a[k]); }
    if (it.c !== undefined && it.c.length) __seedInto(el, it.c);
    container.children.push(el);
  }
}
function __applySeed(containerName, items){
  var container = containerName === "head" ? document.head : document.body;
  __seedInto(container, items || []);
}

// --- document global ---
const document = {
  body:    new _Element("body"),
  head:    new _Element("head"),
  documentElement: new _Element("html"),
  createElement: function(tag){ return new _Element(String(tag).toLowerCase()); },
  createTextNode: function(text){ return String(text); },
  createDocumentFragment: function(){ return new _Element("_fragment"); },
  getElementById: function(id){ return _findById(document.documentElement, String(id)); },
  getElementsByTagName: function(tag){ return _findByTag(document.documentElement, String(tag).toLowerCase(), []); },
  querySelector:  function(sel){ return null; }, // Phase A: no selector engine
  querySelectorAll: function(sel){ return []; },
};
document.documentElement.appendChild(document.head);
document.documentElement.appendChild(document.body);

// --- window stub (minimal) ---
const window = { document: document };
window.addEventListener = function(){};
document.addEventListener = function(){};

// Serialize <body> content into a canonical HTML doc for Cornea's engine.
function __cornea_serialize(){
  return "<!doctype html><html>"+document.documentElement.serializeChildren()+"</html>";
}
"#;

/// Find inline `<script>...</script>` bodies only (Phase A: no src, no async).
/// Returns concatenated script source.
fn inline_scripts(html: &str) -> String {
    let mut out = String::new();
    let lower = html.to_ascii_lowercase();
    let mut idx = 0usize;
    while let Some(rel) = lower[idx..].find("<script") {
        let start_abs = idx + rel;
        idx = start_abs;
        // find end of open tag
        let end_tag = match lower[idx..].find('>') {
            Some(e) => idx + e,
            None => break,
        };
        // skip if it's a self-closing or has src= (external) — Phase A inline only
        if lower[idx..end_tag].contains("src=") {
            idx = end_tag + 1;
            continue;
        }
        let close_search = &lower[end_tag + 1..];
        match close_search.find("</script") {
            Some(c) => {
                out.push_str(&html[end_tag + 1..end_tag + 1 + c]);
                idx = end_tag + 1 + c + "</script".len();
            }
            None => break,
        }
    }
    out
}

/// Execute inline scripts against the DOM shim and return the re-serialized
/// HTML. If JS is disabled or there is nothing to run, returns the input
/// unchanged. On a script error, returns the original HTML (the engine will
/// note the failure to the caller via `execute_checked`).
pub fn execute(html: &str) -> String {
    execute_checked(html).0
}

/// `(html, notes)`: notes carries script warnings (unsupported API / errors).
pub fn execute_checked(html: &str) -> (String, Vec<String>) {
    let scripts = inline_scripts(html);
    if scripts.trim().is_empty() {
        return (html.to_string(), Vec::new());
    }

    let mut ctx = Context::default();
    let mut notes = Vec::new();

    // Load the DOM shim first.
    match ctx.eval(Source::from_bytes(DOM_SHIM)) {
        Ok(_) => {}
        Err(e) => {
            notes.push(format!("js shim error: {}", e));
            return (html.to_string(), notes);
        }
    }

    // Mirror the static HTML (head and body, scripts excluded) so scripts can
    // find and extend existing content instead of replacing the whole page.
    if let Err(e) = seed_static(&mut ctx, html) {
        notes.push(format!("js seed error: {}", e));
        return (html.to_string(), notes);
    }

    // Run the page scripts.
    if let Err(e) = ctx.eval(Source::from_bytes(&scripts)) {
        notes.push(format!("js script error: {}", e));
        return (html.to_string(), notes);
    }

    // Serialize back to HTML.
    match ctx.eval(Source::from_bytes("__cornea_serialize()")) {
        Ok(v) => match v.to_string(&mut ctx) {
            Ok(s) => (s.to_std_string_lossy(), notes),
            Err(e) => {
                notes.push(format!("js serialize error: {}", e));
                (html.to_string(), notes)
            }
        },
        Err(e) => {
            notes.push(format!("js serialize failed: {}", e));
            (html.to_string(), notes)
        }
    }
}

/// Parse the static HTML and push its head/body (minus scripts) into the
/// shim via `__applySeed`. Deterministic: the seed is generated from the
/// parsed tree, so it survives html5ever's implied-element repairs.
fn seed_static(ctx: &mut Context, html: &str) -> Result<(), boa_engine::JsError> {
    let (head, body) = seed_lists(html);
    let prog = format!(
        "__applySeed(\"head\", {});\n__applySeed(\"body\", {});",
        head, body
    );
    ctx.eval(Source::from_bytes(prog.as_bytes()))?;
    Ok(())
}

/// Split the parsed document into JSON children arrays for head and body.
fn seed_lists(html: &str) -> (String, String) {
    let parsed = dom::parse(html);
    let mut head = Vec::new();
    let mut body = Vec::new();
    if let Node::Elem(doc) = &parsed.root {
        for c in &doc.children {
            if let Node::Elem(html_el) = c
                && html_el.tag == "html"
            {
                for h in &html_el.children {
                    if let Node::Elem(section) = h {
                        if section.tag == "head" {
                            head = section.children.iter().filter_map(node_json).collect();
                        } else if section.tag == "body" {
                            body = section.children.iter().filter_map(node_json).collect();
                        }
                    }
                }
            }
        }
    }
    (
        serde_json::to_string(&head).unwrap_or_else(|_| "[]".into()),
        serde_json::to_string(&body).unwrap_or_else(|_| "[]".into()),
    )
}

/// One parsed node -> compact seed item (JSON is a valid JS expression, so it
/// can be injected verbatim). Scripts are excluded; whitespace-only text was
/// already dropped by the parser.
fn node_json(n: &Node) -> Option<serde_json::Value> {
    match n {
        Node::Text(s) => Some(serde_json::Value::String(s.clone())),
        Node::Elem(e) => {
            if e.tag == "script" {
                return None;
            }
            let mut obj = serde_json::Map::new();
            obj.insert("t".into(), serde_json::Value::String(e.tag.clone()));
            if !e.attrs.is_empty() {
                let mut a = serde_json::Map::new();
                for (k, v) in &e.attrs {
                    a.insert(k.clone(), serde_json::Value::String(v.clone()));
                }
                obj.insert("a".into(), serde_json::Value::Object(a));
            }
            let kids: Vec<serde_json::Value> = e.children.iter().filter_map(node_json).collect();
            if !kids.is_empty() {
                obj.insert("c".into(), serde_json::Value::Array(kids));
            }
            Some(serde_json::Value::Object(obj))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_disabled_returns_input_unchanged() {
        let html = "<div>A</div>";
        assert_eq!(execute(html), html);
    }

    #[test]
    fn js_builds_element_via_document_and_keeps_static_content() {
        let html = r#"<script>var d=document.createElement("div");d.setAttribute("id","x");d.textContent="hi";document.body.appendChild(d);</script><p>seed</p>"#;
        let (out, notes) = execute_checked(html);
        assert!(notes.is_empty(), "notes: {:?}", notes);
        // static markup is mirrored into the shim, so it survives script runs
        assert!(
            out.contains("seed"),
            "static content must be preserved: {}",
            out
        );
        assert!(
            out.contains("<div id=\"x\">hi</div>"),
            "script-built element must be present: {}",
            out
        );
        assert!(
            !out.contains("<script"),
            "scripts must not reserialize: {}",
            out
        );
    }

    #[test]
    fn js_inner_html_parses_markup_into_elements() {
        let html = r#"<script>var d=document.createElement("div");d.innerHTML="<span>built</span>";document.body.appendChild(d);</script>"#;
        let (out, notes) = execute_checked(html);
        assert!(notes.is_empty(), "notes: {:?}", notes);
        assert!(
            out.contains("<span>built</span>"),
            "innerHTML markup must become real elements, got: {}",
            out
        );
        assert!(
            !out.contains("&lt;span"),
            "markup must not leak as escaped text: {}",
            out
        );
    }

    #[test]
    fn js_get_element_by_id_mounts_into_static_html() {
        let html = r#"<div id="root"><em>replace me</em></div><script>var r=document.getElementById("root");r.innerHTML="<i>x</i>";</script>"#;
        let (out, notes) = execute_checked(html);
        assert!(notes.is_empty(), "notes: {:?}", notes);
        assert!(
            out.contains("<div id=\"root\"><i>x</i></div>"),
            "script must mount into the seeded static node, got: {}",
            out
        );
        assert!(
            !out.contains("replace me"),
            "innerHTML must replace content: {}",
            out
        );
    }

    #[test]
    fn js_execution_is_byte_deterministic() {
        let html = r#"<div id="root"></div><script>var r=document.getElementById("root");r.innerHTML="<i>x</i>";</script>"#;
        let (a, _) = execute_checked(html);
        let (b, _) = execute_checked(html);
        assert_eq!(a, b, "same scripted page must serialize identically");
    }

    #[test]
    fn js_error_is_reported_not_hung() {
        let html = r#"<script>throw new Error("boom");</script><p>ok</p>"#;
        let (_out, notes) = execute_checked(html);
        assert!(!notes.is_empty(), "should surface the script error");
        assert!(
            notes.iter().any(|n| n.contains("boom")),
            "notes: {:?}",
            notes
        );
    }

    #[test]
    fn js_unsupported_api_throws_and_is_noted() {
        let html = r#"<script>setTimeout(function(){},10);</script><p>ok</p>"#;
        let (_out, notes) = execute_checked(html);
        assert!(
            !notes.is_empty(),
            "setTimeout is not defined in shim; must error not hang"
        );
    }

    #[test]
    fn inline_scripts_extractor() {
        let html = r#"<html><head><script>a()</script></head><body><div></div><script src="x.js"></script><script>b()</script></body></html>"#;
        let js = inline_scripts(html);
        assert!(!js.contains("x.js"), "external src must be skipped");
        assert!(js.contains("a()"), "inline head script included");
        assert!(js.contains("b()"), "inline body script included");
    }
}
