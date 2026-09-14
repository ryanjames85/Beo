plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "org.beo.diagnostics"
    // Matches Beo's newest hand-verified system-image level (App.tsx's
    // VERIFIED_API_LEVELS) — this app is only ever sideloaded onto Beo's
    // own devices, so there's no reason to compile against anything newer.
    compileSdk = 36

    defaultConfig {
        applicationId = "org.beo.diagnostics"
        // Matches the lowest level in VERIFIED_API_LEVELS's own fallback
        // range — this should install on any device Beo can create.
        minSdk = 28
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        viewBinding = false
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
}
