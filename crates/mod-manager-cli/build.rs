fn main() {
    // no link time in the file header: the same source and tools then give the
    // same file, byte for byte
    if std::env::var("TARGET").is_ok_and(|target| target == "x86_64-pc-windows-gnu") {
        println!("cargo:rustc-link-arg=-Wl,--no-insert-timestamp");
    }

    println!("cargo:rerun-if-changed=build.rs");
}
