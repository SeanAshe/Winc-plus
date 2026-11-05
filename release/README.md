# Releasing winc

Every merge into `main` is a release. It creates the annotated tag `v<version>`, explicitly
dispatches the Release workflow on that tag, builds four native targets, and assembles one
**draft** GitHub Release. A maintainer reviews and publishes the draft. No platform job uploads
directly to a Release.

Raise the version on `dev` before opening the pull request into `main`: its "Release version" check
fails while the version is already tagged, and a push to `main` with such a version fails instead
of releasing it again. To retry a failed build, rerun the Release workflow on the existing tag.

## Layout

| Path | Contents |
| --- | --- |
| `.github/workflows/release-tag.yml` | Tags each merge into `main` and starts the Release workflow |
| `.github/workflows/release.yml` | Entry point: prepare, the four platform builds, and the draft |
| `.github/workflows/release-{prepare,macos,linux,windows}.yml` | Reusable jobs called by `release.yml` |
| `.github/actions/build-setup/` | Shared toolchain and cache setup; pins the release Rust version |
| `release/macos/` | `package.sh` (app, dmg, appcast), `verify.sh`, and Apple signing setup |
| `release/linux/` | `package.sh` (AppImage and deb), pinned packaging tools, desktop entry and AppRun |
| `release/windows/` | `package.ps1` (NSIS, MSI, portable zip), `sign.ps1`, `verify.ps1`, installer templates |
| `release/common/` | Shared by all platforms: version, build config, update signing, asset staging, publication |

## Version and source

The release version is `version` in the `[workspace.package]` section of `Cargo.toml`. To prepare
a version:

```sh
# Edit [workspace.package] version in Cargo.toml, then refresh the workspace entries in Cargo.lock.
cargo update --workspace
```

Commit `Cargo.toml` and `Cargo.lock` together, then merge into `main`. `cargo update --workspace`
changes only the workspace's own entries, never dependency versions. The Release workflow rejects
a `Cargo.lock` that does not match `Cargo.toml`, a tag that does not equal `v<version>`, a checkout
that differs from the workflow's immutable commit, or a tag that moved after the build began.

The Release workflow can also be dispatched manually. With `release_tag` set, a branch-context request
dispatches a new run on that existing tag. With no tag selected, a branch run builds downloadable
Actions artifacts and does not create a Release. An existing version tag is not recreated;
rerun the Release workflow on that tag to retry a failed build.

## Signing configuration

Apart from the update key, names are identical to DEEIX Chat. Store private values as repository or
organization **Secrets**, not Variables or committed files.

| Secrets | Purpose |
| --- | --- |
| `WINC_UPDATE_PRIVATE_KEY` | Required: contents of a current Sparkle exported key file, base64 encoding a 32-byte Ed25519 seed |
| `SIGN_AND_NOTARIZE_SCRIPT_URL`, `SIGN_AND_NOTARIZE_BASE_URL`, `SIGN_AND_NOTARIZE_API_TOKEN`, `SIGN_AND_NOTARIZE_HELPER_URL` | Optional external Apple signing provider; configure all four together |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` | Optional local Apple Developer ID provider; certificate is a base64-encoded P12 |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Optional Windows Authenticode provider; certificate is a base64-encoded PFX |

The four external signing Secrets already configured for Winc retain their original names and
meaning. A complete external provider takes priority over local Apple credentials. Partial provider
configuration fails; with no Apple provider, packages are ad-hoc signed. Without a Windows
certificate, installers are unsigned. Updater signatures remain mandatory on every platform.

`WINC_UPDATE_PRIVATE_KEY` is a **Sparkle seed** that signs every update package and appcast. Store
it in this repository's Secrets; it is never shared with other projects. With the pinned Sparkle
tools, generate a dedicated Winc key and export it using
`generate_keys --account com.deeix.winc`, followed by
`generate_keys --account com.deeix.winc -x <private-key-file>`. Use the exported file's contents as
the Secret; do not commit it. Older 96-byte Sparkle exports are rejected.
Keep this key across releases: an installed application trusts the public key embedded in its build.

The build derives `WINC_UPDATE_PUBLIC_KEY` from that seed and `WINC_UPDATE_CHANNELS` from the
version (see below). Neither needs a repository Variable.

## Artifacts and updates

| Platform | Release filenames, prefixed with `Winc-<version>-` |
| --- | --- |
| macOS Apple silicon | `macos-arm64.dmg` |
| macOS Intel | `macos-x64.dmg` |
| Linux x86_64 | `linux-x64.AppImage`, `linux-x64.deb` |
| Windows x86_64 | `windows-x64-setup.exe`, `windows-x64.msi`, `windows-x64-portable.zip` |

All packages include the terminal helper. macOS containers use the same final app and retain the
macOS 11 minimum. Linux packages are built on Ubuntu 24.04; the deb declares glibc 2.39 or newer.
Debian prerelease versions use `~` so the final release sorts after its betas.

Each release also has four signed appcasts for the in-app updater: `appcast-macos-arm64.xml`,
`appcast-macos-x86_64.xml`, `appcast-linux.xml`, and `appcast-windows.xml`. They list the dmg, the
AppImage, and the NSIS and MSI installers, each with its EdDSA signature; the deb and the portable
zip are downloads only. `SHA256SUMS` covers every file.

### Update channel

The in-app updater reads this repository's GitHub Releases. The channel URL built into the app is
`https://github.com/SeanAshe/Winc-plus/releases/latest/download` for both releases and prereleases
(`releases/latest` is the newest non-prerelease).

The app reads `<channel URL>/appcast-<platform>.xml`, and each appcast's download URLs are
`<channel URL>/<file name>`. Publishing a GitHub Release that contains the appcasts and the
packages they list ships the update. Appcasts are signed, so those assets must be uploaded
unchanged. The channel URL is defined once, in `channelBase` in `release/common/config.mjs`.

Windows NSIS installations are per-user; MSI installations are per-machine and use the fixed
UpgradeCode `00FDC6A5-D527-59AE-B4A5-E051B25222F3`. Do not change it between releases. Each installer
writes `winc-installation` beside the executable. The updater selects the matching NSIS/MSI
enclosure, verifies its signature, waits for the app to exit, installs silently, and restarts only
when requested. MSI updates request elevation when required. Portable Windows and deb installations
continue to use manual or package-manager updates.

## Verification and publication

Before uploading, native jobs verify their packages: macOS checks both container contents,
architectures, minimum system versions, signatures and notarization where configured; Linux checks
the AppImage for bundled Wayland libraries and inspects the deb; Windows checks signatures and
exercises NSIS/MSI installation and the update launcher using paths containing spaces.

The single publication job requires all four verified target artifacts. It refuses missing,
duplicate or unexpected assets, multiple drafts for the tag, and replacing an already published
Release. Retrying a draft replaces the same expected asset names. Publishing remains a manual
action after reviewing the draft, release notes and checksums.
