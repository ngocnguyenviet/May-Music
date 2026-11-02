#!/usr/bin/env bash
set -euo pipefail
app_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export PATH="$app_dir/bin:$PATH"
# This interface needs no GPU renderer. Explicit user values take precedence.
export GSK_RENDERER="${GSK_RENDERER-cairo}"
export GDK_DISABLE="${GDK_DISABLE-gl,vulkan}"

if [[ ! -x "$app_dir/bin/may-music" ]]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo 'Chưa có bản build. Xem README.md để cài Rust và GTK4 development, rồi chạy scripts/build.sh.' >&2
        exit 1
    fi
    "$app_dir/scripts/build.sh"
fi

if [[ "${1:-}" != '--help' && "${1:-}" != '--diagnose' ]]; then
    missing=()
    command -v "${MAY_MPV:-mpv}" >/dev/null 2>&1 || missing+=(mpv)
    command -v "${MAY_YTDLP:-yt-dlp}" >/dev/null 2>&1 || missing+=(yt-dlp)
    if (( ${#missing[@]} )); then
        echo "Thiếu: ${missing[*]}. Xem README.md để cài dependency." >&2
        exit 1
    fi
fi
exec "$app_dir/bin/may-music" "$@"
