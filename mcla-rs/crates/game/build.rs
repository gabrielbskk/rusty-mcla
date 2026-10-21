fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        // oboe's static C++ code needs the Android C++ runtime at dlopen time.
        println!("cargo:rustc-link-lib=dylib=c++_shared");
    }
}
