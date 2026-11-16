//! The wasm surface's plain JSON functions (expressir#620's tier
//! contract): the same operations the wasm-bindgen wrappers export
//! run natively here — schema parse, remark attach, format, check,
//! and the Part 21 population — so a regression in the wire behavior
//! is caught without a wasm toolchain.

use expressir_rs::wasm;

const SCHEMA: &str = "SCHEMA demo;\nENTITY e;\n  a : STRING;\n  (*\"demo.e\" the note *)\nEND_ENTITY;\nEND_SCHEMA;\n";
const POPULATION: &str = "ISO-10303-21;\nHEADER;\nFILE_NAME('x');\nENDSEC;\nDATA;\n#1=E('v');\nENDSEC;\nEND-ISO-10303-21;\n";

#[test]
fn schema_parse_inventories_declarations() {
    let json = wasm::parse_schema_json(SCHEMA.to_string()).expect("schema parse");
    assert!(json.contains("\"name\":\"demo\""), "schemas: {json}");
    assert!(json.contains("\"a\""), "attributes: {json}");
}

#[test]
fn wire_schema_matches_the_c_abi_shape() {
    let json = wasm::wire_schema_json(SCHEMA.to_string()).expect("wire parse");
    assert!(
        json.contains("\"_class\":\"Expressir::Model::ExpFile\""),
        "model wire: {json}"
    );
}

#[test]
fn attach_format_check_round_trip() {
    // The model-hash wire — what attach/format/check consume — comes
    // from the wire_schema op (the C ABI's expr_parse_schema shape).
    let wire_json = wasm::wire_schema_json(SCHEMA.to_string()).expect("wire parse");

    let attached =
        wasm::attach_remarks_json(wire_json.clone(), SCHEMA.to_string(), false).expect("attach");
    assert!(attached.contains("the note"), "attached: {attached}");

    let text = wasm::format_json(attached.clone(), false).expect("format");
    assert!(text.contains("SCHEMA demo;"), "format: {text}");
    let stripped = wasm::format_json(attached, true).expect("format stripped");
    assert!(stripped.contains("END_ENTITY;"), "stripped: {stripped}");

    let findings = wasm::check_json(wire_json, None).expect("check");
    assert!(findings.starts_with('['), "findings: {findings}");
}

#[test]
fn resolve_fills_cross_schema_base_paths() {
    const TWO_SCHEMAS: &str = "SCHEMA base;\nENTITY base_entity;\nEND_ENTITY;\nEND_SCHEMA;\nSCHEMA user_schema;\nUSE FROM base;\nENTITY user_entity SUBTYPE OF (base_entity);\nEND_ENTITY;\nEND_SCHEMA;\n";
    let wire_json = wasm::wire_schema_json(TWO_SCHEMAS.to_string()).expect("wire parse");
    let resolved = wasm::resolve_json(wire_json).expect("resolve");
    assert!(
        resolved.contains("\"base_path\":\"source.exp.base.base_entity\""),
        "resolved: {resolved}"
    );
}

#[test]
fn round_trip_population_emits_canonical_text() {
    let src = "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('DEMO'));\nENDSEC;\nDATA;\n#1=E('v');\nENDSEC;\nEND-ISO-10303-21;\n";
    let text = wasm::round_trip_population_json(src.to_string()).expect("round trip");
    assert!(text.contains("#1=E('v');"), "{text}");
}

#[test]
fn population_parse_and_validate() {
    let json = wasm::parse_population_json(POPULATION.to_string()).expect("population");
    assert!(json.contains("E"), "population: {json}");
    let verdict = wasm::validate_population_json("[\"E\"]".to_string(), POPULATION.to_string())
        .expect("validate");
    assert!(verdict.contains("\"valid\""), "verdict: {verdict}");
}

#[test]
fn p21_tooling_surfaces() {
    let src = "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('DEMO'));\nENDSEC;\nDATA;\n#1=E('v');\nENDSEC;\nEND-ISO-10303-21;\n".to_string();
    let wire = wasm::population_wire_json(src.clone()).expect("population wire");
    assert!(wire.contains("\"E\""), "{wire}");

    let diff = wasm::diff_population_json(src.clone(), src).expect("diff");
    assert!(diff.contains("\"added\":[]"), "{diff}");
}

#[test]
fn coverage_surface_classifies_entities() {
    let attached = wasm::attach_remarks_json(
        wasm::wire_schema_json(SCHEMA.to_string()).expect("wire parse"),
        SCHEMA.to_string(),
        false,
    )
    .expect("attach");
    let report = wasm::coverage_json(attached, "[]".to_string()).expect("coverage");
    assert!(report.contains("\"e\""), "rows: {report}");
    assert!(report.contains("\"documented\""), "rows: {report}");
}

#[test]
fn writer_and_symbol_surfaces() {
    let wire = wasm::wire_schema_json(SCHEMA.to_string()).expect("wire parse");

    let xsd = wasm::xsd_xml_json(wire.clone()).expect("xsd");
    assert!(xsd.starts_with("<xs:schema"), "{xsd}");

    let uos = wasm::uos_xml_json(
        "{\"instances\":{}}".to_string(),
        wire,
        "{\"id\":\"uos01\",\"target_namespace\":\"urn:t\",\"prefix\":\"t\"}".to_string(),
    )
    .expect("uos");
    assert!(uos.contains("<t:uos"), "{uos}");

    let symbols = wasm::lsp_symbols_json(SCHEMA.to_string()).expect("symbols");
    assert!(symbols.contains("\"kind\":"), "{symbols}");
}
