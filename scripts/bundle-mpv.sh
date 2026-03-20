#!/bin/bash
# Collects mpv + all dylib dependencies from Homebrew and rewrites loader paths
# so the bundle is self-contained. Run once before building.

set -euo pipefail

DEST="$(cd "$(dirname "$0")/.." && pwd)/src-tauri/binaries/mpv-bundle"
MPV_BIN="$(which mpv)"

if [ -z "$MPV_BIN" ]; then
    echo "Error: mpv not found in PATH. Install it via: brew install mpv"
    exit 1
fi

echo "==> Collecting mpv and dependencies into $DEST"
rm -rf "$DEST"
mkdir -p "$DEST"

# Copy mpv binary
cp "$MPV_BIN" "$DEST/mpv"
chmod +x "$DEST/mpv"

# Recursively discover all dylib dependencies from /opt/homebrew
collect_dylibs() {
    local binary="$1"
    otool -L "$binary" 2>/dev/null | awk '{print $1}' | grep '^/opt/homebrew' | while read -r dep; do
        # Resolve symlinks to get the real file
        local real_dep
        real_dep="$(readlink -f "$dep" 2>/dev/null || echo "$dep")"
        local base
        base="$(basename "$dep")"
        local real_base
        real_base="$(basename "$real_dep")"

        # Copy the real file if not already copied
        if [ ! -f "$DEST/$real_base" ]; then
            cp "$real_dep" "$DEST/$real_base"
            chmod 644 "$DEST/$real_base"
            # Recurse into this dylib's dependencies
            collect_dylibs "$DEST/$real_base"
        fi

        # Create symlink if the referenced name differs from the real file
        if [ "$base" != "$real_base" ] && [ ! -e "$DEST/$base" ]; then
            ln -s "$real_base" "$DEST/$base"
        fi
    done
}

echo "==> Discovering dylib dependencies..."
collect_dylibs "$DEST/mpv"

echo "==> Rewriting load paths to @loader_path/"
for file in "$DEST"/*; do
    # Skip symlinks
    [ -L "$file" ] && continue
    # Skip non-Mach-O files
    file_type="$(file "$file")"
    echo "$file_type" | grep -q "Mach-O" || continue

    # Rewrite all /opt/homebrew references
    otool -L "$file" 2>/dev/null | awk '{print $1}' | grep '^/opt/homebrew' | while read -r dep; do
        local_name="$(basename "$dep")"
        install_name_tool -change "$dep" "@loader_path/$local_name" "$file" 2>/dev/null || true
    done

    # Also rewrite the install name (id) for dylibs
    if echo "$file_type" | grep -q "dynamically linked shared library"; then
        local_name="$(basename "$file")"
        install_name_tool -id "@loader_path/$local_name" "$file" 2>/dev/null || true
    fi
done

echo "==> Ad-hoc codesigning..."
for file in "$DEST"/*; do
    [ -L "$file" ] && continue
    file_type="$(file "$file")"
    echo "$file_type" | grep -q "Mach-O" || continue
    codesign --force --sign - "$file" 2>/dev/null || true
done

echo "==> Verifying: no remaining /opt/homebrew references"
bad=0
for file in "$DEST"/*; do
    [ -L "$file" ] && continue
    file_type="$(file "$file")"
    echo "$file_type" | grep -q "Mach-O" || continue
    if otool -L "$file" 2>/dev/null | grep -q '/opt/homebrew'; then
        echo "WARNING: $file still references /opt/homebrew:"
        otool -L "$file" | grep '/opt/homebrew'
        bad=1
    fi
done

if [ $bad -eq 0 ]; then
    echo "==> All clear! No /opt/homebrew references remain."
fi

count=$(find "$DEST" -maxdepth 1 -type f | wc -l | tr -d ' ')
size=$(du -sh "$DEST" | awk '{print $1}')
echo "==> Done: $count files, $size total in $DEST"
