# App icons

The CEC "critical error" brand mark (cyan bracket-triangle + magenta bar),
generated at every size Tauri needs from `assets/cec-logo.png`:

| File | Size | Used by |
|------|------|---------|
| `32x32.png` | 32 | Linux / small |
| `128x128.png` · `128x128@2x.png` | 128 / 256 | Linux / macOS |
| `icon.png` | 512 | source / Linux |
| `icon.ico` | 16–256 (multi) | **Windows** taskbar, exe, installer |
| `icon.icns` | 128–512 | macOS app |

These are the real brand — not AllMyStuff placeholders. If the logo art
changes, re-generate the whole set from the new source at the same sizes.
