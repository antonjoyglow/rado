fn main() {
    // include_image! embeds the tray icon without telling cargo to watch it.
    println!("cargo:rerun-if-changed=icons/tray.png");
    tauri_build::build()
}
