//! The moxml-compatible XML builder for the P28 writers (#640): the
//! ruby P28::Uos emit serializes through moxml (leptris adapter), and
//! the delegation must produce byte-identical text. The shape was
//! probed against moxml: no XML declaration, two-space indent per
//! level, newline between children and before a close tag, empty
//! elements as a pair (never self-closing), insertion-order
//! attributes, `&`/`<`/`>` escaped in text and additionally `"`/`\n`
//! in attribute values.

pub struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Option<Children>,
}

enum Children {
    Text(String),
    Nodes(Vec<Element>),
}

impl Element {
    pub fn new(name: impl Into<String>) -> Self {
        Element {
            name: name.into(),
            attrs: Vec::new(),
            children: Some(Children::Nodes(Vec::new())),
        }
    }

    pub fn attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attrs.push((name.into(), value.into()));
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.children = Some(Children::Text(text.into()));
        self
    }

    pub fn node(mut self, child: Element) -> Self {
        if let Some(Children::Nodes(nodes)) = &mut self.children {
            nodes.push(child);
        }
        self
    }

    pub fn to_xml(&self) -> String {
        // The moxml document serialization ends with a newline (probed).
        format!("{}\n", self.serialize(0))
    }

    fn serialize(&self, depth: usize) -> String {
        let ind = "  ".repeat(depth);
        let attrs: String = self
            .attrs
            .iter()
            .map(|(k, v)| format!(" {}={}", k, escape_attr(v)))
            .collect();
        let open = format!("{}<{}{}", ind, self.name, attrs);
        match &self.children {
            None => format!("{}></{}>", open, self.name),
            Some(Children::Nodes(nodes)) if nodes.is_empty() => {
                format!("{}></{}>", open, self.name)
            }
            Some(Children::Text(text)) => {
                format!("{}>{}</{}>", open, escape_text(text), self.name)
            }
            Some(Children::Nodes(nodes)) => {
                let inner: Vec<String> = nodes
                    .iter()
                    .map(|child| child.serialize(depth + 1))
                    .collect();
                format!("{}>\n{}\n{}</{}>", open, inner.join("\n"), ind, self.name)
            }
        }
    }
}

/// The bare-name form is unreachable behind `quoted` above; keep the
/// attribute string central so both branches stay in sync.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::Element;

    #[test]
    fn matches_the_probed_moxml_shape() {
        let doc = Element::new("iso_10303_28")
            .attr("xmlns", "urn:common")
            .node(Element::new("express").attr("id", "s1").text("SCHEMA x;"))
            .node(Element::new("empty"));
        assert_eq!(
            doc.to_xml(),
            "<iso_10303_28 xmlns=\"urn:common\">\n  <express id=\"s1\">SCHEMA x;</express>\n  <empty></empty>\n</iso_10303_28>"
        );
    }

    #[test]
    fn escapes_like_moxml() {
        let doc = Element::new("r")
            .attr("a", "q\"x & y<z>")
            .node(Element::new("t").text(">"));
        assert_eq!(
            doc.to_xml(),
            "<r a=\"q&quot;x &amp; y&lt;z&gt;\">\n  <t>&gt;</t>\n</r>"
        );
    }

    #[test]
    fn text_only_root_stays_inline() {
        assert_eq!(Element::new("a").text("b").to_xml(), "<a>b</a>");
    }
}
