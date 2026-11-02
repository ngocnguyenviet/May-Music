#!/usr/bin/env bash
# Run explicitly to expose `may-music` in the current user's command PATH.
set -euo pipefail
app_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
command_dir="${MAY_BIN_DIR:-$HOME/.local/bin}"
python3 - "$app_dir" "$command_dir" <<'PY'
import os
from pathlib import Path
import shlex
import sys
import tempfile

root, directory = sys.argv[1:]
directory = Path(directory)
directory.mkdir(parents=True, exist_ok=True)
target = directory / 'may-music'
marker = '# Mây Music command launcher\n'
if target.exists() or target.is_symlink():
    if target.is_symlink() or marker not in target.read_text(errors='replace')[:100]:
        raise SystemExit(f'Đã có một lệnh khác tại {target}; giữ nguyên, chưa ghi đè.')
content = '#!/usr/bin/env bash\n' + marker + 'exec ' + shlex.quote(str(Path(root) / 'run.sh')) + ' "$@"\n'
with tempfile.NamedTemporaryFile(mode='w', dir=directory, prefix='.may-music-', delete=False) as f:
    f.write(content)
    temporary = Path(f.name)
try:
    temporary.chmod(0o755)
    temporary.replace(target)
finally:
    temporary.unlink(missing_ok=True)
print(f'Đã tạo lệnh: {target}')
PY
