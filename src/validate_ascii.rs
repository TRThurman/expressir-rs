//! The ASCII-validation engine (#643): the non-ASCII scan, character
//! classification, and replacement suggestions — the engine behind the
//! ruby `validate ascii` command. The presentation (tables, paint,
//! YAML rendering) stays ruby-tier; this module produces the data.

use serde::Serialize;

use crate::remark;

/// The common Unicode math symbols with their AsciiMath replacements
/// (the ruby UNICODE_TO_ASCIIDOC_MAP, mirrored exactly).
const UNICODE_TO_ASCIIMATH: &[(&str, &str)] = &[
    ("α", "alpha"),
    ("β", "beta"),
    ("γ", "gamma"),
    ("Γ", "Gamma"),
    ("δ", "delta"),
    ("Δ", "Delta"),
    ("ε", "epsilon"),
    ("ζ", "zeta"),
    ("η", "eta"),
    ("θ", "theta"),
    ("Θ", "Theta"),
    ("ι", "iota"),
    ("κ", "kappa"),
    ("λ", "lambda"),
    ("Λ", "Lambda"),
    ("μ", "mu"),
    ("ν", "nu"),
    ("ξ", "xi"),
    ("Ξ", "Xi"),
    ("π", "pi"),
    ("Π", "Pi"),
    ("ρ", "rho"),
    ("σ", "sigma"),
    ("Σ", "Sigma"),
    ("τ", "tau"),
    ("υ", "upsilon"),
    ("φ", "phi"),
    ("Φ", "Phi"),
    ("χ", "chi"),
    ("ψ", "psi"),
    ("Ψ", "Psi"),
    ("ω", "omega"),
    ("Ω", "Omega"),
    ("×", "xx"),
    ("÷", "div"),
    ("±", "pm"),
    ("∓", "mp"),
    ("∞", "oo"),
    ("≤", "le"),
    ("≥", "ge"),
    ("≠", "ne"),
    ("≈", "~~"),
    ("≅", "cong"),
    ("≡", "equiv"),
    ("∈", "in"),
    ("∉", "notin"),
    ("⊂", "subset"),
    ("⊃", "supset"),
    ("∩", "cap"),
    ("∪", "cup"),
    ("∧", "and"),
    ("∨", "or"),
    ("¬", "neg"),
    ("∀", "forall"),
    ("∃", "exists"),
    ("∄", "nexists"),
    ("∇", "grad"),
    ("∂", "del"),
    ("∑", "sum"),
    ("∏", "prod"),
    ("∫", "int"),
    ("∮", "oint"),
    ("√", "sqrt"),
    ("⊥", "perp"),
    ("‖", "norm"),
    ("→", "rarr"),
    ("←", "larr"),
    ("↔", "harr"),
    ("⇒", "rArr"),
    ("⇐", "lArr"),
    ("⇔", "hArr"),
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CharDetail {
    #[serde(rename = "char")]
    pub character: String,
    pub hex: String,
    pub utf8: String,
    pub is_math: bool,
    pub replacement: String,
    pub replacement_type: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Violation {
    pub line_number: usize,
    pub column: usize,
    pub match_text: String,
    pub line: String,
    pub char_details: Vec<CharDetail>,
}

/// Scans one file's source for non-ASCII violations. Without
/// `check_remarks`, embedded remark regions (`(* ... *)`) are masked
/// to spaces first — non-ASCII inside documentation blocks is not
/// flagged; tail remarks (`--`) are part of code lines and stay in
/// scope (issue #285). Byte positions and line alignment are
/// preserved by the mask.
pub fn scan_source(source: &str, check_remarks: bool) -> Vec<Violation> {
    let scan_source = if check_remarks {
        source.to_string()
    } else {
        mask_embedded_remarks(source)
    };

    let mut violations = Vec::new();
    for (idx, line) in scan_source.lines().enumerate() {
        let line_number = idx + 1;
        let mut chars = line.char_indices().peekable();
        let all: Vec<(usize, char)> = line.char_indices().collect();
        let _ = &mut chars;
        let mut i = 0usize;
        while i < all.len() {
            if (all[i].1 as u32) <= 0x7F {
                i += 1;
                continue;
            }
            // A run of consecutive non-ASCII characters.
            let start = i;
            while i < all.len() && (all[i].1 as u32) > 0x7F {
                i += 1;
            }
            let run: String = all[start..i].iter().map(|(_, c)| c).collect();
            let column = all[start].0;
            // ruby String#index: the match column is the CHARACTER index
            // within the line, not the byte offset.
            let char_column = line[..column].chars().count();
            let char_details: Vec<CharDetail> =
                run.chars().filter_map(process_non_ascii_char).collect();
            if char_details.is_empty() {
                continue;
            }
            violations.push(Violation {
                line_number,
                column: char_column,
                match_text: run,
                line: line.to_string(),
                char_details,
            });
        }
    }
    violations
}

/// Builds a masked copy where every byte inside an embedded remark
/// region becomes an ASCII space (the ruby mask_embedded_remarks).
fn mask_embedded_remarks(source: &str) -> String {
    let remarks = remark::scan(source);
    let embedded: Vec<_> = remarks.iter().filter(|r| r.embedded()).collect();
    if embedded.is_empty() {
        return source.to_string();
    }
    let mut bytes = source.as_bytes().to_vec();
    for region in embedded {
        for b in &mut bytes[region.position..region.end_position] {
            *b = 0x20;
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| source.to_string())
}

fn process_non_ascii_char(c: char) -> Option<CharDetail> {
    if (c as u32) <= 0x7F {
        return None;
    }
    let code_point = c as u32;
    let hex = format!("0x{code_point:x}");
    let utf8 = {
        let mut buf = [0u8; 4];
        let encoded = c.encode_utf8(&mut buf);
        encoded
            .bytes()
            .map(|b| format!("0x{b:x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };

    let text = c.to_string();
    if let Some((_, replacement)) = UNICODE_TO_ASCIIMATH.iter().find(|(k, _)| *k == text) {
        return Some(CharDetail {
            character: text,
            hex,
            utf8,
            is_math: true,
            replacement: (*replacement).to_string(),
            replacement_type: "asciimath".to_string(),
        });
    }

    Some(CharDetail {
        character: text,
        hex,
        utf8,
        is_math: false,
        replacement: encode_express_format(code_point),
        replacement_type: "iso-10303-11".to_string(),
    })
}

/// The ISO 10303-11 encoded form: an 8-hex-digit body for the BMP,
/// the four-octet group/plane/row/cell form above it.
fn encode_express_format(code_point: u32) -> String {
    if code_point < 0x1_0000 {
        format!("\"{:08X}\"", code_point)
    } else {
        let group = (code_point >> 24) & 0xFF;
        let plane = (code_point >> 16) & 0xFF;
        let row = (code_point >> 8) & 0xFF;
        let cell = code_point & 0xFF;
        format!("\"{group:02X}{plane:02X}{row:02X}{cell:02X}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::scan_source;

    #[test]
    fn scans_runs_and_suggests_replacements() {
        let source = "SCHEMA s;\n  a : STRING := 'α ≤ β';\nEND_SCHEMA;\n";
        // Each consecutive non-ASCII run is its own violation (the
        // ruby scan regex matches runs, not whole lines).
        let violations = scan_source(source, false);
        assert_eq!(violations.len(), 3);
        assert_eq!(violations[0].match_text, "α");
        assert_eq!(violations[1].match_text, "≤");
        assert_eq!(violations[2].match_text, "β");
        for v in &violations {
            assert_eq!(v.line_number, 2);
            assert_eq!(v.char_details.len(), 1);
            assert!(v.char_details[0].is_math);
        }
        assert_eq!(violations[0].char_details[0].replacement, "alpha");
        assert_eq!(violations[1].char_details[0].replacement, "le");
        assert_eq!(violations[2].char_details[0].replacement, "beta");
    }

    #[test]
    fn masks_embedded_remarks_unless_asked() {
        let source = "SCHEMA s;\n(* α documentation *)\n  a : STRING;\nEND_SCHEMA;\n";
        assert!(scan_source(source, false).is_empty());
        let checked = scan_source(source, true);
        assert_eq!(checked.len(), 1);
        assert_eq!(checked[0].char_details[0].replacement, "alpha");
    }

    #[test]
    fn iso_encoding_for_unknown_chars() {
        let source = "SCHEMA s;\n  a : BINARY;\n  b : STRING := '中';\nEND_SCHEMA;\n";
        let violations = scan_source(source, false);
        assert_eq!(violations.len(), 1);
        let detail = &violations[0].char_details[0];
        assert!(!detail.is_math);
        assert_eq!(detail.replacement_type, "iso-10303-11");
        assert_eq!(detail.replacement, "\"00004E2D\"");
    }
}
