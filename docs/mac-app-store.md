# The Mac App Store build

The App Store build is the same Rust Mac app (`crates/tandem-desktop`). The difference is that it runs
in Apple's App Sandbox: it can read and write only the files a person picks in an open or save panel,
opens from Finder, or drops on the window, and that access ends when the app quits. The .dmg build is
not sandboxed and is unchanged.

## What the app does differently inside the sandbox

Each of these applies only when `APP_SANDBOX_CONTAINER_ID` is set, which macOS does for a sandboxed
process.

- **Saving.** A save normally writes a hidden temp file next to the PDF, checks it, and renames it
  over the PDF. The sandbox does not let the app create files in the PDF's folder. Instead, the temp
  file goes in a folder the system lends on the same volume (`NSItemReplacementDirectory`,
  `crates/tandem-desktop/src/sandbox.rs`) and is renamed over the PDF from there
  (`safe_write::set_temp_folder`). The sandbox also refuses to open the folder, so after the rename
  the saved file itself is flushed to disk.
- **Recent files.** The start screen and File ▸ Open Recent open files by path, which the sandbox
  refuses after a relaunch. Every recent file therefore keeps a security-scoped bookmark in
  `<user data>/bookmarks.json`, and opening a file by path first uses its bookmark to get access
  back. Clear Menu deletes the bookmarks.
- **User data.** The app's data lives in its container
  (`~/Library/Containers/com.shiro-legacy.tandem-pdf/Data/Library/Application Support/Tandem PDF`).
  Recent files, trusted certificates and the drawn signature from a .dmg install do not carry over.
- **Second launches.** Outside the sandbox, a second launch from the command line hands its files
  to the running app over a loopback socket. The sandbox allows no sockets, so this hand-over is
  off. The app still starts, and Finder sends files to the running app by itself.
- **Files it may not read.** A file the sandbox refuses (for example, a recent file whose bookmark
  no longer works) gets "Tandem PDF doesn't have permission to open …", with a pointer to
  File ▸ Open…. Every build shows this for a file without read permission.
- **Links.** Links open through `NSWorkspace` instead of `/usr/bin/open`, in every build.
- **Command-line modes.** `--mcp`, `--drive` and `--walk` still run, but they can reach only files
  inside the container, plus files the person has opened. In a sandboxed walk, `shot` steps fail
  (WebKit's snapshot is refused), while `state` steps work.

Entitlements (`crates/tandem-desktop/entitlements-appstore.plist`): the App Sandbox, read-write
access to user-selected files, app-scoped bookmarks, printing, and outgoing network connections. No camera
or microphone. The network entitlement is there only for WebKit, which draws the window: without it,
WebKit's networking process quits at launch ("does not have permission to communicate with network
resources") and the window stays blank, as 0.1.3 build 2 did. The app itself has no networking code.

## Trying the sandboxed app on this Mac

```bash
MAS=local APPLE_SIGNING_IDENTITY="Apple Development: Name (ID)" scripts/package-mac.sh
```

This builds `crates/target/release/bundle/macos/Tandem PDF.app`, signed with the sandbox
entitlements. It has no provisioning profile, so it runs only on this Mac. With no identity, the app is
signed ad hoc instead.

## Setup that only the account owner can do (once)

1. Register the App ID `com.shiro-legacy.tandem-pdf`
   (developer.apple.com ▸ Certificates, IDs & Profiles ▸ Identifiers). The bundle ID cannot change
   after the first upload.
2. Create two certificates: **Apple Distribution** and **Mac Installer Distribution**. The easiest way
   is Xcode ▸ Settings ▸ Accounts ▸ Manage Certificates.
3. Create a **Mac App Store** provisioning profile for that App ID and download it.
4. In App Store Connect:
   - accept the agreements;
   - create the app (name, primary language, the bundle ID above, an SKU);
   - set a price;
   - add a privacy policy URL (required even though nothing is collected);
   - answer App Privacy with "Data Not Collected";
   - add screenshots and a description.

## Making an upload

```bash
MAS=1 \
APPLE_PROVISIONING_PROFILE=path/to/Tandem_PDF_App_Store.provisionprofile \
APPLE_SIGNING_IDENTITY="Apple Distribution: Name (TEAMID)" \
APPLE_INSTALLER_IDENTITY="3rd Party Mac Developer Installer: Name (TEAMID)" \
BUILD_NUMBER=1 \
scripts/package-mac.sh
```

- The script copies the application and team identifiers from the profile into the entitlements.
- Every Mac build also gets an asset catalog (`Resources/Assets.car`, named by `CFBundleIconName`), built
  from `icons/icon.icns` with Xcode's `actool`. App Store Connect and Transporter show the icon from
  there. `CFBundleIconFile` stays the full `icon.icns`, since `actool`'s own `AppIcon.icns` stops at 256 px.
  It embeds the profile in the app and signs everything.
- It writes `crates/target/universal-apple-darwin/release/bundle/appstore/Tandem PDF_<version>_<build>.pkg`.
  Nothing is uploaded.
- `BUILD_NUMBER` must be higher than the previous upload's. The version stays the one in
  `tauri.conf.json`.
- Upload the .pkg with Apple's Transporter app, or with
  `xcrun altool --upload-app -t macos -f <pkg> --apiKey <key id> --apiIssuer <issuer id>`.
- App Store Connect asks about export compliance (encryption) for each build. The answer is the
  owner's to give.

An upload (`MAS=1`) is a universal app for Apple silicon and Intel Macs (`universal-apple-darwin`,
about 71 MB instead of 37 MB). It needs `rustup target add x86_64-apple-darwin` once, and takes
about twice as long to build. `UNIVERSAL=0` builds for this Mac only, and `UNIVERSAL=1` makes any
other build universal too. A universal build's output goes to
`crates/target/universal-apple-darwin/release/bundle/` instead of `crates/target/release/bundle/`.
To try the Intel half on an Apple silicon Mac, start it under Rosetta: `open --arch x86_64 -a <app>`.
