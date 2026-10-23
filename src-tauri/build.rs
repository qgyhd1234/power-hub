fn main() {
    tauri_build::build();
    vendor_webview2_loader();
}

/// 把入库的 WebView2Loader.dll 投放到产物目录。
/// Windows 按可执行文件所在目录搜索 dll：cargo run 的产物在 target/<profile>/，
/// cargo test 的在 target/<profile>/deps/，缺一处对应场景就起不来。
fn vendor_webview2_loader() {
    const DLL: &str = "WebView2Loader.dll";
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(DLL);
    if !src.exists() {
        return;
    }
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    // OUT_DIR 形如 <target>/<profile>/build/<pkg>-<hash>/out，向上三级即 <target>/<profile>
    let Some(profile_dir) = std::path::Path::new(&out_dir).ancestors().nth(3) else {
        return;
    };
    let mut destinations = vec![profile_dir.join(DLL)];
    let deps_dir = profile_dir.join("deps");
    if deps_dir.is_dir() {
        destinations.push(deps_dir.join(DLL));
    }
    for dst in destinations {
        // 投放失败不炸构建，但要留痕——否则症状（exe 起不来）无从排查
        if std::fs::copy(&src, &dst).is_err() {
            eprintln!("warning: WebView2Loader.dll 投放失败：{}", dst.display());
        }
    }
}
