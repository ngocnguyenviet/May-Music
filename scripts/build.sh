#!/usr/bin/env bash
set -euo pipefail
app_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if ! pkg-config --exists gtk4; then
    echo 'Thiếu GTK4 development. Ubuntu/Debian: sudo apt install build-essential pkg-config libgtk-4-dev' >&2
    exit 1
fi
cargo build --manifest-path "$app_dir/Cargo.toml" --release --locked --target-dir "$app_dir/target"
mkdir -p "$app_dir/bin"
install -m 755 "$app_dir/target/release/may-music" "$app_dir/bin/may-music"
echo "Đã build. Chạy: $app_dir/run.sh"
