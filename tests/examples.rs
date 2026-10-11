use offload::config::{Config, parse_pack};

#[test]
fn shipped_examples_are_valid() {
    let root = env!("CARGO_MANIFEST_DIR");
    let cfg = Config::load(&std::path::Path::new(root).join("examples/hectorir.toml")).unwrap();
    assert_eq!(cfg, Config::default(), "examples/hectorir.toml must match the built-in preset");
    let pack = std::fs::read_to_string(std::path::Path::new(root).join("examples/gopro.pack.toml")).unwrap();
    assert_eq!(parse_pack(&pack).unwrap().name, "GoPro");
}
