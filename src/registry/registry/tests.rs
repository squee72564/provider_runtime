#[test]
fn test_module_scaffold_exists() {
    let this_file_exists = std::path::Path::new(file!()).exists();
    assert!(this_file_exists);
}
