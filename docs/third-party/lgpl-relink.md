# LGPL components in the macOS package and how to relink `libffmpeg.dylib`

Owner: ds2. Written 2026-09-15 for `out/PDF4All-P0-darwin-arm64`. Facts and verified URLs only,
no legal conclusions and no licence decision for the app.

> **Retired with Electron (2026-10-19).** The desktop app is now Rust and Tauri: it ships no Electron, Chromium or
> `libffmpeg.dylib`, so this record describes only the old Electron packages. The macOS and Windows apps link the
> system's own web view (WKWebView, WebView2) and ship no LGPL code. The LGPL code that ships now is in the Linux
> AppImage only: the system libraries it bundles (GTK 3, GLib, Pango, Cairo, HarfBuzz, gdk-pixbuf, WebKitGTK 4.1,
> libsoup 3 and others; mostly LGPL-2.1-or-later), dynamically linked and replaceable, with their licence texts and
> Debian package versions in `usr/lib/Tandem PDF/licenses/system-libraries` (collected by `scripts/package-linux.sh`).
> The `.deb` bundles none of them (they are its dependencies). Whether the AppImage also needs a written source offer is
> an open owner question (`docs/electron-retirement-report.md`).

Evidence tags: **[V-bin]** read out of the packaged bundle, **[V-url]** URL fetched today,
**[V-tree]** read out of `node_modules/electron`, **[V-run]** printed by the packaged Electron
runtime, **[?]** uncertain.

## 1. Which files in the package are LGPL

| File | Licence evidence | Where |
|---|---|---|
| `PDF4All-P0.app/Contents/Frameworks/Electron Framework.framework/Versions/A/Libraries/libffmpeg.dylib` | **LGPL-2.1-or-later.** 2,317,968 B, arm64 Mach-O dylib, sha256 `fe2fa345bdb93b193f26bade263b769d05343536b47bd6bbeaf1e28ac3572904`; embedded strings: `FFmpeg version git-2026-07-15-6cfe2122b0`, `libavcodec license: LGPL version 2.1 or later`, the same for `libavformat` and `libavutil`; **no** GPL string [V-bin] | macOS package only |
| same file in `out/PDF4All-P0-win32-x64` as `ffmpeg.dll` | same upstream component, not inspected here [V-bin: `p0-package-license-check.md` covers win only partially] | Windows package |

**This is the only LGPL binary in the bundle.** The other bundled dylib,
`…/Libraries/libvk_swiftshader.dylib` (15.8 MB), is SwiftShader under **Apache-2.0**, not LGPL
[V-bin; `p0-package-license-check.md` §1].

### Listings in the Chromium aggregate that do not correspond to shipped code

`out/PDF4All-P0-darwin-arm64/LICENSES.chromium.html` (19.2 MB, sitting *beside* the `.app`) has 779
licence titles; **42** of those bodies mention the LGPL [V-bin]. Only one of them was traced so
far, because it was the one the earlier audit flagged:

- **`Braille Translation Library` (liblouis, LGPL-3.0+)** — the aggregate carries its notice, but
  a search of the whole `PDF4All-P0.app` for `liblouis` and `lou_` symbols returns **0 hits**, so
  no liblouis code is present in this package [V-bin].

The remaining LGPL-mentioning titles (`ffmpeg`, `glibc`, `libsecret`, `libbrlapi`,
`Netscape Portable Runtime (NSPR)`, several `symphonia-*` Rust crates, `hyphenation-patterns`,
`hunspell dictionaries`, the Android `play-services-*` family, …) have **not** been mapped to
binaries one by one. Treat "which of these are actually compiled in" as **[?] unresolved** rather
than answered — the Chromium aggregate is generated from Chromium's full source tree, not from the
contents of this Electron build.

## 2. Where the source and the build instructions live

| Item | Value |
|---|---|
| Electron release | `v44.4.0`; track/tree: `https://github.com/electron/electron/tree/v44.4.0`, source archive: `https://github.com/electron/electron/archive/refs/tags/v44.4.0.tar.gz` (19,764,833 B) [V-url] |
| Electron build instructions (version-pinned) | `https://github.com/electron/electron/blob/v44.4.0/docs/development/build-instructions-gn.md`; rendered: `https://www.electronjs.org/docs/latest/development/build-instructions-gn` [V-url] |
| Chromium tag shipped inside that Electron | `152.0.7977.78` [V-run: `chrome` from the packaged runtime] |
| Chromium source | `https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.78` [V-url] |
| ffmpeg source inside Chromium | `https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.78/third_party/ffmpeg` [V-url], plus Chromium's `src/third_party/ffmpeg/README.chromium` for the local patch set |
| Versions reported by the shipped runtime | `electron 44.4.0`, `chrome 152.0.7977.78`, `node 24.21.0`, `v8 15.2.124.19-electron.0`, `icu 78.2`, `modules 149` — printed with `ELECTRON_RUN_AS_NODE=1 <app>/Contents/MacOS/PDF4All-P0 -p "JSON.stringify(process.versions)"` [V-run] |
| ffmpeg version inside the dylib | upstream commit string `git-2026-07-15-6cfe2122b0` (Chromium pins its own ffmpeg revision; that revision is what `third_party/ffmpeg` at the tag above contains) [V-bin] |

## 3. The LGPL text itself

The **complete** ffmpeg licence text — including "Most files in FFmpeg are under the GNU Lesser
General Public License version 2.1 or later (LGPL v2.1+)", the LGPL‑v3 section, and the LGPL-2.1
body with the clause *"b) Use a suitable shared library mechanism for linking with the Library"*
and *"complete object files to the recipients, so that they can relink them"* — is inside
`LICENSES.chromium.html` under the entry `<span class="title">ffmpeg</span>` (that block is ~48 KB)
[V-bin].

There is **no standalone LGPL text file** in `docs/third-party/`, and that aggregate is **not
inside the `.app`** — it sits next to it in the output directory. A recipient handed only
`PDF4All-P0.app` therefore gets no LGPL text at all. That is finding **F2** in
`.quad/shared/p0/p0-package-license-check.md` and is a packaging decision for whoever prepares a
distribution; it is not fixed here.

## 4. Replacing (relinking) `libffmpeg.dylib`

Mechanically it is a file swap, verified [V-bin]:

1. The Electron Framework binary carries an ordinary dynamic link,
   `LC_LOAD_DYLIB @rpath/libffmpeg.dylib` (read out of its Mach-O load commands; 69 dylib load
   commands in total). ffmpeg is **not** statically linked into the framework. Its single
   `LC_RPATH` is `@loader_path/Libraries`, and the binary's loader path is
   `…/Versions/A/Electron Framework`, so that `@rpath` resolves to `…/Versions/A/Libraries`.
2. The file it resolves to is
   `PDF4All-P0.app/Contents/Frameworks/Electron Framework.framework/Versions/A/Libraries/libffmpeg.dylib`.
   Replacing that one file, keeping the name, leaves every other link intact.
3. This package is built with `osxSign: false` (`scripts/package.mjs`), so it is unsigned and a
   replacement needs no re-signing. On any signed/notarized build, swapping a dylib inside the
   bundle invalidates the signature and the bundle has to be re-signed — a property of code
   signing, not of this project.
4. Electron publishes a prebuilt replacement for exactly this version — **the drop-in artifact**:

| Item | Value |
|---|---|
| URL | `https://github.com/electron/electron/releases/download/v44.4.0/ffmpeg-v44.4.0-darwin-arm64.zip` (200, 606,501 B) [V-url] |
| sha256 of the zip | `5afa98fa554c862d67290429031a4cce5c7fb5c22c4e068cb086c0ff3404a439` |
| Contents | exactly one file, `libffmpeg.dylib`, 1,260,608 B, arm64 Mach-O dylib, sha256 `a6ba6183b6f9b1748a4725dc1e0b1b3c2b9fa7b1de78ae55221a47fb7c576f39` |
| Licence strings in it | identical to the bundled one: same `FFmpeg version git-2026-07-15-6cfe2122b0`, `libavcodec/libavformat/libavutil license: LGPL version 2.1 or later`, no GPL string [V-bin] |
| Difference | the two files are **not** byte-identical (2,317,968 B bundled vs 1,260,608 B in the release asset) although they report the same ffmpeg commit. Why they differ is **[?] not established here**; Electron ships more than one ffmpeg variant and we did not verify which configuration each one is. |

A rebuild-from-source route exists too (Electron's GN build instructions above, with Chromium's
`third_party/ffmpeg` at tag `152.0.7977.78`), but nothing on this project has attempted it.

## 5. What is still open

- The LGPL notice for *our* distribution is not written anywhere in the package; what ships today
  is Chromium's aggregate, beside the `.app`, naming the upstream project. See §3.
- Whether any LGPL-mentioning component other than ffmpeg is actually linked into
  `Electron Framework` is unresolved (§1) — the aggregate is not a component list for this build.
- The two ffmpeg variants are not explained (§4) and no decision has been made about which one a
  distribution should carry.
- The Windows package's `ffmpeg.dll` was not inspected in this pass, and that package additionally
  carries the proprietary `d3dcompiler_47.dll` (finding F1).

## 6. Re-check commands

```bash
APP=out/PDF4All-P0-darwin-arm64
F="$APP/PDF4All-P0.app/Contents/Frameworks/Electron Framework.framework/Versions/A/Libraries/libffmpeg.dylib"
shasum -a 256 "$F"                      # fe2fa345…2904
python3 -c "import re,sys;b=open(sys.argv[1],'rb').read();print(sorted({x.decode() for x in re.findall(rb'[\x20-\x7e]{6,}',b) if b'LGPL' in x}))" "$F"
grep -rl "lou_getTable\|liblouis" "$APP/PDF4All-P0.app/" ; echo "exit 1 = no liblouis code"
ELECTRON_RUN_AS_NODE=1 "$APP/PDF4All-P0.app/Contents/MacOS/PDF4All-P0" -p "JSON.stringify(process.versions)"
curl -sSLo /tmp/ffmpeg.zip https://github.com/electron/electron/releases/download/v44.4.0/ffmpeg-v44.4.0-darwin-arm64.zip
shasum -a 256 /tmp/ffmpeg.zip           # 5afa98fa…a439
```
