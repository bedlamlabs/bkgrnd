#!/usr/bin/env python3
"""Generate tray and app icons for Play."""

import struct
import zlib
import os

ICON_DIR = os.path.join(os.path.dirname(__file__), '..', 'client', 'src-tauri', 'icons')
os.makedirs(ICON_DIR, exist_ok=True)


def make_png(width, height, pixels):
    """Create a minimal PNG from RGBA pixel data."""
    def chunk(chunk_type, data):
        c = chunk_type + data
        return struct.pack('>I', len(data)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)

    raw = b''
    for y in range(height):
        raw += b'\x00'  # filter: none
        for x in range(width):
            idx = (y * width + x) * 4
            raw += bytes(pixels[idx:idx+4])

    header = b'\x89PNG\r\n\x1a\n'
    ihdr = chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
    idat = chunk(b'IDAT', zlib.compress(raw))
    iend = chunk(b'IEND', b'')
    return header + ihdr + idat + iend


def draw_play_triangle(size, color, filled=True):
    """Draw a play triangle (right-pointing) on transparent background."""
    pixels = [0] * (size * size * 4)
    margin = size * 0.2
    r, g, b, a = color

    for y in range(size):
        for x in range(size):
            # Normalized coordinates
            nx = (x - margin) / (size - 2 * margin)
            ny = (y - margin) / (size - 2 * margin)

            if nx < 0 or nx > 1 or ny < 0 or ny > 1:
                continue

            # Triangle: right-pointing. At nx=0, ny spans 0.15-0.85. At nx=0.85, ny=0.5
            center_y = 0.5
            half_height = 0.42 * (1.0 - nx / 0.85)

            if nx <= 0.85 and abs(ny - center_y) <= half_height:
                idx = (y * size + x) * 4
                if filled:
                    pixels[idx] = r
                    pixels[idx+1] = g
                    pixels[idx+2] = b
                    pixels[idx+3] = a
                else:
                    # Outline only — draw if near edge
                    edge_dist = min(
                        abs(nx),  # left edge
                        abs(nx - 0.85),  # right edge (tip)
                        abs(abs(ny - center_y) - half_height)  # top/bottom edges
                    )
                    if edge_dist < 0.08:
                        pixels[idx] = r
                        pixels[idx+1] = g
                        pixels[idx+2] = b
                        pixels[idx+3] = a

    return pixels


def draw_pause(size, color):
    """Draw pause bars on transparent background."""
    pixels = [0] * (size * size * 4)
    margin = size * 0.25
    r, g, b, a = color

    bar_width = size * 0.18
    gap = size * 0.12
    center_x = size / 2

    for y in range(size):
        for x in range(size):
            ny = (y - margin) / (size - 2 * margin)
            if ny < 0 or ny > 1:
                continue

            # Left bar
            left_start = center_x - gap / 2 - bar_width
            left_end = center_x - gap / 2
            # Right bar
            right_start = center_x + gap / 2
            right_end = center_x + gap / 2 + bar_width

            if (left_start <= x <= left_end) or (right_start <= x <= right_end):
                idx = (y * size + x) * 4
                pixels[idx] = r
                pixels[idx+1] = g
                pixels[idx+2] = b
                pixels[idx+3] = a

    return pixels


def draw_play_circle(size, color):
    """Draw a filled play triangle inside a circle."""
    pixels = [0] * (size * size * 4)
    r, g, b, a = color
    center = size / 2
    radius = size * 0.42

    for y in range(size):
        for x in range(size):
            dx = x - center
            dy = y - center
            dist = (dx*dx + dy*dy) ** 0.5

            # Circle outline
            if abs(dist - radius) < 1.2:
                idx = (y * size + x) * 4
                pixels[idx] = r
                pixels[idx+1] = g
                pixels[idx+2] = b
                pixels[idx+3] = a
                continue

            # Inner triangle (smaller)
            if dist > radius:
                continue
            inner_margin = size * 0.3
            nx = (x - inner_margin) / (size - 2 * inner_margin)
            ny_val = (y - inner_margin) / (size - 2 * inner_margin)
            if nx < 0 or nx > 1 or ny_val < 0 or ny_val > 1:
                continue

            center_y = 0.5
            half_h = 0.38 * (1.0 - nx / 0.8)
            if nx <= 0.8 and abs(ny_val - center_y) <= half_h:
                idx = (y * size + x) * 4
                pixels[idx] = r
                pixels[idx+1] = g
                pixels[idx+2] = b
                pixels[idx+3] = a

    return pixels


# Tray icons (22x22 for macOS menu bar, @2x = 44x44)
TRAY_SIZE = 44  # @2x
WHITE = (230, 230, 230, 255)
BLUE = (74, 158, 255, 255)
ORANGE = (245, 166, 35, 255)
GRAY = (160, 160, 160, 255)

# icon.png — idle play outline
pixels = draw_play_triangle(TRAY_SIZE, GRAY, filled=False)
with open(os.path.join(ICON_DIR, 'icon.png'), 'wb') as f:
    f.write(make_png(TRAY_SIZE, TRAY_SIZE, pixels))

# icon-playing.png — filled play (yt-dlp mode)
pixels = draw_play_triangle(TRAY_SIZE, WHITE, filled=True)
with open(os.path.join(ICON_DIR, 'icon-playing.png'), 'wb') as f:
    f.write(make_png(TRAY_SIZE, TRAY_SIZE, pixels))

# icon-playing-chromium.png — play in circle (browser fallback)
pixels = draw_play_circle(TRAY_SIZE, ORANGE)
with open(os.path.join(ICON_DIR, 'icon-playing-chromium.png'), 'wb') as f:
    f.write(make_png(TRAY_SIZE, TRAY_SIZE, pixels))

# icon-paused.png — pause bars
pixels = draw_pause(TRAY_SIZE, GRAY)
with open(os.path.join(ICON_DIR, 'icon-paused.png'), 'wb') as f:
    f.write(make_png(TRAY_SIZE, TRAY_SIZE, pixels))

# App bundle icons (required by tauri.conf.json)
for size, name in [(32, '32x32.png'), (128, '128x128.png'), (256, '128x128@2x.png')]:
    pixels = draw_play_triangle(size, BLUE, filled=True)
    with open(os.path.join(ICON_DIR, name), 'wb') as f:
        f.write(make_png(size, size, pixels))

# icon.ico — just use 32x32
import shutil
shutil.copy(os.path.join(ICON_DIR, '32x32.png'), os.path.join(ICON_DIR, 'icon.ico'))

# icon.icns — create a minimal one from 128x128
# For a proper .icns we'd need a tool, but Tauri accepts PNG fallback
shutil.copy(os.path.join(ICON_DIR, '128x128.png'), os.path.join(ICON_DIR, 'icon.icns'))

print(f"Icons generated in {os.path.abspath(ICON_DIR)}")
for f in sorted(os.listdir(ICON_DIR)):
    size = os.path.getsize(os.path.join(ICON_DIR, f))
    print(f"  {f} ({size} bytes)")
