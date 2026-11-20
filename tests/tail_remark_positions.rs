#[test]
fn label_remark_renders_on_the_label_line() {
    let source = "SCHEMA t;\nFUNCTION g (s : INTEGER) : INTEGER;\n  CASE s OF\n    1 : -- why\n      v := 1;\n  END_CASE;\n  RETURN (v);\n  END_FUNCTION;\nEND_SCHEMA;\n";
    let mut wire = expressir::ParsedTree::parse(source).expect("parses").to_model_json("t.exp").expect("wire");
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);
    // confirm the attach placed the label remark
    let case = wire.pointer("/schemas/0/functions/0/statements/0").expect("case");
    let remarks = case.get("untagged_remarks").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let label_remarks: Vec<_> = remarks.iter().filter(|r| {
        r.get("placement").and_then(|p| p.as_str()) == Some("inline")
            && r.get("region").and_then(|g| g.as_str()) == Some("label")
    }).collect();
    assert!(!label_remarks.is_empty(), "no label remark on the case: {:?}", remarks);
    let text = expressir::formatter::format(&wire, false);
    assert!(text.contains("1 : -- why"), "label remark not rendered:\n{text}");
}
