#!/usr/bin/env bash
set -euo pipefail

# Shared macOS codesign, DMG creation, Apple notarization, and stapler helper
# for Diving Desktop (`Diving.app`).
#
# Subcommands:
#   --preflight   Validate signing & notary environment variables and test Apple credentials
#   --package     Sign Diving.app, create & sign DMG, notarize, staple DMG + app, stage DMG & tar.gz
#   --cleanup     Remove temporary certificate/keychain artifacts

mode="${1:-}"
if [ -z "$mode" ]; then
  echo "Usage: $0 <--preflight|--package|--cleanup>" >&2
  exit 1
fi

require_env() {
  local name="$1"
  if [ -z "${!name:-}" ]; then
    echo "::error::Missing required environment variable: ${name}" >&2
    exit 1
  fi
}

codesign_with_retry() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if codesign "$@"; then
      echo "codesign succeeded on attempt ${attempt}"
      return 0
    fi
    if [ "$attempt" -eq 5 ]; then
      echo "::error::codesign still failing after ${attempt} attempts" >&2
      return 1
    fi
    echo "codesign attempt ${attempt} failed (timestamp server?); retrying in 15s..."
    sleep 15
  done
}

staple_with_retry() {
  local target="$1"
  local attempt
  for attempt in 1 2 3; do
    if xcrun stapler staple "$target"; then
      echo "stapled ${target} on attempt ${attempt}"
      return 0
    fi
    if [ "$attempt" -eq 3 ]; then
      echo "::error::xcrun stapler staple failed for ${target} after ${attempt} attempts" >&2
      return 1
    fi
    echo "stapler attempt ${attempt} for ${target} failed (propagation?); retrying in 15s..."
    sleep 15
  done
}

preflight() {
  require_env MACOS_CERTIFICATE
  require_env MACOS_CERTIFICATE_PWD
  require_env AC_USERNAME
  require_env AC_PASSWORD
  require_env AC_TEAM_ID

  xcrun notarytool history \
    --apple-id "$AC_USERNAME" \
    --password "$AC_PASSWORD" \
    --team-id "$AC_TEAM_ID" >/dev/null
  echo "Notary credentials OK"
}

cleanup() {
  local cert_path="${RUNNER_TEMP:-/tmp}/certificate.p12"
  local keychain_path="${KEYCHAIN_PATH:-${RUNNER_TEMP:-/tmp}/app-signing.keychain-db}"
  rm -f "$cert_path"
  security delete-keychain "$keychain_path" >/dev/null 2>&1 || rm -f "$keychain_path"
}

package_macos() {
  require_env APP_PATH
  require_env STAGE_DIR
  require_env DMG_NAME
  require_env TAR_NAME
  require_env MACOS_CERTIFICATE
  require_env MACOS_CERTIFICATE_PWD
  require_env AC_USERNAME
  require_env AC_PASSWORD
  require_env AC_TEAM_ID

  local vol_name="${VOL_NAME:-Diving Installer}"
  local tmp_root="${RUNNER_TEMP:-/tmp}"
  local keychain_path="${KEYCHAIN_PATH:-${tmp_root}/app-signing.keychain-db}"
  local cert_path="${tmp_root}/certificate.p12"

  trap 'rm -f "'"$cert_path"'"' EXIT

  if [ ! -d "$APP_PATH" ]; then
    echo "::error::Missing .app bundle at ${APP_PATH}" >&2
    exit 1
  fi

  local app_basename bin_path archs
  app_basename="$(basename "$APP_PATH")"
  bin_path="${APP_PATH}/Contents/MacOS/${app_basename%.app}"
  if [ ! -f "$bin_path" ]; then
    echo "::error::Missing app executable at ${bin_path}" >&2
    exit 1
  fi
  archs="$(lipo -archs "$bin_path")"
  case "$archs" in
    *arm64*x86_64*|*x86_64*arm64*)
      echo "Verified universal binary architectures: ${archs}"
      ;;
    *)
      echo "::error::Expected universal arm64+x86_64 binary, got: ${archs}" >&2
      exit 1
      ;;
  esac

  # Create an isolated temporary keychain under RUNNER_TEMP (never inside checkout).
  security delete-keychain "$keychain_path" >/dev/null 2>&1 || rm -f "$keychain_path"
  security create-keychain -p "" "$keychain_path"
  security set-keychain-settings -lut 21600 "$keychain_path"
  security unlock-keychain -p "" "$keychain_path"

  printf '%s' "$MACOS_CERTIFICATE" | base64 --decode > "$cert_path"
  security import "$cert_path" \
    -k "$keychain_path" \
    -P "$MACOS_CERTIFICATE_PWD" \
    -T /usr/bin/codesign \
    -T /usr/bin/security
  rm -f "$cert_path"

  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "" "$keychain_path"
  security list-keychains -d user -s "$keychain_path"

  local cert_id="${MACOS_CERTIFICATE_NAME:-}"
  if [ -z "$cert_id" ]; then
    cert_id="$(security find-identity -v -p codesigning "$keychain_path" | awk '/"Developer ID Application:/ { print $2; exit }')"
  fi
  if [ -z "$cert_id" ]; then
    echo "::error::No Developer ID Application signing identity found in temporary keychain" >&2
    security find-identity -v -p codesigning "$keychain_path" >&2 || true
    exit 1
  fi
  echo "Signing identity: $(security find-identity -v -p codesigning "$keychain_path" | grep -F "$cert_id" | head -n 1 || echo "$cert_id")"

  xattr -cr "$APP_PATH"

  # Re-sign with Developer ID Application + Hardened Runtime + secure timestamp.
  codesign_with_retry \
    --force \
    --deep \
    --verbose \
    --options runtime \
    --timestamp \
    --keychain "$keychain_path" \
    --sign "$cert_id" \
    "$APP_PATH"

  codesign --verify --deep --strict --verbose=2 "$APP_PATH"
  if ! codesign -dvv "$APP_PATH" 2>&1 | grep -q "^Timestamp="; then
    echo "::error::No secure timestamp embedded in signed ${APP_PATH}" >&2
    exit 1
  fi

  mkdir -p "$STAGE_DIR"
  local dmg_path="${STAGE_DIR}/${DMG_NAME}"
  local tar_path="${STAGE_DIR}/${TAR_NAME}"
  rm -f "$dmg_path" "$tar_path"

  create-dmg \
    --volname "$vol_name" \
    --window-pos 200 120 \
    --window-size 800 400 \
    --icon-size 100 \
    --icon "$app_basename" 200 190 \
    --hide-extension "$app_basename" \
    --app-drop-link 600 185 \
    "$dmg_path" \
    "$APP_PATH"

  codesign_with_retry \
    --force \
    --verbose \
    --timestamp \
    --keychain "$keychain_path" \
    --sign "$cert_id" \
    "$dmg_path"

  echo "Submitting ${dmg_path} for Apple notarization..."
  xcrun notarytool submit "$dmg_path" \
    --apple-id "$AC_USERNAME" \
    --password "$AC_PASSWORD" \
    --team-id "$AC_TEAM_ID" \
    --wait

  sleep 30
  staple_with_retry "$dmg_path"
  staple_with_retry "$APP_PATH"

  xcrun stapler validate "$dmg_path"
  xcrun stapler validate "$APP_PATH"
  spctl -a -t open --context context:primary-signature -v "$dmg_path"

  # Repackage the signed and stapled .app for updater / archive downloads.
  COPYFILE_DISABLE=1 tar -czf "$tar_path" -C "$(dirname "$APP_PATH")" "$app_basename"
  ls -lh "$dmg_path" "$tar_path"
}

case "$mode" in
  --preflight) preflight ;;
  --package) package_macos ;;
  --cleanup) cleanup ;;
  *)
    echo "Unknown mode: ${mode}" >&2
    exit 1
    ;;
esac
