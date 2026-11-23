//! A comment inside the item list of a SELECT type is written back where
//! it stands (lutaml/expressir#741): after the item it follows on its
//! line, or on a line of its own before the item it precedes. The wire
//! held these remarks on the TYPE, unplaced, and they were written after
//! `END_TYPE;`. The items are handled as the items of an ENUMERATION are
//! (lutaml/expressir#724).

const SOURCE: &str = "SCHEMA select_probe;\nTYPE first_select = SELECT (a, -- after the first item\n   b);\nEND_TYPE;\nTYPE second_select = SELECT (\n  a, -- after an item\n  -- on its own line between two items\n  b, -- after the second item\n  c); -- after the list\nEND_TYPE;\nTYPE third_select = EXTENSIBLE GENERIC_ENTITY SELECT\n  (-- after the parenthesis\n   a,\n   b);\nEND_TYPE;\nTYPE fourth_select = SELECT BASED_ON third_select WITH (\n  -- on its own line before the first item\n  c);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n";

const EXPECTED: &str = "SCHEMA select_probe;\n\nTYPE first_select = SELECT\n  (a, -- after the first item\n   b);\nEND_TYPE;\n\nTYPE second_select = SELECT\n  (a, -- after an item\n   -- on its own line between two items\n   b, -- after the second item\n   c); -- after the list\nEND_TYPE;\n\nTYPE third_select = EXTENSIBLE GENERIC_ENTITY SELECT\n  (-- after the parenthesis\n   a,\n   b);\nEND_TYPE;\n\nTYPE fourth_select = SELECT BASED_ON third_select WITH\n  (-- on its own line before the first item\n   c);\nEND_TYPE;\n\nENTITY a;\nEND_ENTITY;\n\nENTITY b;\nEND_ENTITY;\n\nENTITY c;\nEND_ENTITY;\n\nEND_SCHEMA;\n";

fn attached(source: &str) -> serde_json::Value {
    let mut wire = expressir::ParsedTree::parse(source)
        .expect("parses")
        .to_model_json("s.exp")
        .expect("wire");
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);
    wire
}

fn formatted(source: &str) -> String {
    expressir::formatter::format(&attached(source), false)
}

#[test]
fn each_comment_is_written_where_it_stands_in_the_list() {
    assert_eq!(formatted(SOURCE), EXPECTED);
}

#[test]
fn the_same_text_again_from_its_own_output() {
    let once = formatted(SOURCE);
    assert_eq!(formatted(&once), once);
}

#[test]
fn the_remarks_are_held_by_the_items() {
    let wire = attached(SOURCE);
    let field = |ptr: &str, name: &str| {
        wire.pointer(ptr)
            .and_then(|r| r.get(name))
            .and_then(serde_json::Value::as_str)
            .map(String::from)
    };
    let items = "/schemas/0/types/1/underlying_type/items";
    assert_eq!(
        field(&format!("{items}/0/untagged_remarks/0"), "placement").as_deref(),
        Some("inline")
    );
    assert_eq!(
        field(&format!("{items}/1/untagged_remarks/0"), "placement").as_deref(),
        Some("leading")
    );
    assert_eq!(
        field(&format!("{items}/1/untagged_remarks/1"), "placement").as_deref(),
        Some("inline")
    );
    assert_eq!(
        field(&format!("{items}/2/untagged_remarks/0"), "text").as_deref(),
        Some("after the list")
    );
    assert!(wire
        .pointer("/schemas/0/types/1/untagged_remarks")
        .and_then(serde_json::Value::as_array)
        .is_none_or(Vec::is_empty));
}

#[test]
fn the_stripped_form_writes_none_of_them() {
    let text = expressir::formatter::format(&attached(SOURCE), true);
    assert!(!text.contains("--"), "{text}");
}

#[test]
fn a_comment_after_the_last_item_is_written_after_the_semicolon() {
    // As for the last item of an ENUMERATION: before the `)` a tail
    // remark would take the `)` and the `;` with it.
    let text = formatted("SCHEMA s;\nTYPE t = SELECT\n  (a,\n   b -- after the last item\n  );\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   b); -- after the last item
END_TYPE;"
        ),
        "{text}"
    );
}

#[test]
fn a_comment_after_a_select_on_one_line_stays_on_the_line_of_the_semicolon() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT (a, b); -- after the select\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   b); -- after the select
END_TYPE;"
        ),
        "{text}"
    );
}

#[test]
fn a_comment_after_two_items_on_a_line_follows_the_second() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT\n  (a, b, -- after two items\n   c);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   b, -- after two items
   c);"
        ),
        "{text}"
    );
}

#[test]
fn a_block_comment_after_an_item_is_written_before_the_next_item() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT\n  (a, (* about b *)\n   b);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   (* about b *)
   b);"
        ),
        "{text}"
    );
}

#[test]
fn a_comment_of_a_where_rule_of_the_type_stays_with_the_rule() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT\n  (a, -- first\n   b);\nWHERE\n  wr1 : TRUE; -- of the rule\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a, -- first
   b);
WHERE
  wr1: TRUE; -- of the rule
END_TYPE;"
        ),
        "{text}"
    );
}

// Not changed: a comment between `SELECT` and the parenthesis, one on a
// line of its own after the last item, and the head of a RULE, whose
// `FOR` items are references too.
#[test]
fn a_comment_before_the_parenthesis_is_written_as_before() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT -- before the parenthesis\n  (a,\n   b);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   b);
END_TYPE; -- before the parenthesis"
        ),
        "{text}"
    );
}

#[test]
fn a_comment_on_its_own_line_after_the_last_item_is_written_as_before() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT\n  (a,\n   b\n   -- after the last item\n  );\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   b);
END_TYPE; -- after the last item"
        ),
        "{text}"
    );
}

#[test]
fn a_comment_on_the_head_of_a_rule_is_written_as_before() {
    let text = formatted("SCHEMA s;\nRULE r FOR (a, b); -- on the rule head\nWHERE\n  wr1 : TRUE;\nEND_RULE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "RULE r FOR (a, b);
  -- on the rule head
WHERE"
        ),
        "{text}"
    );
}

// A remark on a line of its own before an item is written at the depth
// of the items, in an ENUMERATION as in a SELECT.
#[test]
fn an_item_after_a_comment_keeps_its_indentation() {
    let text = formatted("SCHEMA s;\nTYPE e = ENUMERATION OF\n  (p,\n   -- before q\n   q);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (p,
   -- before q
   q);"
        ),
        "{text}"
    );
}

// Seen in review. The lines inside a remark of several lines are left as
// the formatter writes them; the remark and the item after it start at
// the depth of the items.
#[test]
fn a_remark_of_several_lines_keeps_its_inner_lines() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT\n  (a,\n   (* c\n   d\n   e *)\n   b);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   (*
  c
     d
     e
  *)
   b);"
        ),
        "{text}"
    );
    let text = formatted("SCHEMA s;\nTYPE e = ENUMERATION OF\n  (p,\n   (* c\n   d\n   e *)\n   q);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (p,
   (*
  c
     d
     e
  *)
   q);"
        ),
        "{text}"
    );
}

// Seen in review. Not changed: a block comment with code after it on its
// line (it is not written, as before), and a comment on a line of its own
// between `SELECT` and the parenthesis.
#[test]
fn a_block_comment_with_code_after_it_is_left_as_before() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT\n  (a, (* x *) b,\n   c);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   b,
   c);
END_TYPE;
"
        ),
        "{text}"
    );
    assert!(!text.contains("(* x *)"), "{text}");
}

#[test]
fn a_comment_on_its_own_line_before_the_parenthesis_is_written_as_before() {
    let text = formatted("SCHEMA s;\nTYPE t = SELECT\n  -- before the parenthesis\n  (a,\n   b);\nEND_TYPE;\nENTITY a;\nEND_ENTITY;\nENTITY b;\nEND_ENTITY;\nENTITY c;\nEND_ENTITY;\nEND_SCHEMA;\n");
    assert!(
        text.contains(
            "  (a,
   b);
END_TYPE; -- before the parenthesis"
        ),
        "{text}"
    );
}
