use serde_json::json;

#[test]
fn resolve_uses_the_root_path_segment() {
    let mut wire = json!({
        "_class": "Expressir::Model::ExpFile",
        "path": "spec/syntax/single.exp",
        "schemas": [{
            "_class": "Expressir::Model::Declarations::Schema",
            "id": "single_schema",
            "entities": [
                {"_class": "Expressir::Model::Declarations::Entity", "id": "empty_entity"},
                {"_class": "Expressir::Model::Declarations::Entity", "id": "subtype_empty_entity",
                 "subtype_of": [{"_class": "Expressir::Model::References::SimpleReference", "id": "empty_entity"}]}
            ]
        }]
    });
    let n = expressir::checker::resolve_references(&mut wire);
    assert!(n > 0, "nothing resolved");
    let fill = wire["schemas"][0]["entities"][1]["subtype_of"][0]["base_path"]
        .as_str()
        .expect("fill");
    assert_eq!(fill, "spec/syntax/single.exp.single_schema.empty_entity");
}
