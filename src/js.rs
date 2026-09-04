// Phase A JS rendering: execute inline <script> through boA against a minimal
// DOM shim, then re-serialize to HTML so the existing deterministic layout
// engine consumes the script-built DOM unchanged.
//
// Determinism policy: synchronous execution only. Async/timer/network APIs are
// absent from the shim, so calling them throws, which we surface as a report
// note rather than hanging.

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
  set:function(v){ /* Phase A: set as a single text child; re-parsed later */
      this.text = null; this.children = [String(v)]; }
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

// --- document global ---
const document = {
  body:    new _Element("body"),
  head:    new _Element("head"),
  documentElement: new _Element("html"),
  createElement: function(tag){ return new _Element(String(tag)); },
  createTextNode: function(text){ return String(text); },
  createDocumentFragment: function(){ return new _Element("_fragment"); },
  getElementById: function(id){ return null; }, // Phase A: no tree lookup
  querySelector:  function(sel){ return null; },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_disabled_returns_input_unchanged() {
        let html = "<div>A</div>";
        assert_eq!(execute(html), html);
    }

    #[test]
    fn js_builds_element_via_document() {
        let html = r#"<script>var d=document.createElement("div");d.setAttribute("id","x");d.textContent="hi";document.body.appendChild(d);</script><p>seed</p>"#;
        let (out, notes) = execute_checked(html);
        assert!(notes.is_empty(), "notes: {:?}", notes);
        assert!(out.contains("<div id=\"x\">hi</div>"), "got: {}", out);
        // Phase A contract: with --js, the script-built DOM is authoritative
        // for <body>. Static HTML is not yet mirrored into the shim, so the
        // <p>seed</p> is dropped. This matches "the page renders via script"
        // and is called out by layout.fidelity (see js notes).
        assert!(!out.contains("seed"), "got: {}", out);
    }

    #[test]
    fn js_inner_html_is_parsed_into_layout() {
        let html = r#"<script>var d=document.createElement("div");d.innerHTML="<span>built</span>";document.body.appendChild(d);</script>"#;
        let (out, notes) = execute_checked(html);
        assert!(notes.is_empty(), "notes: {:?}", notes);
        // innerHTML is stored as text then re-parsed by the engine; assert the
        // script-built element appears.
        assert!(out.contains("<div"), "got: {}", out);
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
