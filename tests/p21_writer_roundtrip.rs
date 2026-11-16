//! The P21 writer round trip (#642 PR-1): parse → write → parse must
//! hold value identity over the baked ISO 10303-21 conformance corpus,
//! and the writer must be deterministic (write(parse(write(x))) ==
//! write(x)). Value identity, not byte identity — comments and source
//! formatting do not survive the round trip by design.

use std::path::PathBuf;

use expressir_rs::population::{parse_full_population, write_population};

fn corpus() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p21_corpus");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("corpus dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("entries")
        .into_iter()
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|e| e == "p21"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let source = std::fs::read_to_string(&p).expect("read");
            (name, source)
        })
        .collect()
}

#[test]
fn round_trips_the_conformance_corpus_with_value_identity() {
    for (name, source) in corpus() {
        let first = parse_full_population(&source)
            .unwrap_or_else(|err| panic!("{name}: parse failed: {err}"));
        let text = write_population(&first);
        let second = parse_full_population(&text)
            .unwrap_or_else(|err| panic!("{name}: reparse failed: {err}"));
        assert_eq!(
            first, second,
            "{name}: value identity lost in the round trip"
        );
        let again = write_population(&second);
        assert_eq!(text, again, "{name}: the writer is not deterministic");
    }
}

#[test]
fn re_emits_named_sections_and_anchors() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p21_corpus/annex-e-0.p21"),
    )
    .expect("fixture");
    let population = parse_full_population(&source).expect("parse");
    let text = write_population(&population);
    assert!(text.contains("DATA ('"), "named sections lost: {text}");
    assert_eq!(population.sections.len(), 2, "annex-e has two sections");
}

#[test]
fn round_trips_a_constructed_population() {
    use expressir_rs::population::{Instance, P21Value as Value, Record, Section, Typed};

    let population = expressir_rs::population::FullPopulation {
        header: vec![(
            "FILE_SCHEMA".to_string(),
            vec![Value::List(vec![Value::Str("DEMO".to_string())])],
        )],
        instances: vec![
            (
                1,
                Instance::Simple(Record {
                    keyword: "POINT".to_string(),
                    params: vec![
                        Value::Number("1.5".to_string()),
                        Value::Unset,
                        Value::Bool(true),
                    ],
                }),
            ),
            (
                2,
                Instance::Complex(vec![
                    Record {
                        keyword: "A".to_string(),
                        params: vec![Value::Ref(1)],
                    },
                    Record {
                        keyword: "B".to_string(),
                        params: vec![Value::Typed(Typed {
                            keyword: "LEN".to_string(),
                            params: vec![Value::Enum("METRE".to_string())],
                        })],
                    },
                ]),
            ),
        ],
        sections: vec![Section {
            name: Some("ONE".to_string()),
            contexts: vec!["BASE".to_string()],
            instance_ids: vec![1, 2],
        }],
        anchors: vec![("lbl".to_string(), Value::Ref(1))],
        references: vec![(3, "http://example.org/p".to_string())],
    };
    let text = write_population(&population);
    assert!(text.contains("DATA ('ONE', ('BASE'));"), "{text}");
    assert!(text.contains("#2=(A(#1)B(LEN(.METRE.)));"), "{text}");
    assert!(text.contains("<lbl> = #1;"), "{text}");
    assert!(text.contains("#3 = <http://example.org/p>;"), "{text}");
    let reparsed = parse_full_population(&text).expect("reparse");
    // The grammar captures .T. as an enumeration (the ruby builder has
    // no boolean special-case either) — value identity holds through
    // the enum form.
    let mut expected = population.clone();
    expected.instances[0].1 = Instance::Simple(Record {
        keyword: "POINT".to_string(),
        params: vec![
            Value::Number("1.5".to_string()),
            Value::Unset,
            Value::Enum("T".to_string()),
        ],
    });
    assert_eq!(expected, reparsed);
}
