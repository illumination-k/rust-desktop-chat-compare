fn main() {
    // `generate_context!` requires `frontendDist` to exist. Create it so plain
    // `cargo build`/`clippy` work without the frontend; `pnpm tauri build` fills it.
    let dist = std::path::Path::new("../dist");
    if !dist.join("index.html").exists() {
        println!("cargo::warning=frontend not built; run `pnpm --filter chat-tauri build`");
        let _ = std::fs::create_dir_all(dist);
    }
    tauri_build::build();
}
