//! A comment on a line of its own before `DERIVE`, `INVERSE`, `UNIQUE` or
//! `WHERE` of an ENTITY (or the `WHERE` of a TYPE) is written back before
//! that keyword. The attacher held such a remark on the next member of
//! the section, and the formatter wrote it after the keyword.

const SOURCE: &str = "SCHEMA sections;\nENTITY part;\n  size : INTEGER;\n  -- before DERIVE\nDERIVE\n  twice : INTEGER := size * 2;\n  -- before INVERSE\nINVERSE\n  used : SET OF holder FOR held;\n  -- before UNIQUE\nUNIQUE\n  ur1: size;\n  -- before WHERE\nWHERE\n  wr1: size > 0;\n  -- before the end\nEND_ENTITY;\nENTITY holder;\n  held : part;\nEND_ENTITY;\nTYPE positive = REAL;\n  -- before WHERE of a type\nWHERE\n  wr1: SELF > 0;\n  -- before the end of the type\nEND_TYPE;\nEND_SCHEMA;\n";

fn attached(source: &str) -> serde_json::Value {
    let mut wire = expressir::ParsedTree::parse(source)
        .expect("parses")
        .to_model_json("s.exp")
        .expect("wire");
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);
    wire
}

fn format(source: &str) -> String {
    expressir::formatter::format(&attached(source), false)
}

/// The text the formatter writes for a source already in its style: the
/// source itself, with blank lines between the declarations.
fn expected_of(source: &str) -> String {
    let mut out = String::new();
    for line in source.lines() {
        if (line.starts_with("ENTITY ") || line.starts_with("TYPE ")) && !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    out.replace("END_SCHEMA;", "\nEND_SCHEMA;")
}

#[test]
fn each_comment_is_written_before_the_keyword_it_precedes() {
    assert_eq!(format(SOURCE), expected_of(SOURCE));
}

#[test]
fn the_same_text_again_from_its_own_output() {
    let once = format(SOURCE);
    assert_eq!(format(&once), once);
}

fn placed(wire: &serde_json::Value, ptr: &str) -> (Option<String>, Option<String>) {
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
}

fn trailing(region: &str) -> (Option<String>, Option<String>) {
    (Some("trailing".to_string()), Some(region.to_string()))
}

#[test]
fn each_remark_closes_the_section_before_its_keyword() {
    let wire = attached(SOURCE);
    let entity = "/schemas/0/entities/0/untagged_remarks";
    assert_eq!(
        placed(&wire, &format!("{entity}/0")),
        trailing("attributes")
    );
    assert_eq!(
        placed(&wire, &format!("{entity}/1")),
        trailing("derived_attributes")
    );
    assert_eq!(
        placed(&wire, &format!("{entity}/2")),
        trailing("inverse_attributes")
    );
    assert_eq!(
        placed(&wire, &format!("{entity}/3")),
        trailing("unique_rules")
    );
    assert_eq!(
        placed(&wire, &format!("{entity}/4")),
        trailing("where_rules")
    );
    let ty = "/schemas/0/types/0/untagged_remarks";
    assert_eq!(
        placed(&wire, &format!("{ty}/0")),
        trailing("underlying_type")
    );
    assert_eq!(placed(&wire, &format!("{ty}/1")), trailing("where_rules"));
}

#[test]
fn the_stripped_form_writes_none_of_them() {
    let text = expressir::formatter::format(&attached(SOURCE), true);
    assert!(!text.contains("--") && !text.contains("(*"), "{text}");
}

/// Every subset of the four sections, a comment before each keyword that
/// exists and before END_ENTITY.
#[test]
fn every_combination_of_sections_keeps_each_comment_before_its_keyword() {
    for mask in 0..16u32 {
        let has = |bit: u32| mask & (1 << bit) != 0;
        let mut body = String::from("ENTITY e;\n  a : INTEGER;\n");
        let mut n = 0;
        let mut note = |body: &mut String| {
            body.push_str(&format!("  -- c{n}\n"));
            n += 1;
        };
        if has(0) {
            note(&mut body);
            body.push_str("DERIVE\n  d : INTEGER := a;\n");
        }
        if has(1) {
            note(&mut body);
            body.push_str("INVERSE\n  i : SET OF h FOR hh;\n");
        }
        if has(2) {
            note(&mut body);
            body.push_str("UNIQUE\n  ur1: a;\n");
        }
        if has(3) {
            note(&mut body);
            body.push_str("WHERE\n  wr1: a > 0;\n");
        }
        note(&mut body);
        body.push_str("END_ENTITY;\nENTITY h;\n  hh : e;\nEND_ENTITY;\n");
        let source = format!("SCHEMA s;\n{body}END_SCHEMA;\n");
        let once = format(&source);
        assert_eq!(once, expected_of(&source), "mask {mask}");
        assert_eq!(format(&once), once, "second pass, mask {mask}");
    }
}

#[test]
fn two_comments_in_a_row_and_a_block_comment() {
    let source = "SCHEMA s;\nENTITY e;\n  a : INTEGER;\n  -- one\n  (* two *)\n  -- three\nWHERE\n  wr1: a > 0;\nEND_ENTITY;\nEND_SCHEMA;\n";
    assert_eq!(format(source), expected_of(source));
}

#[test]
fn a_comment_that_names_a_keyword_and_attributes_that_look_like_one() {
    let source = "SCHEMA s;\nENTITY e;\n  where_x : INTEGER;\n  derive_y : INTEGER;\n  -- the WHERE rules and the DERIVE section follow\nDERIVE\n  d : INTEGER := where_x;\n  -- unique and inverse are not here\nWHERE\n  wr1: where_x > 0;\nEND_ENTITY;\nEND_SCHEMA;\n";
    assert_eq!(format(source), expected_of(source));
}

#[test]
fn lower_case_keywords_are_written_in_the_formatters_case() {
    let source = "SCHEMA s;\nENTITY e;\n  a : INTEGER;\n  -- c0\nderive\n  d : INTEGER := a;\n  -- c1\nunique\n  ur1: a;\n  -- c2\nwhere\n  wr1: a > 0;\nEND_ENTITY;\nEND_SCHEMA;\n";
    let upper = source
        .replace("derive", "DERIVE")
        .replace("unique", "UNIQUE")
        .replace("where\n", "WHERE\n");
    assert_eq!(format(source), expected_of(&upper));
}

#[test]
fn an_entity_declared_in_a_function() {
    let source = "SCHEMA s;\nFUNCTION f : INTEGER;\n  ENTITY e;\n    a : INTEGER;\n    -- c0\n  WHERE\n    wr1: a > 0;\n  END_ENTITY;\n  RETURN (1);\nEND_FUNCTION;\nEND_SCHEMA;\n";
    let text = format(source);
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle} in {text}"))
    };
    assert!(at("-- c0") < at("WHERE"), "{text}");
    assert!(at("a : INTEGER;") < at("-- c0"), "{text}");
    assert_eq!(format(&text), text);
}

#[test]
fn the_where_of_a_rule_after_an_entity_is_unchanged() {
    let source = "SCHEMA s;\nENTITY e;\n  a : INTEGER;\nEND_ENTITY;\nRULE r FOR (e);\n  LOCAL\n    x : INTEGER := 1;\n  END_LOCAL;\n  -- before the WHERE of the rule\nWHERE\n  wr1: TRUE;\nEND_RULE;\nEND_SCHEMA;\n";
    let wire = attached(source);
    assert_eq!(
        placed(&wire, "/schemas/0/rules/0/untagged_remarks/0"),
        trailing("statements")
    );
    assert!(wire
        .pointer("/schemas/0/entities/0/untagged_remarks")
        .is_none());
    let text = expressir::formatter::format(&wire, false);
    assert!(
        text.contains("END_LOCAL;\n  -- before the WHERE of the rule\nWHERE\n"),
        "{text}"
    );
}

#[test]
fn a_comment_at_the_end_of_a_code_line_stays_with_its_holder() {
    let source = "SCHEMA s;\nENTITY e;\n  a : INTEGER; -- of a\nDERIVE\n  d : INTEGER := a; -- of d\nWHERE\n  wr1: a > 0;\nEND_ENTITY;\nEND_SCHEMA;\n";
    let wire = attached(source);
    assert!(wire
        .pointer("/schemas/0/entities/0/untagged_remarks")
        .is_none());
    assert_eq!(format(source), expected_of(source));
}
