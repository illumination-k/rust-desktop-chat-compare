#!/usr/bin/env bash
# System libraries needed to build every GUI app on Debian/Ubuntu
# (webkit2gtk for Tauri/Dioxus, GTK/xdo for menus, xkbcommon/fontconfig for winit-based apps).
set -euo pipefail

sudo apt-get update
sudo apt-get install -y --no-install-recommends \
	libwebkit2gtk-4.1-dev \
	libgtk-3-dev \
	libayatana-appindicator3-dev \
	librsvg2-dev \
	libxdo-dev \
	libxkbcommon-dev \
	libfontconfig1-dev \
	libssl-dev
