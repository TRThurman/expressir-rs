//! A `--` comment after the opening parenthesis of the item list of a
//! USE FROM / REFERENCE FROM clause is written back after that
//! parenthesis (lutaml/expressir#ISSUE). The attacher gave it the opener
//! region, the same as a comment before the parenthesis, and the
//! formatter wrote both before the parenthesis.

const SOURCE: &str = "SCHEMA interface_probe;\nREFERENCE FROM first_schema ( -- after the parenthesis\n  a,\n  b);\nUSE FROM second_schema ( -- after the parenthesis of USE\n  c AS d);\nREFERENCE FROM third_schema -- before the parenthesis\n  (e,\n   f);\nUSE FROM fourth_schema; -- after the clause\nREFERENCE FROM fifth_schema\n  (g,\n   h); -- after the list\nREFERENCE FROM sixth_schema (i); -- one line\nENTITY part;\n  size : INTEGER;\nEND_ENTITY;\nEND_SCHEMA;\n";

const EXPECTED: &str = "SCHEMA interface_probe;\n\nREFERENCE FROM first_schema ( -- after the parenthesis\n   a,\n   b);\n\nUSE FROM second_schema ( -- after the parenthesis of USE\n   c AS d);\n\nREFERENCE FROM third_schema -- before the parenthesis\n  (e,\n   f);\n\nUSE FROM fourth_schema; -- after the clause\n\nREFERENCE FROM fifth_schema\n  (g,\n   h); -- after the list\n\nREFERENCE FROM sixth_schema\n  (i); -- one line\n\nENTITY part;\n  size : INTEGER;\nEND_ENTITY;\n\nEND_SCHEMA;\n";

fn attached(source: &str) -> serde_json::Value {
    let mut wire = expressir::ParsedTree::parse(source)
        .expect("parses")
        .to_model_json("s.exp")
        .expect("wire");
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);
    wire
}

fn placed(wire: &serde_json::Value, interface: usize) -> (Option<String>, Option<String>) {
    let ptr = format!("/schemas/0/interfaces/{interface}/untagged_remarks/0");
    let remark = wire
        .pointer(&ptr)
        .unwrap_or_else(|| panic!("no remark at {ptr}"));
    let field = |name: &str| {
        remark
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(String::from)
    };
    (field("placement"), field("region"))
}

#[test]
fn each_comment_is_written_on_the_side_of_the_parenthesis_it_had() {
    let text = expressir::formatter::format(&attached(SOURCE), false);
    assert_eq!(text, EXPECTED);
}

#[test]
fn the_same_text_again_from_its_own_output() {
    let once = expressir::formatter::format(&attached(SOURCE), false);
    assert_eq!(expressir::formatter::format(&attached(&once), false), once);
}

#[test]
fn the_region_tells_the_two_sides_apart() {
    let wire = attached(SOURCE);
    let inline = |region: Option<&str>| (Some("inline".to_string()), region.map(String::from));
    assert_eq!(placed(&wire, 0), inline(Some("items")));
    assert_eq!(placed(&wire, 1), inline(Some("items")));
    assert_eq!(placed(&wire, 2), inline(Some("opener")));
    assert_eq!(placed(&wire, 3), inline(None));
    assert_eq!(placed(&wire, 4), inline(None));
}

#[test]
fn a_comment_after_an_item_on_the_first_line_keeps_the_opener_region() {
    let source = "SCHEMA s;\nREFERENCE FROM t (a, -- after an item\n  b);\nEND_SCHEMA;\n";
    let wire = attached(source);
    assert_eq!(
        placed(&wire, 0),
        (Some("inline".to_string()), Some("opener".to_string()))
    );
}

#[test]
fn a_line_with_a_block_comment_as_well_is_left_as_it_was() {
    // `(* c *)` stands before the parenthesis and `-- d` after it. The
    // region is decided per line, so neither is moved.
    let source = "SCHEMA s;\nUSE FROM x (* c *) ( -- d\n  a);\nEND_SCHEMA;\n";
    let wire = attached(source);
    for remark in 0..2 {
        let region = wire
            .pointer(&format!(
                "/schemas/0/interfaces/0/untagged_remarks/{remark}/region"
            ))
            .and_then(serde_json::Value::as_str);
        assert_eq!(region, Some("opener"));
    }
    let once = expressir::formatter::format(&wire, false);
    assert_eq!(
        once,
        "SCHEMA s;\n\nUSE FROM x (* c *) -- d\n  (a);\nEND_SCHEMA;\n"
    );
    assert_eq!(expressir::formatter::format(&attached(&once), false), once);
}

#[test]
fn the_stripped_form_keeps_the_list_on_its_own_line() {
    let text = expressir::formatter::format(&attached(SOURCE), true);
    assert!(!text.contains("--"), "{text}");
    assert!(
        text.contains("REFERENCE FROM first_schema\n  (a,\n   b);"),
        "{text}"
    );
}
