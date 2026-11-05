//! 実行環境のライブラリは実行ファイルの隣に置く
fn main() {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path"),
        Ok("linux") => println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN"),
        _ => {}
    }
}
