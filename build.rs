use protoc_rust;

fn main() {
    protoc_rust::Codegen::new()
        .out_dir("src/protos")
        .inputs(&["protos/Mumble.proto"])
        .include("protos")
        .run()
        .expect("Running protoc failed.");
}
