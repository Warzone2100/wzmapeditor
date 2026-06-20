#!/usr/bin/env bash
# Fetch the latest WZ2100 release data and slim base.wz for the web bundle.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data_dir="$repo_root/crates/wzmapeditor/data"
overrides_dir="$data_dir/terrain_overrides"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

asset="warzone2100_win_x64_archive.zip"
url="$(gh api repos/Warzone2100/warzone2100/releases/latest \
  --jq ".assets[] | select(.name == \"$asset\") | .browser_download_url")"
if [ -z "$url" ]; then
  echo "error: could not resolve download URL for $asset" >&2
  exit 1
fi

echo "Downloading $asset"
curl -fL --retry 3 -o "$work/wz.zip" "$url"

echo "Extracting web archives"
mkdir -p "$overrides_dir"
unzip -o -j "$work/wz.zip" 'data/base.wz' -d "$work"
unzip -o -j "$work/wz.zip" 'data/mp.wz' -d "$data_dir"
unzip -o -j "$work/wz.zip" 'data/terrain_overrides/classic.wz' -d "$overrides_dir"

echo "Slimming base.wz"
python3 "$repo_root/scripts/slim-data.py" "$work/base.wz" "$data_dir/base.wz"
