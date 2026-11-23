//! A comment on a line of its own between two actions of a CASE is written
//! back above the label of the action it precedes. The wire holds it as
//! the leading remark of that action's statement, and the formatter wrote
//! it between the label and the statement.

fn lines(l: &[&str]) -> String {
    format!("{}\n", l.join("\n"))
}

/// The statements of a FUNCTION between its declaration and END_FUNCTION.
fn wrap(body: &[&str]) -> String {
    let mut all = vec![
        "SCHEMA case_probe;",
        "FUNCTION f(x : INTEGER; y : INTEGER) : INTEGER;",
        "  LOCAL r : INTEGER := 0; END_LOCAL;",
    ];
    all.extend_from_slice(body);
    all.extend_from_slice(&["  RETURN (r);", "END_FUNCTION;", "END_SCHEMA;"]);
    lines(&all)
}

fn expected(body: &[&str]) -> String {
    let mut all = vec![
        "SCHEMA case_probe;",
        "",
        "FUNCTION f(x : INTEGER;",
        "           y : INTEGER) : INTEGER;",
        "  LOCAL",
        "    r : INTEGER := 0;",
        "  END_LOCAL;",
    ];
    all.extend_from_slice(body);
    all.extend_from_slice(&["  RETURN (r);", "END_FUNCTION;", "", "END_SCHEMA;"]);
    lines(&all)
}

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

fn pretty(source: &str) -> String {
    expressir::formatter::pretty_format(&attached(source), "", "{}").expect("pretty")
}

fn main_source() -> String {
    wrap(&[
        "  CASE x OF",
        "    1 : r := 3;",
        "    -- own line between two case actions",
        "    2 : r := 4;",
        "  END_CASE;",
    ])
}

fn main_expected() -> String {
    expected(&[
        "  CASE x OF",
        "  1 :",
        "    r := 3;",
        "  -- own line between two case actions",
        "  2 :",
        "    r := 4;",
        "  END_CASE;",
    ])
}

#[test]
fn the_comment_is_written_above_the_label_it_precedes() {
    assert_eq!(format(&main_source()), main_expected());
}

#[test]
fn the_same_text_again_from_its_own_output() {
    let once = format(&main_source());
    assert_eq!(format(&once), once);
}

#[test]
fn the_stripped_form_has_no_comment_and_is_as_before() {
    let text = expressir::formatter::format(&attached(&main_source()), true);
    assert_eq!(
        text,
        expected(&[
            "  CASE x OF",
            "  1 :",
            "    r := 3;",
            "  2 :",
            "    r := 4;",
            "  END_CASE;",
        ])
    );
}

#[test]
fn the_remark_is_held_as_leading_by_the_statement_of_the_action_with_its_region() {
    let wire = attached(&main_source());
    let remark = wire
        .pointer("/schemas/0/functions/0/statements/0/actions/1/statement/untagged_remarks/0")
        .expect("remark on the statement of the second action");
    assert_eq!(remark["placement"], "leading");
    assert_eq!(remark["region"], "before_label");
}

#[test]
fn a_comment_after_the_label_keeps_the_statement_without_a_region() {
    let source = wrap(&[
        "  CASE x OF",
        "    1 : r := 3;",
        "    2 :",
        "      -- after the label",
        "      r := 4;",
        "  END_CASE;",
    ]);
    let wire = attached(&source);
    let remark = wire
        .pointer("/schemas/0/functions/0/statements/0/actions/1/statement/untagged_remarks/0")
        .expect("remark on the statement of the second action");
    assert_eq!(remark["placement"], "leading");
    assert!(remark.get("region").is_none_or(serde_json::Value::is_null));
    assert!(format(&source).contains("  2 :\n    -- after the label\n    r := 4;"));
}

#[test]
fn the_pretty_form_writes_it_above_the_label_too() {
    let text = pretty(&main_source());
    assert!(
        text.contains(
            "r := 3;\n    -- own line between two case actions\n    2 :\n        r := 4;"
        ),
        "{text}"
    );
    assert_eq!(pretty(&text), text);
}

#[test]
fn a_comment_before_the_first_action_and_two_in_a_row() {
    let source = wrap(&[
        "  CASE x OF",
        "    -- before the first action",
        "    1 : r := 3;",
        "    -- first of two",
        "    -- second of two",
        "    2 : r := 4;",
        "  END_CASE;",
    ]);
    let want = expected(&[
        "  CASE x OF",
        "  -- before the first action",
        "  1 :",
        "    r := 3;",
        "  -- first of two",
        "  -- second of two",
        "  2 :",
        "    r := 4;",
        "  END_CASE;",
    ]);
    assert_eq!(format(&source), want);
    assert_eq!(format(&want), want);
}

#[test]
fn a_block_comment_and_every_shape_of_label() {
    let source = wrap(&[
        "  CASE x OF",
        "    1 : r := 1;",
        "    (* a block comment *)",
        "    2, 3 : r := 2;",
        "    -- a negative number",
        "    -5 : r := 3;",
        "    -- a parenthesised expression",
        "    (6 + 1) : r := 4;",
        "    -- an identifier",
        "    abc : r := 5;",
        "    -- a label on its own line",
        "    7, 8 :",
        "      r := 6;",
        "    -- before a label that is followed by a comment",
        "    9 :",
        "      -- a comment after the label",
        "      r := 7;",
        "  END_CASE;",
    ]);
    let want = expected(&[
        "  CASE x OF",
        "  1 :",
        "    r := 1;",
        "  (* a block comment *)",
        "  2, 3 :",
        "    r := 2;",
        "  -- a negative number",
        "  -5 :",
        "    r := 3;",
        "  -- a parenthesised expression",
        "  (6 + 1) :",
        "    r := 4;",
        "  -- an identifier",
        "  abc :",
        "    r := 5;",
        "  -- a label on its own line",
        "  7, 8 :",
        "    r := 6;",
        "  -- before a label that is followed by a comment",
        "  9 :",
        "    -- a comment after the label",
        "    r := 7;",
        "  END_CASE;",
    ]);
    assert_eq!(format(&source), want);
    assert_eq!(format(&want), want);
}

#[test]
fn a_compound_action_keeps_the_comments_inside_it() {
    let source = wrap(&[
        "  CASE y OF",
        "    1 : BEGIN",
        "      r := 1;",
        "      -- inside the compound",
        "      r := 2;",
        "    END;",
        "    -- after the compound",
        "    2 : BEGIN",
        "      -- first in the compound",
        "      r := 3;",
        "    END;",
        "  END_CASE;",
    ]);
    let want = expected(&[
        "  CASE y OF",
        "  1 :",
        "    BEGIN",
        "      r := 1;",
        "      -- inside the compound",
        "      r := 2;",
        "    END;",
        "  -- after the compound",
        "  2 :",
        "    BEGIN",
        "      -- first in the compound",
        "      r := 3;",
        "    END;",
        "  END_CASE;",
    ]);
    assert_eq!(format(&source), want);
    assert_eq!(format(&want), want);
}

#[test]
fn a_case_inside_a_case_action_inside_if_and_repeat() {
    let source = wrap(&[
        "  CASE y OF",
        "    1 : CASE x OF",
        "      1 : r := 1;",
        "      -- inner between",
        "      2 : r := 2;",
        "    END_CASE;",
        "    -- outer between",
        "    2 : r := 3;",
        "  END_CASE;",
        "  IF x > 0 THEN",
        "    CASE y OF",
        "      1 : r := 1;",
        "      -- in IF",
        "      2 : r := 2;",
        "    END_CASE;",
        "  END_IF;",
        "  REPEAT i := 1 TO 3;",
        "    CASE y OF",
        "      1 : r := 1;",
        "      -- in REPEAT",
        "      2 : r := 2;",
        "    END_CASE;",
        "  END_REPEAT;",
    ]);
    let want = expected(&[
        "  CASE y OF",
        "  1 :",
        "    CASE x OF",
        "    1 :",
        "      r := 1;",
        "    -- inner between",
        "    2 :",
        "      r := 2;",
        "    END_CASE;",
        "  -- outer between",
        "  2 :",
        "    r := 3;",
        "  END_CASE;",
        "  IF x > 0 THEN",
        "    CASE y OF",
        "    1 :",
        "      r := 1;",
        "    -- in IF",
        "    2 :",
        "      r := 2;",
        "    END_CASE;",
        "  END_IF;",
        "  REPEAT i := 1 TO 3;",
        "    CASE y OF",
        "    1 :",
        "      r := 1;",
        "    -- in REPEAT",
        "    2 :",
        "      r := 2;",
        "    END_CASE;",
        "  END_REPEAT;",
    ]);
    assert_eq!(format(&source), want);
    assert_eq!(format(&want), want);
}

#[test]
fn the_comments_before_otherwise_and_end_case_are_as_they_were() {
    let source = wrap(&[
        "  CASE x OF",
        "    1 : r := 3;",
        "    -- before OTHERWISE",
        "    OTHERWISE : r := 5;",
        "    -- before END_CASE",
        "  END_CASE;",
        "  CASE y OF",
        "    1 : r := 3;",
        "    -- before END_CASE of a CASE without OTHERWISE",
        "  END_CASE;",
    ]);
    let want = expected(&[
        "  CASE x OF",
        "  1 :",
        "    r := 3;",
        "    -- before OTHERWISE",
        "  OTHERWISE :",
        "    r := 5;",
        "    -- before END_CASE",
        "  END_CASE;",
        "  CASE y OF",
        "  1 :",
        "    r := 3;",
        "    -- before END_CASE of a CASE without OTHERWISE",
        "  END_CASE;",
    ]);
    assert_eq!(format(&source), want);
}

#[test]
fn a_comment_at_the_end_of_an_action_line_and_one_after_a_bare_label_stay() {
    let source = wrap(&[
        "  CASE x OF",
        "    1 : -- after a bare label",
        "      r := 3;",
        "    2 : r := 4; -- at the end of an action line",
        "  END_CASE;",
    ]);
    let want = expected(&[
        "  CASE x OF",
        "  1 : -- after a bare label",
        "    r := 3;",
        "  2 :",
        "    r := 4; -- at the end of an action line",
        "  END_CASE;",
    ]);
    assert_eq!(format(&source), want);
    assert_eq!(format(&want), want);
}

/// Block comments before and after the label, nested ones included.
fn block_cases() -> Vec<(Vec<&'static str>, Vec<&'static str>)> {
    vec![
        (
            vec![
                "  CASE x OF",
                "    1 : r := 3;",
                "    (* a *)",
                "    2 :",
                "      (* b *)",
                "      r := 4;",
                "  END_CASE;",
            ],
            vec![
                "  CASE x OF",
                "  1 :",
                "    r := 3;",
                "  (* a *)",
                "  2 :",
                "    (* b *)",
                "    r := 4;",
                "  END_CASE;",
            ],
        ),
        (
            vec![
                "  CASE x OF",
                "    1 : r := 3;",
                "    (* a *)",
                "    2 : (* b *)",
                "      r := 4;",
                "  END_CASE;",
            ],
            vec![
                "  CASE x OF",
                "  1 :",
                "    r := 3;",
                "  (* a *)",
                "  2 :",
                "    (* b *)",
                "    r := 4;",
                "  END_CASE;",
            ],
        ),
        (
            vec![
                "  CASE x OF",
                "    1 : r := 3;",
                "    2 :",
                "      (* a (* n *) z *)",
                "      r := 4;",
                "  END_CASE;",
            ],
            vec![
                "  CASE x OF",
                "  1 :",
                "    r := 3;",
                "  2 :",
                "    (* a (* n *) z *)",
                "    r := 4;",
                "  END_CASE;",
            ],
        ),
        (
            vec![
                "  CASE x OF",
                "    1 : r := 3;",
                "    2 :",
                "      -- z",
                "      (* a (* n *) c *)",
                "      r := 4;",
                "  END_CASE;",
            ],
            vec![
                "  CASE x OF",
                "  1 :",
                "    r := 3;",
                "  2 :",
                "    -- z",
                "    (* a (* n *) c *)",
                "    r := 4;",
                "  END_CASE;",
            ],
        ),
        (
            vec![
                "  CASE x OF",
                "    1 : r := 3;",
                "    (* a (* n *) z *)",
                "    2 : r := 4;",
                "  END_CASE;",
            ],
            vec![
                "  CASE x OF",
                "  1 :",
                "    r := 3;",
                "  (* a (* n *) z *)",
                "  2 :",
                "    r := 4;",
                "  END_CASE;",
            ],
        ),
    ]
}

#[test]
fn block_and_nested_block_comments_are_judged_each_from_its_own_position() {
    for (src, want) in block_cases() {
        let (source, want) = (wrap(&src), expected(&want));
        assert_eq!(format(&source), want, "{source}");
        assert_eq!(format(&want), want, "second pass of {source}");
    }
}

#[test]
fn the_plain_form_writes_the_comments_above_the_label() {
    let wire = attached(&main_source());
    let text = expressir::formatter::format_with_keep(&wire, true, true);
    assert!(
        text.contains("r := 3;\n  -- own line between two case actions\n  2 :\n    r := 4;"),
        "{text}"
    );
}
