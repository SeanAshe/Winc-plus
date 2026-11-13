#!/usr/bin/env bash
# Verify what a macOS release ships: the .app inside the final DMG. Run after the last
# repackaging and before anything is uploaded.
#
# The app binary and the terminal helper must be built for --arch and must not
# require a newer macOS than the bundle declares (LSMinimumSystemVersion), so
# the app never installs on a system where either program cannot start. The bundle must pass codesign; with a Developer ID
# (APPLE_TEAM_ID, from signing-identity.sh) the app and the DMG must also be
# signed by that team, carry a stapled notarization ticket and pass Gatekeeper.
#
# Usage: verify.sh --arch <arm64|x86_64> <file.dmg>...
set -euo pipefail

arch=""
artifacts=()
while (($# > 0)); do
  case "$1" in
    --arch) arch="${2:?--arch needs a value}"; shift 2 ;;
    *) artifacts+=("$1"); shift ;;
  esac
done
[[ "$arch" == arm64 || "$arch" == x86_64 ]] || { echo "usage: verify.sh --arch <arm64|x86_64> <file.dmg>..." >&2; exit 2; }
((${#artifacts[@]} > 0)) || { echo "verify: no artifact given" >&2; exit 2; }

signed=false
[[ -n "${APPLE_TEAM_ID:-}" ]] && signed=true

mounts=()
# ${a[@]+"${a[@]}"}: bash 3.2 (macOS, CI runners included) treats an empty
# array as unset under `set -u`.
cleanup() {
  local path
  for path in ${mounts[@]+"${mounts[@]}"}; do
    hdiutil detach -quiet "$path" || hdiutil detach -force -quiet "$path" || true
    rmdir "$path" 2>/dev/null || true
  done
}
trap cleanup EXIT

fail() {
  echo "verify: $*" >&2
  exit 1
}

# version_le A B: true when dotted version A <= B (10.15 < 12.0 < 12.0.1).
version_le() {
  local -a a b
  local i x y
  IFS=. read -r -a a <<<"$1"
  IFS=. read -r -a b <<<"$2"
  for ((i = 0; i < ${#a[@]} || i < ${#b[@]}; i++)); do
    x=${a[i]:-0} y=${b[i]:-0}
    ((10#$x < 10#$y)) && return 0
    ((10#$x > 10#$y)) && return 1
  done
  return 0
}

# Gatekeeper must name the notarized Developer ID as the source. A host with
# assessments disabled accepts anything, so that answer proves nothing.
assess() {
  local output
  output="$(spctl --assess --verbose=2 "$@" 2>&1)" || fail "Gatekeeper rejected ${*: -1}: $output"
  if [[ "$output" == *"override=security disabled"* ]]; then
    echo "warning: Gatekeeper assessments are disabled on this host; relying on the stapled ticket" >&2
  elif [[ "$output" != *"source=Notarized Developer ID"* ]]; then
    fail "Gatekeeper accepted ${*: -1} but not as a notarized Developer ID: $output"
  fi
}

check_team() {
  local team
  # An unsigned file makes codesign fail; that should end in the message below.
  team="$(codesign -dv "$1" 2>&1 | sed -n 's/^TeamIdentifier=//p' || true)"
  [[ "$team" == "$APPLE_TEAM_ID" ]] || fail "$1 is signed by team '${team:-none}', expected $APPLE_TEAM_ID"
}

verify_dmg_signature() {
  local requirement
  # Disk images can report TeamIdentifier=not set even with a valid Developer
  # ID signature. Validate the signing certificate's OU (the Team ID) instead
  # of the CodeDirectory field, and require Apple's Developer ID chain.
  [[ "$APPLE_TEAM_ID" =~ ^[A-Z0-9]{10}$ ]] || fail "APPLE_TEAM_ID must contain 10 uppercase letters or digits"
  requirement="=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists \
and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"$APPLE_TEAM_ID\""
  codesign --verify --strict --verbose=2 --test-requirement "$requirement" "$1" \
    || fail "$1 must be signed by a Developer ID Application certificate for team $APPLE_TEAM_ID"
}

verify_app() {
  local app="$1" plist minimum executable binary archs minos
  plist="$app/Contents/Info.plist"
  minimum="$(plutil -extract LSMinimumSystemVersion raw -o - "$plist")" || fail "$plist has no LSMinimumSystemVersion"
  executable="$(plutil -extract CFBundleExecutable raw -o - "$plist")" || fail "$plist has no CFBundleExecutable"

  # The app itself and the terminal helper (the terminal helper).
  for binary in "$app/Contents/MacOS/$executable" "$app/Contents/MacOS/winc-pty"; do
    [[ -f "$binary" ]] || fail "$binary is missing"
    archs="$(lipo -archs "$binary" 2>/dev/null)" || fail "$binary is not a Mach-O executable"
    [[ "$archs" == "$arch" ]] || fail "$binary is built for '$archs', expected $arch"
    minos="$(vtool -show-build "$binary" | awk '$1 == "minos" || $1 == "version" { print $2; exit }')" || fail "cannot read the build version of $binary"
    [[ -n "$minos" ]] || fail "$binary declares no minimum macOS"
    version_le "$minos" "$minimum" || fail "$binary requires macOS $minos, but the app declares $minimum"
    echo "  $(basename "$binary"): $archs, macOS $minos+"
  done

  # The remote server for each Linux architecture, uploaded to hosts on connection.
  for binary in winc-server-Linux-x86_64 winc-server-Linux-aarch64; do
    [[ -f "$app/Contents/Resources/$binary" ]] || fail "$binary is missing from Resources"
  done

  codesign --verify --deep --strict --verbose=2 "$app"
  if $signed; then
    check_team "$app"
    # Hardened runtime loads the embedded Sparkle framework dynamically; it
    # and its helpers must belong to the same Developer ID team as the app.
    check_team "$app/Contents/Frameworks/Sparkle.framework"
    while IFS= read -r -d '' nested; do
      if file -b "$nested" | grep -q Mach-O; then check_team "$nested"; fi
    done < <(find "$app/Contents/Frameworks" -type f -print0)
    xcrun stapler validate "$app"
    assess --type execute "$app"
  else
    echo "  ad-hoc signed, not notarized"
  fi
}

# Exactly one .app per artifact: a second one, or none, means the layout changed.
the_app() {
  local -a apps=("$1"/*.app)
  ((${#apps[@]} == 1)) && [[ -d "${apps[0]}" ]] || fail "expected one .app in $2, found ${#apps[@]}"
  printf '%s\n' "${apps[0]}"
}

shopt -s nullglob
for artifact in "${artifacts[@]}"; do
  [[ -f "$artifact" ]] || fail "$artifact does not exist"
  echo "--- $artifact"
  case "$artifact" in
    *.dmg)
      if $signed; then
        verify_dmg_signature "$artifact"
        xcrun stapler validate "$artifact"
        assess --type open --context context:primary-signature "$artifact"
      fi
      mount="$(mktemp -d)"
      mounts+=("$mount")
      hdiutil attach -nobrowse -noautoopen -readonly -quiet -mountpoint "$mount" "$artifact"
      app="$(the_app "$mount" "$artifact")"
      verify_app "$app"
      ;;
    *) fail "$artifact is not a .dmg" ;;
  esac
done
echo "verified ${#artifacts[@]} macOS artifact(s)"
