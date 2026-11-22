//! A comment on a line of its own before `END_ENTITY` or `END_TYPE` is
//! written back before that keyword (lutaml/expressir#ISSUE). The
//! closing-keyword table had no entry for the two keywords, so the wire
//! held such a remark on the declaration unplaced and the formatter wrote
//! it after the keyword.

const SOURCE: &str = "SCHEMA before_end;\nENTITY attrs;\n  a : INTEGER;\n  b : INTEGER;\n  -- after the last attribute\nEND_ENTITY;\nENTITY two;\n  a : INTEGER;\n  -- first of two\n  -- second of two\nEND_ENTITY;\nENTITY derived;\n  a : INTEGER;\nDERIVE\n  d : INTEGER := a + 1;\n  -- after the last derived attribute\nEND_ENTITY;\nENTITY uniq;\n  a : INTEGER;\nUNIQUE\n  ur1 : a;\n  -- after the last unique rule\nEND_ENTITY;\nENTITY rules;\n  a : INTEGER;\nWHERE\n  wr1 : a > 0;\n  -- after the last rule\nEND_ENTITY;\nENTITY block;\n  a : INTEGER;\n  (* a block comment *)\nEND_ENTITY;\nENTITY empty;\n  -- nothing declared yet\nEND_ENTITY;\nTYPE colour = ENUMERATION OF\n  (red,\n   green);\n  -- after the list of items\nEND_TYPE;\nTYPE positive = REAL;\nWHERE\n  wr1 : SELF > 0;\n  -- after the last rule of a type\nEND_TYPE;\nEND_SCHEMA;\n";

const EXPECTED: &str = "SCHEMA before_end;\n\nENTITY attrs;\n  a : INTEGER;\n  b : INTEGER;\n  -- after the last attribute\nEND_ENTITY;\n\nENTITY two;\n  a : INTEGER;\n  -- first of two\n  -- second of two\nEND_ENTITY;\n\nENTITY derived;\n  a : INTEGER;\nDERIVE\n  d : INTEGER := a + 1;\n  -- after the last derived attribute\nEND_ENTITY;\n\nENTITY uniq;\n  a : INTEGER;\nUNIQUE\n  ur1: a;\n  -- after the last unique rule\nEND_ENTITY;\n\nENTITY rules;\n  a : INTEGER;\nWHERE\n  wr1: a > 0;\n  -- after the last rule\nEND_ENTITY;\n\nENTITY block;\n  a : INTEGER;\n  (* a block comment *)\nEND_ENTITY;\n\nENTITY empty;\n  -- nothing declared yet\nEND_ENTITY;\n\nTYPE colour = ENUMERATION OF\n  (red,\n   green);\n  -- after the list of items\nEND_TYPE;\n\nTYPE positive = REAL;\nWHERE\n  wr1: SELF > 0;\n  -- after the last rule of a type\nEND_TYPE;\n\nEND_SCHEMA;\n";

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
fn each_comment_is_written_before_the_keyword_it_precedes() {
    let text = expressir::formatter::format(&attached(SOURCE), false);
    assert_eq!(text, EXPECTED);
}

#[test]
fn the_same_text_again_from_its_own_output() {
    let once = expressir::formatter::format(&attached(SOURCE), false);
    assert_eq!(expressir::formatter::format(&attached(&once), false), once);
}

#[test]
fn the_remarks_close_the_last_body_of_the_declaration() {
    let wire = attached(SOURCE);
    let placed = |ptr: &str| {
        let remark = wire
            .pointer(ptr)
            .unwrap_or_else(|| panic!("no remark at {ptr}"));
        let field = |name: &str| {
            remark
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(String::from)
        };
        (field("placement"), field("region"))
    };
    let trailing = |region: &str| (Some("trailing".to_string()), Some(region.to_string()));
    // An entity without WHERE rules: the remark closes its attributes.
    assert_eq!(
        placed("/schemas/0/entities/0/untagged_remarks/0"),
        trailing("attributes")
    );
    assert_eq!(
        placed("/schemas/0/entities/1/untagged_remarks/1"),
        trailing("attributes")
    );
    assert_eq!(
        placed("/schemas/0/entities/4/untagged_remarks/0"),
        trailing("where_rules")
    );
    // A type without WHERE rules: the remark closes its definition.
    assert_eq!(
        placed("/schemas/0/types/0/untagged_remarks/0"),
        trailing("underlying_type")
    );
    assert_eq!(
        placed("/schemas/0/types/1/untagged_remarks/0"),
        trailing("where_rules")
    );
}

#[test]
fn a_comment_at_the_end_of_a_code_line_is_not_taken() {
    // The last attribute's own remark and the remark on the END_ line
    // keep the holders they had.
    let source = "SCHEMA s;\nENTITY e;\n  a : INTEGER; -- of the attribute\nEND_ENTITY; -- of the entity\nEND_SCHEMA;\n";
    let wire = attached(source);
    let text = expressir::formatter::format(&wire, false);
    assert_eq!(
        text,
        "SCHEMA s;\n\nENTITY e;\n  a : INTEGER; -- of the attribute\nEND_ENTITY; -- of the entity\n\nEND_SCHEMA;\n"
    );
}

#[test]
fn a_comment_at_the_end_of_the_last_rule_line_stays_with_the_rule() {
    let source = "SCHEMA s;\nENTITY e;\n  a : INTEGER;\nWHERE\n  wr1 : a > 0; -- of the rule\nEND_ENTITY;\nEND_SCHEMA;\n";
    let wire = attached(source);
    let text_at = |ptr: &str| {
        wire.pointer(ptr)
            .and_then(|r| r.get("text"))
            .and_then(serde_json::Value::as_str)
            .map(String::from)
    };
    assert_eq!(
        text_at("/schemas/0/entities/0/where_rules/0/untagged_remarks/0").as_deref(),
        Some("of the rule")
    );
    assert!(wire
        .pointer("/schemas/0/entities/0/untagged_remarks")
        .is_none());
}

#[test]
fn a_block_comment_at_the_end_of_the_last_rule_line_is_not_taken() {
    let source = "SCHEMA s;\nENTITY e;\n  a : INTEGER;\nWHERE\n  wr1 : a > 0; (* of the rule *)\nEND_ENTITY;\nEND_SCHEMA;\n";
    let wire = attached(source);
    let trailing = wire
        .pointer("/schemas/0/entities/0/untagged_remarks")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|remarks| {
            remarks
                .iter()
                .any(|r| r.get("placement").and_then(serde_json::Value::as_str) == Some("trailing"))
        });
    assert!(!trailing);
}

#[test]
fn the_stripped_form_writes_none_of_them() {
    let text = expressir::formatter::format(&attached(SOURCE), true);
    assert!(!text.contains("--") && !text.contains("(*"), "{text}");
}
