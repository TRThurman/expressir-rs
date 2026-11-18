fn main() {
    let source = "SCHEMA demo;\nENTITY e;\n  a : STRING;\n  (*\"demo.e\" the note *)\nEND_ENTITY;\nEND_SCHEMA;\n";
    let mut wire = expressir::ParsedTree::parse(source)
        .expect("parses")
        .to_model_json("s.exp")
        .expect("wire");
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);
    match expressir::ParsedTree::parse(source) {
        Ok(t) => match t.to_model_json("demo.exp") {
            Ok(v) => println!("MODELJSON OK len={}", v.to_string().len()),
            Err(e) => println!("MODELJSON ERR={e:?}"),
        },
        Err(e) => println!("PARSE ERR={e:?}"),
    }
    println!(
        "REMARKS: {}",
        wire.pointer("/schemas/0/entities/0/remarks")
            .expect("remarks")
    );
    println!(
        "UNTAGGED: {}",
        wire.pointer("/schemas/0/entities/0/untagged_remarks")
            .expect("untagged")
    );
    println!("FORMAT: {}", expressir::formatter::format(&wire, false));
}
