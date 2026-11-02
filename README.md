# Mây Music · bản đầu 0.1.0

App nghe nhạc YouTube cho Linux, viết bằng Rust + GTK4, phát âm thanh bằng mpv và tìm/lấy luồng qua yt-dlp. Playlist lưu trên máy bằng SQLite. Không cần API key.

## Chạy thử

Trong thư mục này:

```bash
./run.sh
```

Có thể tạo lệnh ngắn cho tài khoản hiện tại bằng `./scripts/install-command.sh`. Khi `~/.local/bin` đã có trong `PATH`, dùng:

```bash
may-music          # mở app / hiện lại cửa sổ nếu app đang chạy ẩn
may-music --show   # hiện cửa sổ
may-music --hide   # ẩn, tiếp tục nghe
may-music --play   # phát / tiếp tục; đang phát thì giữ nguyên
may-music --pause  # tạm dừng, giữ vị trí
may-music --toggle # phát / tạm dừng
may-music --stop   # dừng và đưa vị trí về đầu bài, không thoát app
may-music --quit   # thoát hoàn toàn
```

Lệnh này trỏ về thư mục app hiện tại. Nếu di chuyển thư mục, chạy lại script cài lệnh. Không cần sửa `.bashrc` khi `~/.local/bin` đã nằm trong `PATH`.

Bản build đi kèm dành cho **Linux x86_64, được build và kiểm tra trên Ubuntu 26.04**. GTK4 và các thư viện hệ thống vẫn cần có trên máy. Đây chưa phải AppImage chạy trên mọi distro. `bin/mpv` và `bin/yt-dlp` là công cụ đi kèm cho môi trường này; có thể thay bằng bản phù hợp với distro của cậu.

```bash
./run.sh --diagnose
./run.sh --background  # mở ẩn, chưa tự phát
./run.sh --show        # hiện lại cửa sổ app đang chạy
./run.sh --play        # phát / tiếp tục nhạc
./run.sh --pause       # tạm dừng, giữ vị trí
./run.sh --toggle      # phát / tạm dừng
./run.sh --next
./run.sh --previous
./run.sh --stop
./run.sh --quit        # dừng nhạc và thoát hoàn toàn
```

`--play` không đảo trạng thái: gọi khi đang phát vẫn tiếp tục nghe, gọi khi pause thì tiếp tục tại vị trí cũ, gọi sau `--stop` thì phát bài từ đầu. Nếu app chưa chạy, `may-music --play` mở app và phát bài gần nhất; khi chưa có bài gần nhất thì dùng playlist đang chọn. Playlist trống sẽ được báo để cậu chọn bài. `--toggle` dùng khi muốn đổi qua lại giữa phát và pause.

Để thêm app vào menu ứng dụng, giữ nguyên thư mục này rồi chạy tùy chọn:

```bash
./scripts/install-desktop.sh
```

Script chỉ tạo launcher trong thư mục ứng dụng của tài khoản hiện tại. Nếu di chuyển thư mục app, chạy lại script. Không tự bật khởi động cùng máy.

## Cách dùng

1. Nhập tên bài/nghệ sĩ **hoặc dán link video YouTube**, nhấn Enter hoặc **Tìm nhạc**. Từ khóa trả tối đa 16 kết quả; URL video trả đúng bài có ID trong link để cậu nhấn phát hoặc **+** thêm vào playlist. Hỗ trợ `youtube.com/watch?v=…`, `youtu.be/…`, link YouTube Music, Shorts và embed. Bỏ livestream và nội dung không công khai khi nhận diện được.
2. Nhấn **+** để lưu bài vào playlist đang chọn ở thanh bên. Có thể tạo nhiều playlist bằng nút **+** cạnh tiêu đề PLAYLIST.
3. Chọn playlist rồi nhấn **Phát playlist**, hoặc nút phát của một bài để nghe từ vị trí đó. Các bài tiếp theo sẽ tự phát.
4. Trong trang tìm kiếm, nút phát dùng để nghe riêng một bài. Muốn nghe nhiều bài nối tiếp, dùng **Phát playlist**.
5. Có thể đổi tên, xóa playlist, xóa bài và đổi thứ tự. Bài thêm vào playlist đang phát được nối vào hàng đợi; đổi thứ tự áp dụng khi phát lại playlist. Xóa bài đang nghe không cắt ngang bài đó.
6. Nhấn **Chạy nền**, đóng cửa sổ hoặc Ctrl+W để ẩn giao diện và tiếp tục nghe. Mở lại app để hiện cửa sổ. Menu ☰ → **Thoát hoàn toàn** hoặc Ctrl+Q để thoát.

Link video kèm `list=` vẫn mở **một bài đó**. Bản này chưa nhập toàn bộ playlist từ URL `/playlist?list=…`. Link trang khác bị từ chối; không chuyển URL tùy ý cho bộ tải.

| Khi hết bài | Hành vi |
|---|---|
| Còn bài trong hàng đợi | Phát bài tiếp theo |
| Hết hàng đợi, **Tự phát đề xuất** tắt | Lặp bài hiện tại |
| Hết hàng đợi, **Tự phát đề xuất** bật | Lấy bài từ YouTube Mix; nếu không có thì tìm bài liên quan theo kênh/tên bài |
| Không tìm được đề xuất mới | Phát lại bài hiện tại |
| Bài lỗi | Bỏ qua nếu còn bài; hết hàng đợi thì dừng, không lặp lỗi vô hạn |

Đề xuất tránh 50 bài vừa nghe trong phiên, không tự thêm bài đề xuất vào playlist lưu trên máy. Kết quả không giống nguồn đề xuất cá nhân hóa khi đăng nhập YouTube. Không đồng bộ playlist với tài khoản YouTube ở bản đầu.

## Điều khiển nền

- Hỗ trợ MPRIS qua D-Bus: Linux có thể hiển thị bài đang nghe và play/pause/next trong menu âm thanh hoặc điều khiển media của desktop.
- Phím trong app: **Ctrl+Space** phát/tạm dừng, **Ctrl+Right/Left** chuyển bài, **Ctrl+W** ẩn, **Ctrl+Q** thoát.
- Có tua, âm lượng và nhớ playlist, âm lượng, chế độ đề xuất, bài nghe gần nhất. Khi mở lại app, nhạc không tự phát cho đến khi cậu nhấn phát.
- Bản đầu chưa có biểu tượng khay hệ thống riêng; dùng menu media của desktop hoặc mở lại launcher để điều khiển.

## Tài nguyên

- Phát luồng audio, không tải/giải mã video và không chuyển mã sang MP3.
- Một tiến trình mpv khi phát; yt-dlp và JavaScript runtime chỉ chạy khi cần tìm/lấy thông tin.
- Đọc trước tối đa khoảng 90 giây, giới hạn bộ đệm mpv 8 MiB phía trước + 1 MiB phía sau. Tải luồng YouTube theo các đoạn tối đa 1 MiB để tránh giảm tốc request lớn; không tải toàn bộ video vào RAM hay lưu thành file.
- Dùng sự kiện và channel, không quét mạng định kỳ. Hủy yêu cầu cũ khi tìm lại/đổi bài, có thời hạn cho tác vụ mạng và dọn tiến trình khi thoát.
- Launcher mặc định dùng Cairo và tắt GL/Vulkan cho giao diện để giảm RAM. Có thể ghi đè bằng `GDK_DISABLE='' GSK_RENDERER=gl ./run.sh` nếu muốn dùng GPU và hệ thống hỗ trợ.
- Đoạn tải bị cắt được thử lại có giới hạn. Nếu luồng đang nghe vẫn đứt hoặc báo EOF sớm, app lấy lại URL và nối tiếp tại vị trí vừa nghe (tối đa 2 lần mỗi bài), giữ trạng thái pause. EOF sớm không bị hiểu thành hết bài để chuyển bài/lặp từ đầu.
- Khi thiếu dữ liệu, trạng thái hiển thị “Đang nạp thêm âm thanh…”; đây khác với thao tác tạm dừng của người dùng.
- yt-dlp kiểm tra định dạng âm thanh thực sự tải được trước khi giao URL cho mpv; việc bắt đầu bài có thể mất thêm vài giây.
- Ẩn cửa sổ giữ lại GTK để mở lại nhanh; không có WebView. Mức RAM thực tế cần tính cả mpv/yt-dlp và sẽ tăng tạm thời khi tìm kiếm hoặc chuẩn bị phát.

Trong lượt thử release sau khi mở rồi ẩn cửa sổ, tổng app + mpv khoảng **115 MiB RSS / 56 MiB PSS**. Đây là số đo trên màn hình ảo, không phải giới hạn RAM cam kết; xem [VERIFICATION.md](VERIFICATION.md) để biết điều kiện đo và kết quả kiểm thử.

## Cài dependency / build lại

Trên Ubuntu/Debian:

```bash
sudo apt install build-essential pkg-config libgtk-4-dev mpv python3
```

Cài Rust stable bằng hướng dẫn tại https://rustup.rs, và yt-dlp mới từ https://github.com/yt-dlp/yt-dlp#installation. YouTube hiện cần JavaScript runtime được yt-dlp hỗ trợ, như **Node.js 22+** hoặc Deno; chi tiết tại https://github.com/yt-dlp/yt-dlp/wiki/EJS.

```bash
./scripts/build.sh
./run.sh
```

`Cargo.lock` được giữ để build lặp lại. SQLite được build kèm, không cần `libsqlite3-dev`. Source dùng Rust edition 2024 và let-chains; cần Rust 1.88+ hoặc mới hơn theo yêu cầu dependency.

Để dùng mpv/yt-dlp riêng thay cho bản đi kèm:

```bash
MAY_MPV=/usr/bin/mpv MAY_YTDLP=/path/to/yt-dlp ./run.sh
```

## Dữ liệu và cấu hình

Database: `${XDG_DATA_HOME:-~/.local/share}/may-music/library.db`. Playlist chỉ lưu metadata và ID video; URL luồng tạm thời không được lưu vào database. Có thể sao lưu thư mục này sau khi thoát app.

| Biến môi trường | Mục đích |
|---|---|
| `MAY_DATA_DIR` | Đổi thư mục dữ liệu để thử nghiệm riêng |
| `MAY_MPV`, `MAY_YTDLP` | Chọn executable khác |
| `MAY_YTDLP_CONFIG` | Chỉ định file cấu hình yt-dlp khi cần thiết |
| `MAY_MPV_AO` | Chọn audio output; `null` dành cho kiểm thử im lặng |
| `MAY_MPV_LOG` | Ghi log mpv phục vụ chẩn đoán; log có thể chứa URL phát tạm thời |

App bỏ qua cấu hình yt-dlp toàn cục trừ file được chỉ định rõ qua `MAY_YTDLP_CONFIG`. Không tự đọc cookie từ trình duyệt. Nếu YouTube từ chối một bài, thử bài khác và kiểm tra/cập nhật yt-dlp; một số nội dung cần điều kiện truy cập mà bản mặc định không có.

## Kiểm thử

```bash
cargo test --locked
cargo clippy --all-targets -- -D warnings

# Cần mpv thật; phát file WAV tạo tại chỗ, không có âm thanh:
MAY_TEST_MPV=1 MAY_MPV_AO=null cargo test --lib real_mpv -- --nocapture

# Cần Xvfb + D-Bus + python3, kiểm thử giao diện bằng fixture offline:
xvfb-run -a dbus-run-session -- cargo test --bin may-music -- --ignored --nocapture

# Kiểm tra mạng thật: tìm YouTube, lấy audio và phát vài giây rồi dừng:
MAY_MPV_AO=null cargo run --example check_youtube -- 'lofi chill'

# Kiểm tra đề xuất từ nguồn thật, không phát nhạc:
cargo run --example check_radio -- 'Kevin MacLeod Carefree'
```

Kiểm thử toàn luồng dùng executable fixture trong `tests/fixtures/` để chủ động báo EOF/lỗi mà không phụ thuộc YouTube. App sử dụng mpv/yt-dlp thật theo mặc định. Ảnh `preview.png` được render từ app GTK đang chạy với dữ liệu kiểm thử.

## Cấu trúc

`src/model.rs`: playlist, hàng đợi, chọn đề xuất. `store.rs`: SQLite. `youtube.rs`: tác vụ yt-dlp có timeout/hủy. `relay.rs`: cầu HTTP trên loopback, chia request YouTube thành các đoạn nhỏ, hỗ trợ Range/tua và hủy kết nối. `player.rs`: mpv và JSON IPC trong thư mục socket riêng. `engine.rs`: điều phối phát, khôi phục đúng vị trí, bỏ kết quả cũ khi đổi bài. `ui.rs`: GTK4. `mpris.rs`: điều khiển media Linux. `main.rs`: single-instance và lệnh điều khiển.

Tài liệu nền: https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/ · https://mpv.io/manual/stable/ · https://github.com/yt-dlp/yt-dlp
# May-Music
