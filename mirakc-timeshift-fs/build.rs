pub fn main() {
    let gitcl = vergen_gitcl::Gitcl::builder()
        .sha(false) // full SHA
        .build();
    vergen_gitcl::Emitter::default()
        .add_instructions(&gitcl)
        .unwrap()
        .emit()
        .unwrap();
}
