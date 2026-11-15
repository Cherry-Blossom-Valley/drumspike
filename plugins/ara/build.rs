//! bindgen on the ARA 2 API header (the spike's build.rs): `$ARA_API_DIR/ARAInterface.h`
//! into `$OUT_DIR/ara.rs`. `ARA_API_DIR` points at the `ARA_API` folder of the ARA SDK
//! 2.3.0 checkout that `check.sh deps` fetches.
use std::{env, path::PathBuf};

fn main() {
    let api = env::var("ARA_API_DIR").expect("ARA_API_DIR (run check.sh deps)");
    println!("cargo:rerun-if-env-changed=ARA_API_DIR");
    let b = bindgen::Builder::default()
        .header(format!("{api}/ARAInterface.h"))
        .allowlist_type("ARA.*")
        .allowlist_var("kARA.*")
        .prepend_enum_name(false)
        .layout_tests(true)
        .derive_default(true)
        .generate()
        .expect("bindgen");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    b.write_to_file(out.join("ara.rs")).unwrap();
}
