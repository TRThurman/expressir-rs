//! String-literal round trips (#697, #698): a body with leading blanks
//! keeps them (the pre-atomic rule let trivia eat the first blank), and
//! the doubled apostrophe is the in-string escape. The model keeps the
//! source spelling, so the formatter re-emits it verbatim.

#[test]
fn string_literals_round_trip() {
    let source = "SCHEMA s;\nCONSTANT\n  one_blank : STRING := ' ';\n  leading : STRING := ' a';\n  empty : STRING := '';\n  apos : STRING := 'it''s';\n  four : STRING := '''';\n  head3 : STRING := '''a';\n  tail3 : STRING := 'a''';\n  two : STRING := 'a' + 'b';\nEND_CONSTANT;\nEND_SCHEMA;\n";
    let mut wire = expressir::ParsedTree::parse(source)
        .expect("parses")
        .to_model_json("s.exp")
        .expect("wire");
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);

    let constants = wire
        .pointer("/schemas/0/constants")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let value_of = |id: &str| -> String {
        constants
            .iter()
            .find(|c| c.get("id").and_then(|i| i.as_str()) == Some(id))
            .and_then(|c| c.pointer("/expression/value"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(value_of("one_blank"), " ");
    assert_eq!(value_of("leading"), " a");
    assert_eq!(value_of("empty"), "");
    assert_eq!(value_of("apos"), "it''s");
    assert_eq!(value_of("four"), "''");
    assert_eq!(value_of("head3"), "''a");
    assert_eq!(value_of("tail3"), "a''");

    let text = expressir::formatter::format(&wire, false);
    for literal in [
        "' '", "' a'", "''", "'it''s'", "''''", "'''a'", "'a'''",
    ] {
        assert!(text.contains(literal), "{literal} not written:\n{text}");
    }
    assert!(text.contains(":= 'a' + 'b';"), "adjacent literals:\n{text}");
}
