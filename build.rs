fn main() {
    println!("cargo:rerun-if-changed=protos/Mumble.proto");
    println!("cargo:rerun-if-changed=build.rs");

    let protoc = protoc_bin_vendored::protoc_bin_path().expect("failed to fetch vendored protoc");

    std::fs::create_dir_all("src/protos").expect("failed to create src/protos output directory");

    protobuf_codegen::Codegen::new()
        .protoc()
        .protoc_path(&protoc)
        .customize(protobuf_codegen::Customize::default().gen_mod_rs(false))
        .out_dir("src/protos")
        .include("protos")
        .input("protos/Mumble.proto")
        .run_from_script();
}
