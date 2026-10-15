fn main() {
    // Cross-target type checking is useful on Linux, but must never produce a
    // release executable without the native resources and security manifest.
    if !cfg!(windows)
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("PROFILE").as_deref() == Ok("release")
    {
        panic!("Windows release builds require a Windows MSVC/SDK host for resources");
    }
    println!("cargo:rerun-if-changed=resources/settings.rc");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    println!("cargo:rerun-if-changed=../../../assets/icon.ico");
    #[cfg(windows)]
    embed_resource::compile("resources/settings.rc", embed_resource::NONE)
        .manifest_required()
        .expect("Settings resources and DPI manifest must be linked");
}
