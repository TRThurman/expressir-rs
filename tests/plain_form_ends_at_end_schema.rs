//! The plain form (`no_remarks` with `keep_untagged_remarks`) is the schema
//! with its untagged comments. A comment that follows `END_SCHEMA;` stands
//! after the schema, so the plain form ends at `END_SCHEMA;`
//! (lutaml/expressir#758). The ordinary form keeps the comment.

const SOURCE: &str =
    "SCHEMA end_probe;\nENTITY a;\nEND_ENTITY; -- after END_ENTITY\nEND_SCHEMA; -- end_probe\n";

fn attached(source: &str) -> serde_json::Value {
    let mut wire = expressir::ParsedTree::parse(source)
        .expect("parses")
        .to_model_json("s.exp")
        .expect("wire");
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);
    wire
}

#[test]
fn the_plain_form_ends_at_end_schema() {
    let text = expressir::formatter::format_with_keep(&attached(SOURCE), true, true);
    assert!(text.ends_with("END_SCHEMA;\n"), "{text}");
    assert!(!text.contains("-- end_probe"), "{text}");
}

#[test]
fn the_plain_form_keeps_the_comment_after_another_end_keyword() {
    let text = expressir::formatter::format_with_keep(&attached(SOURCE), true, true);
    assert!(text.contains("END_ENTITY; -- after END_ENTITY"), "{text}");
}

#[test]
fn the_ordinary_form_keeps_the_comment_after_end_schema() {
    let text = expressir::formatter::format(&attached(SOURCE), false);
    assert!(text.ends_with("END_SCHEMA; -- end_probe\n"), "{text}");
}

#[test]
fn the_stripped_form_is_as_before() {
    let text = expressir::formatter::format(&attached(SOURCE), true);
    assert_eq!(
        text,
        "SCHEMA end_probe;\n\nENTITY a;\nEND_ENTITY;\n\nEND_SCHEMA;\n"
    );
}

#[test]
fn each_schema_of_a_file_ends_at_its_end_schema() {
    let source = "SCHEMA one;\nEND_SCHEMA; -- one\nSCHEMA two;\nEND_SCHEMA; -- two\n";
    let text = expressir::formatter::format_with_keep(&attached(source), true, true);
    assert!(!text.contains("--"), "{text}");
    assert_eq!(text.matches("END_SCHEMA;\n").count(), 2, "{text}");
}
