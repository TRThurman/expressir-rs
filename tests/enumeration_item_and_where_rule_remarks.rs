//! A `--` comment at the end of the line of an enumeration item, and one
//! on the line of a WHERE rule (after the rule, or after its label), is
//! written back after the code it follows (lutaml/expressir#724). The
//! wire held these remarks on the item and on the rule, unplaced, and no
//! emitter wrote them.

const SOURCE: &str = "SCHEMA lost_probe;\nTYPE colour = ENUMERATION OF\n  (red, -- warm\n   green); -- cold\nEND_TYPE;\nENTITY part;\n  size : INTEGER;\nWHERE\n  wr1 : size > 0; -- not empty\n  wr2 : -- upper limit\n    size < 100;\nEND_ENTITY;\nTYPE positive = INTEGER;\nWHERE\n  wr1 : SELF > 0; -- of a type\nEND_TYPE;\nRULE r FOR (part);\nWHERE\n  wr1 : -- of a rule\n    TRUE;\nEND_RULE;\nEND_SCHEMA;\n";

const EXPECTED: &str = "SCHEMA lost_probe;\n\nTYPE colour = ENUMERATION OF\n  (red, -- warm\n   green); -- cold\nEND_TYPE;\n\nENTITY part;\n  size : INTEGER;\nWHERE\n  wr1: size > 0; -- not empty\n  wr2: -- upper limit\n    size < 100;\nEND_ENTITY;\n\nTYPE positive = INTEGER;\nWHERE\n  wr1: SELF > 0; -- of a type\nEND_TYPE;\n\nRULE r FOR (part);\nWHERE\n  wr1: -- of a rule\n    TRUE;\nEND_RULE;\n\nEND_SCHEMA;\n";

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
fn each_comment_is_written_after_the_code_it_follows() {
    let text = expressir::formatter::format(&attached(SOURCE), false);
    assert_eq!(text, EXPECTED);
}

#[test]
fn the_same_text_again_from_its_own_output() {
    let once = expressir::formatter::format(&attached(SOURCE), false);
    assert_eq!(expressir::formatter::format(&attached(&once), false), once);
}

#[test]
fn the_remarks_are_placed_on_the_item_and_on_the_rule() {
    let wire = attached(SOURCE);
    let placement = |ptr: &str| {
        wire.pointer(ptr)
            .and_then(|r| r.get("placement"))
            .and_then(serde_json::Value::as_str)
            .map(String::from)
    };
    assert_eq!(
        placement("/schemas/0/types/0/underlying_type/items/0/untagged_remarks/0").as_deref(),
        Some("inline")
    );
    assert_eq!(
        placement("/schemas/0/entities/0/where_rules/0/untagged_remarks/0").as_deref(),
        Some("inline")
    );
    let region = wire
        .pointer("/schemas/0/entities/0/where_rules/1/untagged_remarks/0/region")
        .and_then(serde_json::Value::as_str);
    assert_eq!(region, Some("opener"));
}

#[test]
fn the_stripped_form_writes_none_of_them() {
    let text = expressir::formatter::format(&attached(SOURCE), true);
    assert!(!text.contains("--"), "{text}");
}

// Two `--` remarks never share a line: the second would become part of
// the first.
#[test]
fn two_comments_of_a_rule_without_a_label_stay_apart() {
    let source = "SCHEMA two;\nENTITY part;\n  size : INTEGER;\nWHERE\n  (size > 0) -- first\n    AND (size < 5); -- second\nEND_ENTITY;\nEND_SCHEMA;\n";
    let text = expressir::formatter::format(&attached(source), false);
    assert!(
        text.contains("WHERE\n  -- first\n  (size > 0) AND (size < 5); -- second\nEND_ENTITY;"),
        "{text}"
    );
}
