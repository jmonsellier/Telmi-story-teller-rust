//! Chemins d'édition de liens vers SDL2.
//!
//! - Mac (simulateur) : SDL2 de Homebrew.
//! - Miyoo : `MIYOO_LIB_DIR` contient la `libSDL2.so` de Telmi OS (voir
//!   `scripts/build-miyoo.sh`). Ses propres dépendances (EGL, GLES…) ne sont
//!   présentes que sur la console, d'où `--allow-shlib-undefined`.

fn main() {
    println!("cargo:rerun-if-env-changed=MIYOO_LIB_DIR");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("apple") {
        println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
        println!("cargo:rustc-link-search=native=/usr/local/lib");
    }
    if target.starts_with("armv7") {
        if let Some(dir) = std::env::var_os("MIYOO_LIB_DIR") {
            println!("cargo:rustc-link-search=native={}", dir.to_string_lossy());
        }
        println!("cargo:rustc-link-arg-bins=-Wl,--allow-shlib-undefined");
    }
}
