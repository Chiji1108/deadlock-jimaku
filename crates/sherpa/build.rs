//! 実行環境のライブラリ（ONNX Runtime と sherpa-onnx）は、実行ファイルの隣に置く。
//! sherpa-onnx-sys の付ける rpath は、依存の部品の分なので、この実行ファイルには届かない
fn main() {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        // テストの実行ファイル（deps の中）からも、1 つ上のフォルダのライブラリを探す
        Ok("macos") => {
            println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
            println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/..");
        }
        Ok("linux") => {
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/..");
        }
        _ => {}
    }
}
