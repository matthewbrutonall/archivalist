#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
PACKAGE_VERSION="$(awk -F'"' '/^version = / { print $2; exit }' "${WORKSPACE_ROOT}/crates/aspace-cli/Cargo.toml")"
GIT_REVISION="$(git -C "${WORKSPACE_ROOT}" rev-parse --short=9 HEAD)"
DIRTY_SUFFIX=""
if ! git -C "${WORKSPACE_ROOT}" diff --quiet || ! git -C "${WORKSPACE_ROOT}" diff --cached --quiet; then
    DIRTY_SUFFIX="-dirty"
fi
VERSION="${VERSION:-v${PACKAGE_VERSION}-g${GIT_REVISION}${DIRTY_SUFFIX}}"
TARGET="${TARGET:-$(rustc -vV | awk '/^host:/ {print $2}')}"
DIST_DIR="${DIST_DIR:-${WORKSPACE_ROOT}/dist}"
ARCHIVE_NAME="archivalist-sidecar-${VERSION}-${TARGET}"
STAGING_DIR="$(mktemp -d)"

cleanup() {
    rm -rf "${STAGING_DIR}"
}
trap cleanup EXIT

cargo build --locked --release \
    --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" \
    -p aspace-cli --bin aspace-shadow-sidecar

mkdir -p "${DIST_DIR}" "${STAGING_DIR}/${ARCHIVE_NAME}/deploy"
install -m 0755 "${WORKSPACE_ROOT}/target/release/aspace-shadow-sidecar" \
    "${STAGING_DIR}/${ARCHIVE_NAME}/aspace-shadow-sidecar"
install -m 0644 "${WORKSPACE_ROOT}/deploy/aspace-shadow-sidecar.service.example" \
    "${STAGING_DIR}/${ARCHIVE_NAME}/deploy/aspace-shadow-sidecar.service"
install -m 0644 "${WORKSPACE_ROOT}/deploy/aspace-shadow-sidecar.env.example" \
    "${STAGING_DIR}/${ARCHIVE_NAME}/deploy/aspace-shadow-sidecar.env"
install -m 0644 "${WORKSPACE_ROOT}/deploy/CANARY.md" \
    "${STAGING_DIR}/${ARCHIVE_NAME}/CANARY.md"

tar --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime="@${SOURCE_DATE_EPOCH:-0}" \
    -C "${STAGING_DIR}" -czf "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz" "${ARCHIVE_NAME}"
sha256sum "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz" > "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz.sha256"

printf '%s\n' "Created ${DIST_DIR}/${ARCHIVE_NAME}.tar.gz"
printf '%s\n' "Checksum: $(cut -d' ' -f1 "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz.sha256")"
