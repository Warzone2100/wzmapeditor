#!/usr/bin/env python3
"""Strip a Warzone 2100 `.wz` archive down to what wzmapeditor actually reads.

The bundled `base.wz` ships the full game payload (menu backdrops, audio,
campaign scripts, UI images, high-quality KTX2 terrain). The map editor reads
none of that: only terrain/tile textures, model `.pie` files, stats, the
tileset metadata, the level manifest, and the campaign map data. KTX2 terrain
pages have a PNG fallback in the same archive, so they can go too -- default
terrain renders from PNG and a user-uploaded `high.wz` still supplies KTX2.

This makes the web bundle small enough to host. It is non-destructive: it
writes a new archive and never touches the input.

Usage:
    slim-data.py <input.wz> <output.wz>
"""

import collections
import os
import sys
import zipfile
import io
from PIL import Image # pip install Pillow
import oxipng # pip install pyoxipng

# Whole top-level directories the editor never reads.
DROP_DIRS = {
    "audio",
    "sequenceaudio",
    "images",
    "script",
    "shaders",
    "messages",
    "guidetopics",
}

# Path prefixes to drop (menu/credits backdrops live under texpages/).
DROP_PREFIXES = ("texpages/bdrops/",)

# Extensions to drop wherever they appear. `.ktx2` is the high-quality terrain
# that falls back to `.png`; the rest are audio/shader/web assets.
DROP_EXTS = {".ktx2", ".ogg", ".svg", ".js", ".spv", ".frag", ".vert", ".glsl"}


def should_drop(name: str) -> bool:
    top = name.split("/", 1)[0]
    if top in DROP_DIRS:
        return True
    if name.startswith(DROP_PREFIXES):
        return True
    if os.path.splitext(name)[1].lower() in DROP_EXTS:
        return True
    return False

def optimize_png_in_memory(input_bytes: bytes) -> bytes:
    """Takes PNG file data as bytes, resizes it to fit a 512x512 box
    (if larger), optimizes the PNG, and returns the new bytes
    only if they are smaller than the original input bytes.
    """
    # 1. Load the input bytes into an in-memory stream
    in_stream = io.BytesIO(input_bytes)

    # 2. Open and resize the image
    with Image.open(in_stream) as img:
        width, height = img.size

        # Create an in-memory stream for the output
        out_stream = io.BytesIO()

        if width <= 512 and height <= 512:
            # Skip resizing: save a direct copy to the stream
            img.save(out_stream, format="PNG")
        else:
            # Resize: shrink to fit 512x512 box
            img.thumbnail((512, 512), Image.Resampling.LANCZOS)
            img.save(out_stream, format="PNG")

    # Get the unoptimized resized data from the stream
    resized_bytes = out_stream.getvalue()

    try:
        # 3. Optimize the raw bytes in-memory using oxipng
        optimized_bytes = oxipng.optimize_from_memory(resized_bytes, level=6)

        # 4. Compare sizes and return the smallest version
        if len(optimized_bytes) < len(input_bytes):
            return optimized_bytes

        print(
            f"  Warning: Optimized data ({len(optimized_bytes)} bytes) is not "
            f"  smaller than original ({len(input_bytes)} bytes). Keeping original."
        )
        return input_bytes

    except Exception as e:
        # If optimization fails, fallback safely to the input bytes
        print(f"Error during optimization: {e}. Keeping original.")
        return input_bytes

def main() -> int:
    if len(sys.argv) != 3:
        sys.exit(f"usage: {sys.argv[0]} <input.wz> <output.wz>")
    src, dst = sys.argv[1], sys.argv[2]

    kept_files = kept_bytes = dropped_files = dropped_bytes = 0
    dropped_by_top = collections.defaultdict(lambda: [0, 0])

    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(dst, "w") as zout:
        for info in zin.infolist():
            if info.is_dir():
                if not should_drop(info.filename):
                    zout.writestr(info, b"")
                continue
            if should_drop(info.filename):
                dropped_files += 1
                dropped_bytes += info.file_size
                top = info.filename.split("/", 1)[0]
                dropped_by_top[top][0] += 1
                dropped_by_top[top][1] += info.file_size
                continue
            data = zin.read(info.filename)
            orig_data_len = len(data)
            # Optimize PNGs
            if os.path.splitext(info.filename)[1].lower() == '.png':
                data = optimize_png_in_memory(data)
            # Preserve the original compression method (base.wz is STORED, which
            # the in-browser zip reader can slice without inflating).
            out = zipfile.ZipInfo(info.filename, date_time=info.date_time)
            out.compress_type = info.compress_type
            out.external_attr = info.external_attr
            zout.writestr(out, data)
            written_data_len = len(data)
            kept_files += 1
            kept_bytes += written_data_len
            print(f"  wrote: {info.filename} {written_data_len/1024:8.2f} KiB")
            if written_data_len < orig_data_len:
                print(f"   - (original size: {orig_data_len/1024:8.2f} KiB)")

    print(f"{os.path.basename(src)} -> {os.path.basename(dst)}")
    print(f"  kept:    {kept_files:5d} files  {kept_bytes/1e6:8.2f} MB")
    print(f"  dropped: {dropped_files:5d} files  {dropped_bytes/1e6:8.2f} MB")
    print(f"  on disk: {os.path.getsize(src)/1e6:8.2f} MB -> "
          f"{os.path.getsize(dst)/1e6:8.2f} MB")
    print("  dropped by top-level dir:")
    for top, (c, b) in sorted(dropped_by_top.items(), key=lambda x: -x[1][1]):
        print(f"    {top:16s} {c:5d}  {b/1e6:8.2f} MB")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
