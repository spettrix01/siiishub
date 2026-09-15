// No webview-facing commands: the app calls the plugin from Rust, so there
// are no permissions to generate. The helper still links the Android library
// (android/) into the app's Gradle project.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
