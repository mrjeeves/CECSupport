#!/usr/bin/env bash
# Shared by Linux CI, release builds, and `just linux-deps`. Match AllMyStuff's
# media toolchain while keeping Ubuntu 22.04's glibc 2.35 release baseline.
set -euo pipefail

source /etc/os-release
if [[ "$ID" != ubuntu ]]; then
  echo "Install the equivalent Tauri/AllMyStuff development packages for $ID; see docs/LINUX-SUPPORT.md." >&2
  exit 1
fi

sudo_cmd=()
if [[ "$EUID" -ne 0 ]]; then sudo_cmd=(sudo); fi

# Avoid the stalled Azure mirror seen in AllMyStuff's hosted release builds.
if [[ -f /etc/apt/apt-mirrors.txt ]]; then
  "${sudo_cmd[@]}" sed -i '/azure\.archive\.ubuntu\.com/d' /etc/apt/apt-mirrors.txt
fi

apt_options=(-o Acquire::Retries=5 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
if [[ "$VERSION_ID" == 22.04 ]]; then
  # Jammy's stock PipeWire 0.3.48 headers predate the libspa bindings used by
  # the shared capture engine. Use the same Jammy PPA as AllMyStuff.
  "${sudo_cmd[@]}" apt-get "${apt_options[@]}" update
  "${sudo_cmd[@]}" env DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" install -y software-properties-common
  "${sudo_cmd[@]}" add-apt-repository -y --no-update ppa:pipewire-debian/pipewire-upstream
fi

"${sudo_cmd[@]}" apt-get "${apt_options[@]}" update
"${sudo_cmd[@]}" env DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" install -y \
  build-essential pkg-config clang libclang-dev curl ca-certificates file \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libssl-dev libasound2-dev xdg-utils \
  libpipewire-0.3-dev libxkbcommon-dev libgbm-dev
