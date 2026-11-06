# Releasing winc

Every merge into `main` is a release. It creates the annotated tag `v<version>`, explicitly
dispatches the Release workflow on that tag, builds the Windows x64 target, and assembles one
**draft** GitHub Release. A maintainer reviews and publishes the draft. No platform job uploads
directly to a Release.

Raise the version on `dev` before opening the pull request into `main`: its "Release version" check
fails while the version is already tagged, and a push to `main` with such a version fails instead
of releasing it again. To retry a failed build, rerun the Release workflow on the existing tag.

## Layout

| Path | Contents |
| --- | --- |
| `.github/workflows/release-tag.yml` | Tags each merge into `main` and starts the Release workflow |
| `.github/workflows/release.yml` | Entry point: prepare, the Windows build, and the draft |
| `.github/workflows/release-{prepare,windows}.yml` | Reusable jobs called by `release.yml` |
| `.github/actions/build-setup/` | Shared toolchain and cache setup; pins the release Rust version |
| `release/windows/` | `package.ps1` (NSIS, MSI, portable zip), `sign.ps1`, `verify.ps1`, installer templates |
| `release/common/` | Version, build config, update signing, asset staging, and publication |

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
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Optional Windows Authenticode provider; certificate is a base64-encoded PFX |

Without a Windows certificate, installers are unsigned. Updater signatures remain mandatory.

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
| Windows x86_64 | `windows-x64-setup.exe`, `windows-x64.msi`, `windows-x64-portable.zip` |

Packages include the terminal helper.

Each release also has one signed appcast for the in-app updater, `appcast-windows.xml`. It lists
the NSIS and MSI installers, each with its EdDSA signature; the portable zip is a download only.
`SHA256SUMS` covers every file.

### Update channel

The in-app updater reads this repository's GitHub Releases. The channel URL built into the app is
`https://github.com/SeanAshe/Winc-plus/releases/latest/download` for both releases and prereleases
(`releases/latest` is the newest non-prerelease).

The app reads `<channel URL>/appcast-windows.xml`, and the appcast's download URLs are
`<channel URL>/<file name>`. Publishing a GitHub Release that contains the appcast and the
packages it lists ships the update. The appcast is signed, so those assets must be uploaded
unchanged. The channel URL is defined once, in `channelBase` in `release/common/config.mjs`.

Windows NSIS installations are per-user; MSI installations are per-machine and use the fixed
UpgradeCode `00FDC6A5-D527-59AE-B4A5-E051B25222F3`. Do not change it between releases. Each installer
writes `winc-installation` beside the executable. The updater selects the matching NSIS/MSI
enclosure, verifies its signature, waits for the app to exit, installs silently, and restarts only
when requested. MSI updates request elevation when required. Portable Windows installations
continue to use manual updates.

## Verification and publication

Before uploading, the Windows job verifies its packages: it checks signatures and exercises
NSIS/MSI installation and the update launcher using paths containing spaces.

The publication job requires the verified Windows artifacts. It refuses missing,
duplicate or unexpected assets, multiple drafts for the tag, and replacing an already published
Release. Retrying a draft replaces the same expected asset names. Publishing remains a manual
action after reviewing the draft, release notes and checksums.
