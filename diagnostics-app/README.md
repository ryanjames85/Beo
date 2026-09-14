# Beo Diagnostics

A tiny, sideloadable Android app for testing emulator health from inside a
Beo device — audio, GPU rendering, network, and storage — with a controlled,
repeatable signal for each instead of relying on subjective impressions
("YouTube sounds different"). Built when debugging real audio-glitching
reports made that gap obvious.

Plain Kotlin + classic Android Views on purpose — no Compose, no DI, no
database. This is a lightweight dev tool, not a production app, and a
heavier stack would only mean slower builds for zero benefit here.

## This is NOT part of Beo's own build

Beo's own bundled SDK deliberately has no build-tools (kept minimal, see
the main README). This app is built once, separately, using any full
Android SDK with build-tools installed (Android Studio's own SDK works
fine), and the compiled APK is committed into the main repo as a binary
resource (`resources/beo-diagnostics.apk`) — the same relationship a
vendored dependency or the frontend's `dist/` build output has. Rebuild it
only when this app's own code changes, not as part of Beo's normal
`cargo build`/`npm run build`.

## Rebuilding

```bash
# Point at any Android SDK with build-tools (not Beo's own):
export ANDROID_HOME="/path/to/an/Android/Sdk"

./gradlew assembleDebug
```

Output: `app/build/outputs/apk/debug/app-debug.apk`. Copy that to
`../resources/beo-diagnostics.apk` and commit it.

No release signing — this is only ever sideloaded via `adb install`, never
distributed anywhere that would need a real signing key.
