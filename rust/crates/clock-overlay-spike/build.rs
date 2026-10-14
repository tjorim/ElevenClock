fn main() {
    println!("cargo:rerun-if-changed=resources/overlay.rc");
    println!("cargo:rerun-if-changed=resources/overlay.manifest");
    // Only the development spike receives these resources, never clock-app or the R02 probe.
    #[cfg(windows)]
    {
        let compilation = embed_resource::compile("resources/overlay.rc", embed_resource::NONE);
        compilation
            .manifest_required()
            .expect("R03 prototype manifest compilation");
    }
}
