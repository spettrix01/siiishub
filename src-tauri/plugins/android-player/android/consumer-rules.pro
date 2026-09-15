# Applied to the app build. Tauri finds the plugin through reflection and
# libmpv calls back into MPVLib from JNI by name.
-keep class dev.siiis.siiishub.player.** { *; }
-keep class dev.jdtech.mpv.** { *; }
