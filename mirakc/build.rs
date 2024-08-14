pub fn main() {
    let gitcl = vergen_gitcl::GitclBuilder::default()
        .sha(false) // full SHA
        .build()
        .unwrap();
    vergen_gitcl::Emitter::default()
        .add_instructions(&gitcl)
        .unwrap()
        .emit()
        .unwrap();
}
