//! A `(* block *)` comment right after the label of a WHERE rule
//! (`wr1 : (* why *) size > 0;`) is written back after the label, with the
//! expression on the next line, as the `--` comment of the same place is
//! (lutaml/expressir#724). The attacher read "the comment follows code on
//! its line" from the text `--` alone, so a block comment was held by no
//! node that writes it.

const PROBE: &str = "SCHEMA label_probe;\nENTITY part;\n  size : INTEGER;\nWHERE\n  wr1 : (* H1 a block after the label *) size > 0;\n  wr2 : (* H2 a block of two lines\n           after the label *)\n        size < 100;\n  wr3 : size <> 50 (* H3 a block before the semicolon *);\n  wr4 : -- H4 a line comment after the label\n        size <> 60;\nEND_ENTITY;\nTYPE positive = INTEGER;\nWHERE\n  wr1 : (* H5 a block after the label of a type rule *) SELF > 0;\nEND_TYPE;\nRULE r FOR (part);\nWHERE\n  wr1 : (* H6 a block after the label of a rule *) TRUE;\nEND_RULE;\nEND_SCHEMA;\n";

// H3, a block between the expression and the `;`, is not written (it is not
// part of this change; its text is pinned here as it is).
const PROBE_EXPECTED: &str = "SCHEMA label_probe;\n\nENTITY part;\n  size : INTEGER;\nWHERE\n  wr1: (* H1 a block after the label *)\n    size > 0;\n  wr2: (*\n  H2 a block of two lines\n             after the label\n  *)\n    size < 100;\n  wr3: size <> 50;\n  wr4: -- H4 a line comment after the label\n    size <> 60;\nEND_ENTITY;\n\nTYPE positive = INTEGER;\nWHERE\n  wr1: (* H5 a block after the label of a type rule *)\n    SELF > 0;\nEND_TYPE;\n\nRULE r FOR (part);\nWHERE\n  wr1: (* H6 a block after the label of a rule *)\n    TRUE;\nEND_RULE;\n\nEND_SCHEMA;\n";

// The same positions on one line each, for the second pass.
const ONE_LINE: &str = "SCHEMA one;\nENTITY part;\n  size : INTEGER;\nWHERE\n  wr1 : (* H1 *) size > 0;\n  wr4 : -- H4\n        size <> 60;\nEND_ENTITY;\nTYPE positive = INTEGER;\nWHERE\n  wr1 : (* H5 *) SELF > 0;\nEND_TYPE;\nRULE r FOR (part);\nWHERE\n  wr1 : (* H6 *) TRUE;\nEND_RULE;\nEND_SCHEMA;\n";

const MORE: &str = "SCHEMA more_probe;\nENTITY part;\n  size : INTEGER;\nWHERE\n  wr1 : (* a1 *) (* a2 *) size > 0;\n  wr2 : (* b1 *) -- b2\n        size > 1;\n  wr3 : (* c1 : ; -- (* nested *) tail *) size > 2;\n  wr6 : (* e1 *) size > 6; -- e2\nEND_ENTITY;\nEND_SCHEMA;\n";

const MORE_EXPECTED: &str = "SCHEMA more_probe;\n\nENTITY part;\n  size : INTEGER;\nWHERE\n  wr1: (* a1 *) (* a2 *)\n    size > 0;\n  wr2: (* b1 *) -- b2\n    size > 1;\n  wr3: (* c1 : ; -- (* nested *) tail *)\n    size > 2;\n  wr6: (* e1 *)\n    size > 6; -- e2\nEND_ENTITY;\n\nEND_SCHEMA;\n";

// Not part of this change, written as before: a DERIVE line, a block on
// the line after the label, or in the middle of an expression.
const PINNED: &str = "SCHEMA pinned_probe;\nENTITY part;\n  size : INTEGER;\n  ur : INTEGER;\nDERIVE\n  d : INTEGER := (* d1 *) 1;\nUNIQUE\n  ur1 : size;\nWHERE\n  wr4 :\n    (* n1 next line *)\n    size > 3;\n  wr5 : (size > 4) (* m1 *) AND (size < 9);\nEND_ENTITY;\nTYPE unl = INTEGER;\nWHERE\n  SELF > 5;\nEND_TYPE;\nEND_SCHEMA;\n";

const PINNED_EXPECTED: &str = "SCHEMA pinned_probe;\n\nENTITY part;\n  size : INTEGER;\n  ur : INTEGER;\nDERIVE\n  d : INTEGER := 1; (* d1 *)\nUNIQUE\n  ur1: size;\nWHERE\n  wr4: size > 3;\n  wr5: (size > 4) AND (size < 9);\nEND_ENTITY;\n\nTYPE unl = INTEGER;\nWHERE\n  SELF > 5;\nEND_TYPE;\n\nEND_SCHEMA;\n";

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

#[test]
fn each_block_after_a_label_is_written_after_the_label() {
    assert_eq!(format(PROBE), PROBE_EXPECTED);
}

#[test]
fn the_same_text_again_from_its_own_output() {
    let once = format(ONE_LINE);
    assert_eq!(format(&once), once);
    assert!(once.contains("wr1: (* H1 *)\n    size > 0;"), "{once}");
    assert!(once.contains("wr1: (* H6 *)\n    TRUE;"), "{once}");
}

#[test]
fn the_stripped_form_writes_none_of_them() {
    let text = expressir::formatter::format(&attached(PROBE), true);
    assert!(!text.contains("(*") && !text.contains("--"), "{text}");
}

#[test]
fn the_block_is_held_by_the_rule_inline_with_the_opener_region() {
    let wire = attached(PROBE);
    let field = |ptr: &str, name: &str| {
        wire.pointer(ptr)
            .and_then(|r| r.get(name))
            .and_then(serde_json::Value::as_str)
            .map(String::from)
    };
    for rule in [
        "/schemas/0/entities/0/where_rules/0",
        "/schemas/0/entities/0/where_rules/1",
        "/schemas/0/types/0/where_rules/0",
        "/schemas/0/rules/0/where_rules/0",
    ] {
        let remark = format!("{rule}/untagged_remarks/0");
        assert_eq!(
            field(&remark, "placement").as_deref(),
            Some("inline"),
            "{rule}"
        );
        assert_eq!(
            field(&remark, "region").as_deref(),
            Some("opener"),
            "{rule}"
        );
    }
    // The `--` comment of H4 is held the same way.
    let h4 = "/schemas/0/entities/0/where_rules/3/untagged_remarks/0";
    assert_eq!(field(h4, "region").as_deref(), Some("opener"));
}

#[test]
fn several_blocks_a_block_and_a_line_comment_and_odd_text_stay_after_the_label() {
    assert_eq!(format(MORE), MORE_EXPECTED);
}

#[test]
fn the_neighbouring_places_write_what_they_wrote_before() {
    assert_eq!(format(PINNED), PINNED_EXPECTED);
}

// Offsets are bytes: text with accents before the comment does not move it.
#[test]
fn text_with_accents_before_the_comment_does_not_move_it() {
    let source = "SCHEMA u;\n(* é ü schéma *)\nENTITY part;\n  size : INTEGER;\nWHERE\n  wr1 : (* é why ü *) size > 0;\nEND_ENTITY;\nEND_SCHEMA;\n";
    let text = format(source);
    assert!(text.contains("wr1: (* é why ü *)\n    size > 0;"), "{text}");
}
