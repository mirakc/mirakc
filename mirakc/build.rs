pub fn main() {
    vergen::EmitBuilder::builder()
        .git_sha(false) // full SHA
        .emit()
        .unwrap();
}
