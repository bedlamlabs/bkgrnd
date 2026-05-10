# iOS App (bkgrnd)

Source files for an iOS SwiftUI app that:
- Shows **Recent Mixes** (grid)
- Shows **Now Playing**
- Shows **Search**
- Streams audio from WOPR and supports background playback

This repo does not include a generated `.xcodeproj` yet.

## Create the Xcode project

1. In Xcode: **File → New → Project → iOS → App**
2. Product Name: `bkgrnd`
3. Interface: `SwiftUI`
4. Language: `Swift`
5. Copy the files from `ios/bkgrnd/Sources/` into the project.

## Capabilities

In the target settings:
- Enable **Background Modes** → check **Audio, AirPlay, and Picture in Picture**

## WOPR config

Set these in the app’s Settings screen:
- Base URL (default: `http://worp.thriveos.pro:8080`)
  - Use `http://worp.thriveos.pro:808` if you expose WOPR on port 808 (recommended).
- Optional Bearer token (if configured server-side)
