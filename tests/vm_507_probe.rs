//! The #496 regression pin on parsanol =0.20.21: the VM must parse the
//! compiled EXPRESS grammar to the same end position as the packrat
//! lane. On 0.20.20 the VM short-parsed (Ok(10) vs packrat 21028 — the
//! prefix-split choice fail-jump); fixed and released on all five tiers
//! with 0.20.21 (engine 8051d15). The route stays OUT of the default
//! path regardless: the vm_bench example measured 30s+ timeouts on
//! 637KB–4.2MB corpus files on this engine — correctness without speed.

use parsanol::portable::bytecode::compiler::Compiler;
use parsanol::portable::bytecode::vm::{BytecodeVM, VMConfig};
use parsanol::portable::{AstArena, PortableParser};

#[test]
fn ground_truth_three_paths() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/syntax.exp"
    ))
    .unwrap();
    println!("bytes: {}", source.len());

    let grammar = expressir::grammar();
    let mut arena = AstArena::for_input(source.len());
    arena.set_input(source.clone());
    let packrat_end = PortableParser::new(grammar, &source, &mut arena)
        .parse_with_end_pos()
        .map(|r| r.end_pos)
        .expect("packrat parse");
    println!("packrat end_pos: {:?}", packrat_end);

    let program = Compiler::new(grammar.clone()).compile().unwrap();
    let mut arena2 = AstArena::for_input(source.len());
    arena2.set_input(source.clone());
    let vm_end = BytecodeVM::new(&program, &source, &mut arena2, VMConfig::default())
        .run()
        .map(|r| r.end_pos)
        .expect("vm parse");
    println!("vm end_pos: {:?}", vm_end);
    assert_eq!(
        vm_end, packrat_end,
        "the VM must parse to the packrat end position (#496)"
    );
}
