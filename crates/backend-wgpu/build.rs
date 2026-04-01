fn main() {
    wesl::Wesl::new("src/shaders").build_artifact(&"package::main".parse().unwrap(), "gx_shader");
}
