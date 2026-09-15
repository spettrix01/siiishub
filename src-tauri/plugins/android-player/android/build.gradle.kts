plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "dev.siiis.siiishub.player"
    compileSdk = 36

    defaultConfig {
        // libmpv (dev.jdtech.mpv) is built for Android 8.0+.
        minSdk = 26

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        consumerProguardFiles("consumer-rules.pro")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.9.0")
    implementation("com.fasterxml.jackson.core:jackson-databind:2.15.3")
    // Prebuilt libmpv (+ffmpeg, libass) with the MPVLib JNI binding, from
    // the libmpv-android project used by Findroid.
    implementation("dev.jdtech.mpv:libmpv:0.4.1")
    implementation(project(":tauri-android"))
}
