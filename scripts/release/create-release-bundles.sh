#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 INPUT_ARTIFACT_DIR OUTPUT_BUNDLE_DIR" >&2
  exit 2
fi

artifact_dir="$1"
bundle_dir="$2"

# Archive metadata must be stable across recovery builds. The public workflow
# still refuses to replace existing release assets; reproducible packaging is a
# diagnostic and provenance aid, not permission to overwrite published bytes.
export TZ=UTC
archive_timestamp="200001010000"

if [[ ! -d "${artifact_dir}" ]]; then
  echo "input artifact directory does not exist: ${artifact_dir}" >&2
  exit 1
fi
artifact_dir="$(cd "${artifact_dir}" && pwd)"

if [[ -e "${bundle_dir}" && -n "$(find "${bundle_dir}" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
  echo "output bundle directory must be empty: ${bundle_dir}" >&2
  exit 1
fi
mkdir -p "${bundle_dir}"
bundle_dir="$(cd "${bundle_dir}" && pwd)"

manifest="${bundle_dir}/codewhale-bundles-sha256.txt"
: > "${manifest}"

bundle() {
  local platform="$1"
  local cli_src="$2"
  local shim_src="$3"
  local tui_src="$4"
  local ext="$5"
  local variant="$6"

  local stem="codewhale-${platform}${variant:+-}${variant}"
  local stage_root
  stage_root="$(mktemp -d)"
  local stage_dir="${stage_root}/${stem}"
  mkdir -p "${stage_dir}"

  local cli_dst="codewhale"
  local shim_dst="codew"
  local tui_dst="codewhale-tui"
  if [[ "${platform}" == windows-* ]]; then
    cli_dst="codewhale.exe"
    shim_dst="codew.exe"
    tui_dst="codewhale-tui.exe"
  fi

  cp "${artifact_dir}/${cli_src}/${cli_src}" "${stage_dir}/${cli_dst}"
  cp "${artifact_dir}/${shim_src}/${shim_src}" "${stage_dir}/${shim_dst}"
  cp "${artifact_dir}/${tui_src}/${tui_src}" "${stage_dir}/${tui_dst}"

  # actions/upload-artifact intentionally normalizes downloaded files to 0644.
  # Restore the executable contract before constructing Unix archives.
  if [[ "${platform}" != windows-* ]]; then
    chmod 0755 \
      "${stage_dir}/${cli_dst}" \
      "${stage_dir}/${shim_dst}" \
      "${stage_dir}/${tui_dst}"
  fi

  if [[ "${variant}" != "portable" ]]; then
    if [[ "${platform}" == windows-* ]]; then
      cp scripts/release/install.bat "${stage_dir}/"
      sed -i 's/$/\r/' "${stage_dir}/install.bat" 2>/dev/null || true
    else
      cp scripts/release/install.sh "${stage_dir}/"
      chmod +x "${stage_dir}/install.sh"
    fi
  fi

  # zip and tar both record mtimes; normalize every staged entry so identical
  # inputs do not produce packaging-only checksum drift on a rerun.
  find "${stage_dir}" -exec touch -t "${archive_timestamp}" {} +

  local archive="${bundle_dir}/${stem}.${ext}"
  if [[ "${ext}" == "zip" ]]; then
    (cd "${stage_root}" && zip -Xqr "${archive}" "${stem}/")
  elif tar --version 2>/dev/null | grep -q 'GNU tar'; then
    tar \
      --sort=name \
      --mtime='2000-01-01 00:00:00 UTC' \
      --owner=0 \
      --group=0 \
      --numeric-owner \
      --format=ustar \
      -cf - \
      -C "${stage_root}" \
      "${stem}/" | gzip -n > "${archive}"
  else
    COPYFILE_DISABLE=1 tar -cf - -C "${stage_root}" "${stem}/" | gzip -n > "${archive}"
  fi

  local checksum
  checksum="$(sha256sum "${archive}" | awk '{print $1}')"
  printf '%s  %s\n' "${checksum}" "$(basename "${archive}")" >> "${manifest}"
  rm -rf "${stage_root}"
  echo "Created ${archive}"
}

bundle linux-x64 \
  codewhale-linux-x64 codew-linux-x64 codewhale-tui-linux-x64 tar.gz ""
bundle linux-arm64 \
  codewhale-linux-arm64 codew-linux-arm64 codewhale-tui-linux-arm64 tar.gz ""
bundle android-arm64 \
  codewhale-android-arm64 codew-android-arm64 codewhale-tui-android-arm64 tar.gz ""
bundle macos-x64 \
  codewhale-macos-x64 codew-macos-x64 codewhale-tui-macos-x64 tar.gz ""
bundle macos-arm64 \
  codewhale-macos-arm64 codew-macos-arm64 codewhale-tui-macos-arm64 tar.gz ""
bundle windows-x64 \
  codewhale-windows-x64.exe codew-windows-x64.exe codewhale-tui-windows-x64.exe zip ""
bundle windows-x64 \
  codewhale-windows-x64.exe codew-windows-x64.exe codewhale-tui-windows-x64.exe zip portable
bundle windows-arm64 \
  codewhale-windows-arm64.exe codew-windows-arm64.exe codewhale-tui-windows-arm64.exe zip ""
bundle windows-arm64 \
  codewhale-windows-arm64.exe codew-windows-arm64.exe codewhale-tui-windows-arm64.exe zip portable

sort -o "${manifest}" "${manifest}"
echo "Bundle checksum manifest:"
cat "${manifest}"
