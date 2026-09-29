#!/usr/bin/env bash

set -euo pipefail

if ! command -v mise >/dev/null 2>&1; then
	curl https://mise.run | sh >&2
	export PATH="$HOME/.local/bin:$PATH"
fi

mise trust --all >&2
mise settings experimental=true >&2
mise install >&2

cat <<'EOF'
This repository uses mise for tool and task management. Prefer `mise run fmt`,
`mise run lint`, and other `mise run ...` tasks when working in this repo.
EOF
