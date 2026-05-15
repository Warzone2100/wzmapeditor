#!/bin/bash

set -eu -o pipefail

info() {
  echo "::info::$*" >&2
}
warn() {
  echo "::warning::$*" >&2
}
error() {
  echo "::error::$*" >&2
}
set_gh_output() {
  echo "$1=$2" >> "$GITHUB_OUTPUT"
}

NAME="wzmapeditor_${RUSTTARGET}"
if [[ -n "${WZMAPEDITOR_OUTPUT_DESC:-}" ]]; then
  NAME="wzmapeditor_${WZMAPEDITOR_OUTPUT_DESC}"
fi

OUTPUT_DIR="${GITHUB_WORKSPACE}/output"
mkdir -p "${OUTPUT_DIR}"

FIND_CMD="find"
if [[ "$RUNNER_OS" == "macOS" ]]; then
  FIND_CMD="gfind"
fi

CARGO_CMD="cargo"
if [[ "${USE_CROSS:-}" == "true" ]]; then
  CARGO_CMD="cross"
fi

if [[ -n "${RUSTFLAGS:-}" ]]; then
  # If RUSTFLAGS is set, any configuration through target.*.rustflags or build.rustflags will be omitted
  warn "RUSTFLAGS is currently set to '${RUSTFLAGS}'"
fi

# Workspace root is virtual; read the binary crate's manifest directly.
BINARIES="$(cargo read-manifest --manifest-path crates/wzmapeditor/Cargo.toml | jq -r ".targets[] | select(.kind[] | contains(\"bin\")) | .name")"

OUTPUT_LIST=""
for BINARY in $BINARIES; do
  info "Building $BINARY (for target $RUSTTARGET) ..."

  CARGO_TARGET_DIR="./target" ${CARGO_CMD} build --release --target "${RUSTTARGET}" --bin "${BINARY}" -v >&2
  OUTPUT=$(${FIND_CMD} "target/${RUSTTARGET}/release/" -maxdepth 1 -type f -executable \( -name "${BINARY}" -o -name "${BINARY}.*" \) -print0 | xargs -0)

  info "${OUTPUT}"

  if [ "${OUTPUT}" = "" ]; then
    error "Unable to find output"
    exit 1
  fi

  info "Saving ${OUTPUT} ..."

  mv $OUTPUT "${OUTPUT_DIR}" || error "Unable to copy: ${OUTPUT}"

  for f in $OUTPUT; do
    OUTPUT_LIST="$OUTPUT_LIST $(basename "$f")"

    if [[ "${WZMAPEDITOR_PACKAGE_DEBUGSYMBOLS:-}" == "true" ]]; then
      # Strip existing extension and check for a corresponding .pdb file
      PDB_FILE="${f%.*}.pdb"
      if [ -f "$PDB_FILE" ]; then
        info "Found debug symbols: $PDB_FILE"
        mv "$PDB_FILE" "${OUTPUT_DIR}" || error "Unable to copy: ${PDB_FILE}"
        OUTPUT_LIST="$OUTPUT_LIST $(basename "$PDB_FILE")"
      fi
    fi
  done
done

# Trim & normalize whitespace
OUTPUT_LIST=$(echo "${OUTPUT_LIST}" | awk '{$1=$1};1')

cd "${OUTPUT_DIR}"

# Pack into archive
info "Packing files: ${OUTPUT_LIST}"
ARCHIVE_FILE_NAME="${NAME}.zip"
if [[ "$RUNNER_OS" == "Windows" ]]; then
  7z a $ARCHIVE_FILE_NAME ${OUTPUT_LIST}
else
  zip -9r $ARCHIVE_FILE_NAME ${OUTPUT_LIST}
fi
# Set GitHub step output variables
set_gh_output "BUILT_ARCHIVE" "${OUTPUT_DIR}/${ARCHIVE_FILE_NAME}"
