# Tauri finds the plugin class and its @Command methods through reflection.
-keep class dev.siiis.siiishub.player.** { *; }
# libmpv calls back into MPVLib from JNI by name.
-keep class dev.jdtech.mpv.** { *; }
