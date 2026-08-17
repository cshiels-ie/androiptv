import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "dev.androiptv.cast"
    compileSdk = 34

    defaultConfig {
        // Must be <= the host app's minSdk (default 21) or the Gradle
        // merge fails. The Cast SDK floor is 21.
        minSdk = 21

        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

dependencies {
    implementation(project(":tauri-android"))
    implementation("androidx.mediarouter:mediarouter:1.7.0")
    // 21.7.0 does not exist (metadata: ...21.4.0, 21.5.0, 22.x). 21.4.0 is
    // the newest line compiled against SDK 34 — anything newer has
    // minCompileSdk 35+, which fails the AAR-metadata check against our
    // compileSdk 34.
    implementation("com.google.android.gms:play-services-cast-framework:21.4.0")
}
