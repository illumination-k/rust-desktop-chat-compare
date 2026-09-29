#!/usr/bin/env bash
# Measures release binary size, clean/incremental build time and code size of every app.
#
#   bench/measure.sh            # all apps
#   bench/measure.sh egui iced  # a subset
#
# Writes a Markdown table to bench/results/<date>-<os>.md. Runtime metrics
# (startup, memory, CPU while streaming) are measured manually; see docs/comparison.md.
set -euo pipefail

cd "$(dirname "$0")/.."
apps=("$@")
[ ${#apps[@]} -eq 0 ] && apps=(egui tauri iced slint dioxus)

crate_of() { echo "chat-$1"; }
dir_of() { if [ "$1" = tauri ]; then echo apps/tauri; else echo "apps/$1"; fi; }

build() {
	local app=$1
	if [ "$app" = tauri ]; then
		pnpm --filter chat-tauri build >/dev/null
		cargo build --release --locked -p chat-tauri --features tauri/custom-protocol
	else
		cargo build --release --locked -p "$(crate_of "$app")"
	fi
}

seconds() {
	local start end
	start=$(date +%s.%N)
	"$@" >/dev/null 2>&1
	end=$(date +%s.%N)
	echo "$end - $start" | bc | xargs printf '%.1f'
}

loc() {
	tokei "$(dir_of "$1")" -e node_modules -e dist -e gen -e icons -e '*.test.ts' -o json |
		python3 -c 'import json,sys; print(json.load(sys.stdin)["Total"]["code"])'
}

out="bench/results/$(date +%Y-%m-%d)-$(uname -s | tr '[:upper:]' '[:lower:]').md"
{
	echo "# Build metrics ($(date +%Y-%m-%d), $(uname -sm), rustc $(rustc -V | cut -d' ' -f2))"
	echo
	echo "| App | Release binary | Clean build (s) | Incremental build (s) | Code (lines) |"
	echo "| --- | ---: | ---: | ---: | ---: |"
} >"$out"

for app in "${apps[@]}"; do
	echo "measuring $app..." >&2
	# "Clean" = the app crate and its dependencies from scratch, sharing nothing with other apps.
	rm -rf target/release
	clean=$(seconds build "$app")
	touch crates/chat-core/src/lib.rs
	incremental=$(seconds build "$app")
	size=$(du -h "target/release/$(crate_of "$app")" | cut -f1)
	echo "| $app | $size | $clean | $incremental | $(loc "$app") |" >>"$out"
done

echo "wrote $out" >&2
