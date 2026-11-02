#!/usr/bin/env bash
# Optional: run this explicitly to add a launcher to your own Linux application menu.
set -euo pipefail
app_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
dest="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$dest"
python3 - "$app_dir" "$dest/io.github.maymusic.Player.desktop" <<'PY'
import pathlib, sys
root, target = sys.argv[1:]
def quoted(s):
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%') + '"'
pathlib.Path(target).write_text('''[Desktop Entry]
Type=Application
Name=Mây Music
Comment=Nghe nhạc YouTube nhẹ, chạy nền
Exec={exe}
Icon={icon}
Terminal=false
Categories=AudioVideo;Audio;Player;
StartupNotify=true
Actions=Toggle;Next;Quit;

[Desktop Action Toggle]
Name=Phát / tạm dừng
Exec={exe} --toggle

[Desktop Action Next]
Name=Bài tiếp theo
Exec={exe} --next

[Desktop Action Quit]
Name=Thoát hoàn toàn
Exec={exe} --quit
'''.format(exe=quoted(root + '/run.sh'), icon=root + '/assets/may-music.svg'))
print('Đã thêm launcher:', target)
PY
