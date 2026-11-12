plugins {
    id("com.android.application")
}

android {
    namespace = "dev.unko.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.unko.app"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        // cargo ndk writes libunko_app.so here; add more ABIs as needed.
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    sourceSets["main"].jniLibs.srcDir("src/main/jniLibs")

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
}
