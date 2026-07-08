# bkgrnd — Android app (native, sideloaded APK)

Native Android client (Kotlin + Compose + Media3/ExoPlayer). Streams from the
same relay as the PWA/iOS, per-user login. **Phone-only v1** (no Play Store, no
Android Auto, no Spotify — those are deferred). Audio reaches the car over
**Bluetooth**.

## Status: scaffold
Committed as a starting point — **build/iterate in Android Studio** (this repo
was scaffolded on a Mac without a JDK/Gradle, so it hasn't been compiled yet).

Files:
- `RelayClient.kt` — relay API + session. `login()` stores the token (relay
  returns it in the body, commit `a274bee`) and sends it back as
  `Cookie: bkgrnd_session=…`. Library dedups by YouTube video id (client parity).
- `PlaybackService.kt` — Media3 `MediaSessionService` + ExoPlayer; injects the
  cookie on the stream fetch. Lock-screen/notification controls for free.
- `MainActivity.kt` — Compose: login → library list → tap to play via a
  `MediaController`. Minimal on purpose; add a player screen, search, bookmark.

## Build
1. Open `android/` in **Android Studio** (it will generate the Gradle wrapper +
   let you install the SDK/JDK it wants). Or from CLI with a JDK 17 + Gradle:
   `gradle wrapper && ./gradlew assembleRelease`.
2. Output: `app/build/outputs/apk/release/app-release.apk` — sideload it
   (`adb install -r app-release.apk`).
3. Sign in with a relay user (e.g. `ethan`), browse Streams, tap to play.

## Known first-build to-dos
- Add a launcher icon (`res/mipmap-*/ic_launcher`) — Studio's asset wizard.
- Pin versions if Studio flags newer AGP/Kotlin/Compose/Media3.
- The cookie header is read when `PlaybackService` starts; fine because playback
  begins after login. If you add logout→relogin without restart, refresh the
  data-source headers.
- Request `POST_NOTIFICATIONS` at runtime on Android 13+ for the media notification.

## Deferred (not in v1)
Android Auto (`MediaLibraryService` + browse tree + Play Store + Google review),
Spotify, search/bookmark parity polish. The relay's native-auth change is
committed but **not yet deployed** — deploy before the app talks to prod.
