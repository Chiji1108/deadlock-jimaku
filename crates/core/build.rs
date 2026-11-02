fn main() {
    // rust-embed は配信元フォルダのパスをコンパイル時に確定するので、
    // frontend/dist が作られたり更新されたりしたら作り直す
    println!("cargo:rerun-if-changed=../../frontend/dist");
}
