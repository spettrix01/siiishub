# Third-party components

SIIISHUB is released under the GNU General Public License v3.0 or later. Its
Rust dependencies are pinned in `src-tauri/Cargo.lock`, and all of them use
licenses compatible with the GPL. The packages published on the Releases page
also contain the prebuilt components listed below.

## Windows installer

| Component | Version | License | Source |
|---|---|---|---|
| libmpv, `libmpv-2.dll`, built with mpv-winbuild-cmake | mpv v0.41.0-524-g5921fe50b | GPL-2.0-or-later, parts under LGPL | [mpv at 5921fe50b](https://github.com/mpv-player/mpv/tree/5921fe50b), [build scripts](https://github.com/shinchiro/mpv-winbuild-cmake) |
| FFmpeg, linked into `libmpv-2.dll` | N-124056-gc92304f8c | LGPL-2.1-or-later or GPL, depending on the configuration | [FFmpeg at c92304f8c](https://github.com/FFmpeg/FFmpeg/tree/c92304f8c) |
| Mozilla CA certificate store, embedded in the executable to verify HTTPS streams | data of 13 August 2026, from curl.se | MPL-2.0 | [curl.se/docs/caextract.html](https://curl.se/docs/caextract.html) |

`libmpv-2.dll` also contains the libraries that mpv-winbuild-cmake links into
it, such as libass, libplacebo and OpenSSL, at the versions pinned by its build
scripts for that mpv revision.

## Android packages

| Component | Version | License | Source |
|---|---|---|---|
| libmpv-android, `dev.jdtech.mpv:libmpv` | 0.4.1 | MIT | [libmpv-android v0.4.1](https://github.com/jarnedemeulemeester/libmpv-android/tree/v0.4.1) |
| mpv | v0.39.0 | GPL-2.0-or-later | [mpv v0.39.0](https://github.com/mpv-player/mpv/tree/v0.39.0) |
| FFmpeg, configured with GPL components | n7.1 | GPL-3.0-or-later | [FFmpeg n7.1](https://github.com/FFmpeg/FFmpeg/tree/n7.1) |
| Mbed TLS | 3.6.1 | Apache-2.0 or GPL-2.0-or-later | [Mbed TLS v3.6.1](https://github.com/Mbed-TLS/mbedtls/tree/v3.6.1) |

The libmpv-android build scripts at v0.4.1 pin the exact versions and build
options of these libraries and of the others they include, such as libass.

## Linux package

The .deb package uses the libmpv and FFmpeg of the distribution and bundles
neither.

## Other assets

- The interface fonts, Bricolage Grotesque, Geist and Geist Mono, are loaded
  from Google Fonts at runtime under the SIL Open Font License and are not
  bundled.
- The TMDB logo in `dist/img/tmdb-logo.svg` is the official logo, used as the
  TMDB API terms require. Movie and series data and images come from the TMDB
  API at runtime.
- The screenshot of the Android player shows the Sintel trailer,
  © Blender Foundation, under CC BY 3.0.
