fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("protoc-bin-vendored: cannot find protoc binary");
    // SAFETY: called in build.rs, single-threaded before user code runs
    unsafe { std::env::set_var("PROTOC", protoc); }

    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(
            &["proto/rpc.proto"],
            &["proto"],
        )?;
    Ok(())
}
