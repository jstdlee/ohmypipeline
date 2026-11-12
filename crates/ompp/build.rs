fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=OMPP_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
