// The MinGW CRT entry calls the stdcall-decorated `_DllMain@12`; our
// `extern "system" fn DllMain` is exported undecorated. Accept the fixup
// silently on 32-bit MinGW instead of printing a linker warning each build.
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os == "windows" && env == "gnu" && arch == "x86" {
        println!("cargo:rustc-link-arg-cdylib=-Wl,--enable-stdcall-fixup");
    }
}
