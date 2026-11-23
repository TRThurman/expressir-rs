//! A comment on a line of its own in the declaration part of a FUNCTION,
//! PROCEDURE or RULE (before LOCAL, inside the LOCAL block, before a nested
//! declaration) is written back where it stood. The statement-region lookup
//! made it the leading remark of the first statement that follows, and every
//! such comment was written after all the declarations.

use serde_json::Value;

const PROBE: &str = "SCHEMA function_probe;\nFUNCTION f1 (x : INTEGER) : INTEGER;\n  -- F01 on its own line before LOCAL\n  LOCAL\n    -- F02 on its own line before the first variable\n    y : INTEGER;\n    -- F03 on its own line before END_LOCAL\n  END_LOCAL;\n  -- F04 on its own line before the first statement\n  y := x;\n  RETURN (y);\nEND_FUNCTION;\nFUNCTION f2 (x : INTEGER) : INTEGER;\n  -- F05 on its own line before a nested function\n  FUNCTION g (z : INTEGER) : INTEGER;\n    RETURN (z);\n  END_FUNCTION;\n  -- F06 on its own line before a nested procedure\n  PROCEDURE p (z : INTEGER);\n  END_PROCEDURE;\n  -- F07 on its own line before a constant block\n  CONSTANT\n    k : INTEGER := 1;\n  END_CONSTANT;\n  RETURN (g(x) + k);\nEND_FUNCTION;\nPROCEDURE p1 (x : INTEGER);\n  -- F08 on its own line before LOCAL of a procedure\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  y := x;\nEND_PROCEDURE;\nRULE r1 FOR (e);\n  -- F09 on its own line before LOCAL of a rule\n  LOCAL\n    y : INTEGER := 0;\n  END_LOCAL;\nWHERE\n  wr1 : y = 0;\nEND_RULE;\nENTITY e;\nEND_ENTITY;\nEND_SCHEMA;\n";

const EXPECTED: &str = "SCHEMA function_probe;\n\nFUNCTION f1(x : INTEGER) : INTEGER;\n  -- F01 on its own line before LOCAL\n  LOCAL\n    -- F02 on its own line before the first variable\n    y : INTEGER;\n    -- F03 on its own line before END_LOCAL\n  END_LOCAL;\n  -- F04 on its own line before the first statement\n  y := x;\n  RETURN (y);\nEND_FUNCTION;\n\nFUNCTION f2(x : INTEGER) : INTEGER;\n  -- F05 on its own line before a nested function\n  FUNCTION g(z : INTEGER) : INTEGER;\n    RETURN (z);\n  END_FUNCTION;\n  -- F06 on its own line before a nested procedure\n  PROCEDURE p(z : INTEGER);\n  END_PROCEDURE;\n  CONSTANT\n    -- F07 on its own line before a constant block\n    k : INTEGER := 1;\n  END_CONSTANT;\n  RETURN (g(x) + k);\nEND_FUNCTION;\n\nPROCEDURE p1(x : INTEGER);\n  -- F08 on its own line before LOCAL of a procedure\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  y := x;\nEND_PROCEDURE;\n\nRULE r1 FOR (e);\n  -- F09 on its own line before LOCAL of a rule\n  LOCAL\n    y : INTEGER := 0;\n  END_LOCAL;\nWHERE\n  wr1: y = 0;\nEND_RULE;\n\nENTITY e;\nEND_ENTITY;\n\nEND_SCHEMA;\n";

fn attached(source: &str) -> Value {
    let mut wire = expressir::ParsedTree::parse(source)
        .expect("parses")
        .to_model_json("s.exp")
        .expect("wire");
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);
    wire
}

fn in_schema(body: &str) -> String {
    format!("SCHEMA head_probe;\n{body}ENTITY e;\nEND_ENTITY;\nEND_SCHEMA;\n")
}

fn format_text(source: &str) -> String {
    expressir::formatter::format(&attached(source), false)
}

/// (text word, placement, region) of every remark at `ptr`.
fn held(wire: &Value, ptr: &str) -> Vec<(String, String, String)> {
    let text = |r: &Value, k: &str| {
        r.get(k)
            .and_then(Value::as_str)
            .unwrap_or("-")
            .split_whitespace()
            .next()
            .unwrap_or("-")
            .to_string()
    };
    wire.pointer(&format!("{ptr}/untagged_remarks"))
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .map(|r| (text(r, "text"), text(r, "placement"), text(r, "region")))
                .collect()
        })
        .unwrap_or_default()
}

fn t(a: &str, b: &str, c: &str) -> (String, String, String) {
    (a.to_string(), b.to_string(), c.to_string())
}

#[test]
fn the_nine_comments_of_the_probe_are_written_where_they_stood() {
    assert_eq!(format_text(PROBE), EXPECTED);
}

#[test]
fn the_same_text_again_from_its_own_output() {
    let once = format_text(PROBE);
    assert_eq!(format_text(&once), once);
}

#[test]
fn the_stripped_form_writes_none_of_them() {
    let text = expressir::formatter::format(&attached(PROBE), true);
    assert!(!text.contains("--") && !text.contains("(*"), "{text}");
    assert!(text.contains(
        "FUNCTION f1(x : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n"
    ));
}

#[test]
fn each_comment_is_held_where_it_stood() {
    let wire = attached(PROBE);
    assert_eq!(
        held(&wire, "/schemas/0/functions/0"),
        vec![
            t("F01", "trailing", "declarations"),
            t("F03", "trailing", "variables")
        ]
    );
    assert_eq!(
        held(&wire, "/schemas/0/functions/0/variables/0"),
        vec![t("F02", "leading", "-")]
    );
    assert_eq!(
        held(&wire, "/schemas/0/functions/0/statements/0"),
        vec![t("F04", "leading", "-")]
    );
    assert_eq!(
        held(&wire, "/schemas/0/functions/1/functions/0"),
        vec![t("F05", "leading", "-")]
    );
    assert_eq!(
        held(&wire, "/schemas/0/functions/1/procedures/0"),
        vec![t("F06", "leading", "-")]
    );
    assert_eq!(
        held(&wire, "/schemas/0/functions/1/constants/0"),
        vec![t("F07", "leading", "-")]
    );
    assert_eq!(
        held(&wire, "/schemas/0/procedures/0"),
        vec![t("F08", "trailing", "declarations")]
    );
    assert_eq!(
        held(&wire, "/schemas/0/rules/0"),
        vec![t("F09", "trailing", "declarations")]
    );
    assert!(held(&wire, "/schemas/0/rules/0/where_rules/0").is_empty());
}

#[test]
fn a_procedure_with_no_statement_after_the_block() {
    let source = in_schema("PROCEDURE p(x : INTEGER);\n  -- before LOCAL\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\nEND_PROCEDURE;\n");
    let once = format_text(&source);
    assert!(once.contains(
        "PROCEDURE p(x : INTEGER);\n  -- before LOCAL\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\nEND_PROCEDURE;"
    ));
    assert_eq!(format_text(&once), once);
}

#[test]
fn a_nested_function_with_its_own_comment_before_local() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER;\n  FUNCTION g(z : INTEGER) : INTEGER;\n    -- inside g, before LOCAL\n    LOCAL\n      w : INTEGER;\n    END_LOCAL;\n    RETURN (z);\n  END_FUNCTION;\n  -- inside f, before LOCAL\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  RETURN (g(x));\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(once.contains(
        "FUNCTION f(x : INTEGER) : INTEGER;\n  FUNCTION g(z : INTEGER) : INTEGER;\n    -- inside g, before LOCAL\n    LOCAL\n      w : INTEGER;\n    END_LOCAL;\n    RETURN (z);\n  END_FUNCTION;\n  -- inside f, before LOCAL\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  RETURN (g(x));\nEND_FUNCTION;"
    ), "{once}");
    assert_eq!(format_text(&once), once);
}

#[test]
fn two_comments_in_a_row_keep_their_order() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER;\n  -- first\n  -- second\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  RETURN (x);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(
        once.contains("  -- first\n  -- second\n  LOCAL\n"),
        "{once}"
    );
    assert_eq!(format_text(&once), once);
}

#[test]
fn a_block_comment_on_its_own_line_before_local() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER;\n  (* a block *)\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  RETURN (x);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(once.contains("  (* a block *)\n  LOCAL\n"), "{once}");
    assert_eq!(format_text(&once), once);
}

#[test]
fn the_words_local_and_end_local_in_a_comment_are_not_keywords() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER;\n  -- LOCAL starts here\n  LOCAL\n    y : INTEGER;\n    -- END_LOCAL follows\n  END_LOCAL;\n  RETURN (x);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(once.contains(
        "  -- LOCAL starts here\n  LOCAL\n    y : INTEGER;\n    -- END_LOCAL follows\n  END_LOCAL;\n"
    ), "{once}");
    assert_eq!(format_text(&once), once);
}

#[test]
fn a_lower_case_local() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER;\n  -- before local\n  local\n    y : INTEGER;\n  end_local;\n  RETURN (x);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(
        once.contains("  -- before local\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n"),
        "{once}"
    );
    assert_eq!(format_text(&once), once);
}

#[test]
fn a_comment_between_two_variables_is_above_the_second() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n    -- between\n    z : INTEGER;\n  END_LOCAL;\n  RETURN (x);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(
        once.contains("    y : INTEGER;\n    -- between\n    z : INTEGER;\n"),
        "{once}"
    );
    assert_eq!(format_text(&once), once);
}

#[test]
fn a_comment_above_a_group_of_variables_on_one_line() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER;\n  LOCAL\n    -- above the group\n    a, b : INTEGER;\n  END_LOCAL;\n  RETURN (x);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(
        once.contains("    -- above the group\n    a, b : INTEGER;\n"),
        "{once}"
    );
    assert_eq!(format_text(&once), once);
}

#[test]
fn a_comment_before_a_nested_type() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER;\n  -- before a nested type\n  TYPE t = INTEGER;\n  END_TYPE;\n  RETURN (x);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(
        once.contains(
            "FUNCTION f(x : INTEGER) : INTEGER;\n  -- before a nested type\n  TYPE t = INTEGER;\n"
        ),
        "{once}"
    );
    assert_eq!(format_text(&once), once);
}

#[test]
fn comments_at_the_end_of_a_line_and_among_the_statements_stay() {
    let source = in_schema("FUNCTION f(x : INTEGER) : INTEGER; -- after the head\n  LOCAL -- after LOCAL\n    y : INTEGER; -- after a variable\n  END_LOCAL; -- after END_LOCAL\n  -- before the first statement\n  y := x;\n  -- between statements\n  RETURN (y);\n  -- after the last statement\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(once.contains("y : INTEGER; -- after a variable"), "{once}");
    assert!(once.contains("  -- before the first statement\n  y := x;\n  -- between statements\n  RETURN (y);\n  -- after the last statement\nEND_FUNCTION;"), "{once}");
    assert_eq!(format_text(&once), once);
}

const PRETTY_EXPECTED: &str = "SCHEMA function_probe;\n\nENTITY e;\nEND_ENTITY;\n\nFUNCTION f1(x : INTEGER) : INTEGER;\n  -- F01 on its own line before LOCAL\n  LOCAL\n    -- F02 on its own line before the first variable\n    y : INTEGER;\n    -- F03 on its own line before END_LOCAL\n  END_LOCAL;\n  -- F04 on its own line before the first statement\n  y := x;\n  RETURN (y);\nEND_FUNCTION;\n\nFUNCTION f2(x : INTEGER) : INTEGER;\n  -- F05 on its own line before a nested function\n  FUNCTION g(z : INTEGER) : INTEGER;\n    RETURN (z);\n  END_FUNCTION;\n  -- F06 on its own line before a nested procedure\n  PROCEDURE p(z : INTEGER);\n  END_PROCEDURE;\n  CONSTANT\n    -- F07 on its own line before a constant block\n    k : INTEGER := 1;\n  END_CONSTANT;\n  RETURN (g(x) + k);\nEND_FUNCTION;\n\nRULE r1 FOR (e);\n  -- F09 on its own line before LOCAL of a rule\n  LOCAL\n    y : INTEGER := 0;\n  END_LOCAL;\nWHERE\n  wr1: y = 0;\nEND_RULE;\n\nPROCEDURE p1(x : INTEGER);\n  -- F08 on its own line before LOCAL of a procedure\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  y := x;\nEND_PROCEDURE;\n\nEND_SCHEMA;\n";

#[test]
fn the_pretty_form_writes_them_where_they_stood_too() {
    let wire = attached(PROBE);
    let options = r#"{"indent":2,"provenance":false,"no_remarks":false}"#;
    let pretty = expressir::formatter::pretty_format(&wire, "", options).expect("pretty");
    assert_eq!(pretty, PRETTY_EXPECTED);
}

#[test]
fn the_pretty_form_without_remarks_has_none() {
    let wire = attached(PROBE);
    let options = r#"{"indent":2,"provenance":false,"no_remarks":true}"#;
    let pretty = expressir::formatter::pretty_format(&wire, "", options).expect("pretty");
    assert!(!pretty.contains("--"), "{pretty}");
}

fn with_col(body: &str) -> String {
    format!("SCHEMA s;\nTYPE col = ENUMERATION OF (red, green);\nEND_TYPE;\n{body}ENTITY e;\nEND_ENTITY;\nEND_SCHEMA;\n")
}

#[test]
fn a_comment_between_the_names_of_one_declaration_stays_before_the_first_statement() {
    let source = with_col("FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    a,\n    -- cb\n    b : INTEGER;\n  END_LOCAL;\n  RETURN (1);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(
        once.contains(
            "  LOCAL\n    a, b : INTEGER;\n  END_LOCAL;\n  -- cb\n  RETURN (1);\nEND_FUNCTION;"
        ),
        "{once}"
    );
    assert_eq!(format_text(&once), once);
}

#[test]
fn a_comment_in_a_case_stays_in_the_case_when_its_label_is_a_variable_name() {
    for body in [
        "FUNCTION f(p : col) : INTEGER;\n  LOCAL\n    red : INTEGER := 0;\n  END_LOCAL;\n  CASE p OF\n    green : red := 1;\n    -- cc\n    red : red := 2;\n  END_CASE;\n  RETURN (red);\nEND_FUNCTION;\n",
        "FUNCTION f(p : col) : INTEGER;\n  LOCAL\n    red : INTEGER := 0;\n  END_LOCAL;\n  CASE p OF\n    -- cc\n    red : red := 2;\n  END_CASE;\n  RETURN (red);\nEND_FUNCTION;\n",
        "PROCEDURE q(p : col);\n  LOCAL\n    red : INTEGER := 0;\n  END_LOCAL;\n  CASE p OF\n    green : red := 1;\n    -- cc\n    red : red := 2;\n  END_CASE;\nEND_PROCEDURE;\n",
    ] {
        let once = format_text(&with_col(body));
        // Where the CASE writes it is not the subject here: it is not
        // written into the LOCAL block.
        assert!(once.find("-- cc").unwrap() > once.find("END_LOCAL;").unwrap(), "{once}");
        assert!(once.contains("  LOCAL\n    red : INTEGER := 0;\n  END_LOCAL;"), "{once}");
        assert!(!once.contains("    -- cc\n    red : INTEGER"), "{once}");
        assert_eq!(format_text(&once), once);
    }
}

#[test]
fn comments_before_a_begin_an_if_and_in_a_then_body_stay_in_the_statements() {
    let source = with_col("FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER := 0;\n  END_LOCAL;\n  y := 1;\n  -- before begin\n  BEGIN\n    y := 2;\n  END;\n  -- before if\n  IF y = 2 THEN\n    -- in then\n    y := 3;\n  END_IF;\n  RETURN (y);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(once.contains("  y := 1;\n  -- before begin\n  BEGIN\n    y := 2;\n  END;\n  -- before if\n  IF y = 2 THEN\n    -- in then\n    y := 3;\n  END_IF;\n  RETURN (y);"), "{once}");
    assert_eq!(format_text(&once), once);
}

#[test]
fn a_comment_of_several_lines_and_the_closing_keywords_are_written_as_before() {
    let cases: [(&str, &str, &str); 10] = [
        ("a block before LOCAL", "FUNCTION f(p : INTEGER) : INTEGER;\n  (* first line\n     second line *)\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  RETURN (y);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  (*\n  first line\n       second line\n  *)\n  RETURN (y);\nEND_FUNCTION;\n"),
        ("a block before a variable", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    (* first line\n     second line *)\n    y : INTEGER;\n  END_LOCAL;\n  RETURN (y);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  (*\n  first line\n       second line\n  *)\n  RETURN (y);\nEND_FUNCTION;\n"),
        ("a block before END_LOCAL", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n    (* first line\n     second line *)\n  END_LOCAL;\n  RETURN (y);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  (*\n  first line\n       second line\n  *)\n  RETURN (y);\nEND_FUNCTION;\n"),
        ("a block before a nested function", "FUNCTION f(p : INTEGER) : INTEGER;\n  (* first line\n     second line *)\n  FUNCTION g(z : INTEGER) : INTEGER;\n    RETURN (z);\n  END_FUNCTION;\n  RETURN (1);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  FUNCTION g(z : INTEGER) : INTEGER;\n    RETURN (z);\n  END_FUNCTION;\n  (*\n  first line\n       second line\n  *)\n  RETURN (1);\nEND_FUNCTION;\n"),
        ("a nested block before LOCAL", "FUNCTION f(p : INTEGER) : INTEGER;\n  (* a (* b *)\n     c *)\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  RETURN (y);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  (*\n  a (* b *)\n       c\n  *)\n  RETURN (y);\nEND_FUNCTION;\n"),
        ("a nested block before a variable", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    (* a (* b *)\n     c *)\n    y : INTEGER;\n  END_LOCAL;\n  RETURN (y);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  (*\n  a (* b *)\n       c\n  *)\n  RETURN (y);\nEND_FUNCTION;\n"),
        ("a nested block before END_LOCAL", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n    (* a (* b *)\n     c *)\n  END_LOCAL;\n  RETURN (y);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER;\n  END_LOCAL;\n  (*\n  a (* b *)\n       c\n  *)\n  RETURN (y);\nEND_FUNCTION;\n"),
        ("a nested block before a nested function", "FUNCTION f(p : INTEGER) : INTEGER;\n  (* a (* b *)\n     c *)\n  FUNCTION g(z : INTEGER) : INTEGER;\n    RETURN (z);\n  END_FUNCTION;\n  RETURN (1);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  FUNCTION g(z : INTEGER) : INTEGER;\n    RETURN (z);\n  END_FUNCTION;\n  (*\n  a (* b *)\n       c\n  *)\n  RETURN (1);\nEND_FUNCTION;\n"),
        ("closing keywords with comment 0", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER := 0;\n  END_LOCAL;\n  IF y = 0 THEN\n    y := 1;\n    -- c\n  ELSE\n    y := 2;\n    -- c\n  END_IF;\n  REPEAT i := 1 TO 2;\n    y := 3;\n    -- c\n  END_REPEAT;\n  RETURN (y);\n  -- c\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER := 0;\n  END_LOCAL;\n  IF y = 0 THEN\n    y := 1;\n    -- c\n  ELSE\n    y := 2;\n    -- c\n  END_IF;\n  REPEAT i := 1 TO 2;\n    y := 3;\n    -- c\n  END_REPEAT;\n  RETURN (y);\n  -- c\nEND_FUNCTION;\n"),
        ("closing keywords with comment 1", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER := 0;\n  END_LOCAL;\n  IF y = 0 THEN\n    y := 1;\n    (* c *)\n  ELSE\n    y := 2;\n    (* c *)\n  END_IF;\n  REPEAT i := 1 TO 2;\n    y := 3;\n    (* c *)\n  END_REPEAT;\n  RETURN (y);\n  (* c *)\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    y : INTEGER := 0;\n  END_LOCAL;\n  IF y = 0 THEN\n    y := 1;\n    (* c *)\n  ELSE\n    y := 2;\n    (* c *)\n  END_IF;\n  REPEAT i := 1 TO 2;\n    y := 3;\n    (* c *)\n  END_REPEAT;\n  RETURN (y);\n  (* c *)\nEND_FUNCTION;\n"),
    ];
    for (name, body, expected) in cases {
        let once = format_text(&in_schema(body));
        assert!(once.contains(expected), "{name}\n{once}");
    }
}

#[test]
fn a_block_comment_followed_by_code_on_its_line_is_left_as_it_was() {
    let cases: [(&str, &str); 5] = [
        ("FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    (* c *) a : INTEGER;\n    b : INTEGER;\n  END_LOCAL;\n  RETURN (1);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    a : INTEGER;\n    b : INTEGER;\n  END_LOCAL;\n  (* c *)\n  RETURN (1);\nEND_FUNCTION;\n"),
        ("FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    (* c *) a : INTEGER;\n  END_LOCAL;\n  RETURN (1);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    a : INTEGER;\n  END_LOCAL;\n  (* c *)\n  RETURN (1);\nEND_FUNCTION;\n"),
        ("FUNCTION f(p : INTEGER) : INTEGER;\n  (* c *) FUNCTION g(z : INTEGER) : INTEGER;\n    RETURN (z);\n  END_FUNCTION;\n  RETURN (1);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  FUNCTION g(z : INTEGER) : INTEGER;\n    (* c *)\n    RETURN (z);\n  END_FUNCTION;\n  RETURN (1);\nEND_FUNCTION;\n"),
        ("FUNCTION f(p : INTEGER) : INTEGER;\n  (* c *) LOCAL\n    a : INTEGER;\n  END_LOCAL;\n  RETURN (1);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    a : INTEGER;\n  END_LOCAL;\n  (* c *)\n  RETURN (1);\nEND_FUNCTION;\n"),
        ("FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    a : INTEGER;\n    (* c *) END_LOCAL;\n  RETURN (1);\nEND_FUNCTION;\n", "FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    a : INTEGER;\n  END_LOCAL;\n  (* c *)\n  RETURN (1);\nEND_FUNCTION;\n"),
    ];
    for (body, expected) in cases {
        let once = format_text(&in_schema(body));
        assert!(once.contains(expected), "{once}");
        assert_eq!(format_text(&once), once);
    }
}

#[test]
fn a_comment_after_non_ascii_text_in_the_local_block() {
    let source = in_schema("FUNCTION f(p : INTEGER) : INTEGER;\n  LOCAL\n    -- Příliš žluťoučký\n    y : STRING := 'ůě';\n    -- c\n    z : INTEGER;\n  END_LOCAL;\n  RETURN (1);\nEND_FUNCTION;\n");
    let once = format_text(&source);
    assert!(once.contains("    -- Příliš žluťoučký\n    y : STRING := 'ůě';\n    -- c\n    z : INTEGER;\n  END_LOCAL;"), "{once}");
    assert_eq!(format_text(&once), once);
}
