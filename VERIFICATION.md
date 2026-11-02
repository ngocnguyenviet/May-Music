# Kiểm tra bản đầu

Môi trường: Ubuntu 26.04 x86_64, Rust 1.99.1, GTK 4.22.4, mpv 0.41.0, yt-dlp 2026.10.26, Node.js 24.22.0.

- `cargo test`: 17 kiểm thử thư viện + 1 kiểm thử toàn luồng đạt. Kiểm thử mpv thật được bật bằng `MAY_TEST_MPV=1`, phát WAV tạo tại chỗ qua audio output `null`.
- Kiểm thử GUI chạy riêng trong Xvfb + D-Bus: đạt. Tìm kiếm qua fixture, nhấn thêm hai bài, phát playlist, pause qua MPRIS, đóng cửa sổ vẫn tự chuyển bài, mở lại và render `preview.png`.
- `cargo clippy --all-targets -- -D warnings`: đạt.
- Kiểm tra nguồn thật: tìm YouTube → yt-dlp xác minh định dạng audio → mpv tải và phát qua mốc 2 giây: đạt, dùng audio output `null` để không phát ra loa.
- Kiểm tra đề xuất thật: từ “Carefree — Kevin MacLeod” nhận “Life of Riley — Kevin MacLeod”: đạt.
- Binary release: khởi động ẩn → lấy luồng bằng yt-dlp thật → phát YouTube bằng mpv thật → MPRIS báo Playing → lệnh single-instance mở/ẩn/pause → MPRIS báo Paused → thoát: đạt. App và toàn bộ tiến trình con đã kết thúc, không còn socket mpv của lượt thử.
- Launcher: `bash -n`, kiểm tra dependency bằng `run.sh --diagnose`, tạo launcher trong thư mục kiểm thử và `desktop-file-validate`: đạt.
- Cập nhật nhận URL: kiểm thử `watch`, `youtu.be`, YouTube Music, Shorts, embed, query mã hóa; từ chối hostname giả, URL mơ hồ và link chỉ có playlist: đạt. Engine nhận link trả đúng một ID, bỏ `list=` để không nhập cả danh sách. GUI dán link trả một bài: đạt.
- Link YouTube thật `youtu.be/…?si=…` → đúng một kết quả → mpv phát qua mốc 2 giây: đạt.
- Lệnh ngắn `may-music`: cài wrapper tại `~/.local/bin/may-music`, xác nhận `PATH` đã có thư mục này, chạy `--help` và `--diagnose`: đạt. Không sửa cấu hình shell.
- Tham số `--play`/`--pause`: kiểm thử trong phiên D-Bus riêng với binary thật và bộ phát giả. Play bắt đầu playlist đang chọn khi chưa có bài gần nhất; gọi Play nhiều lần không tải lại bài; Pause nhiều lần vẫn tạm dừng; Play sau Pause tiếp tục tại giây 26; Stop → Play phát từ đầu; khởi động mới bằng `--play` phát bài gần nhất; Quit dọn tiến trình con: đạt. `--help` và README mô tả đủ Play/Pause/Toggle/Stop.

## RAM đo trong lượt thử release

Đo qua `/proc/<pid>/smaps_rollup`, trong Xvfb với Cairo, GL/Vulkan tắt, audio output `null`. Số lúc phát được lấy **sau khi mở rồi ẩn lại cửa sổ**; yt-dlp đã hoàn thành lấy link và thoát.

| Trạng thái / tiến trình | RSS | PSS |
|---|---:|---:|
| App mới khởi động ẩn, chưa phát | 37.71 MiB | 11.36 MiB |
| App đang phát nền | 55.85 MiB | 23.66 MiB |
| mpv đang phát | 59.59 MiB | 31.89 MiB |
| Tổng lúc phát nền | **115.44 MiB** | **55.55 MiB** |

RSS cộng dồn có thể tính trùng các trang thư viện dùng chung; PSS phân bổ phần dùng chung giữa các tiến trình. Đây là số đo một lượt trong môi trường kiểm thử, không phải mức RAM cam kết. Desktop thật, audio output, kích thước playlist và bộ đệm có thể làm số khác đi. RAM lúc yt-dlp/JavaScript runtime đang chạy có thể tăng tạm thời; chưa đo đỉnh bộ nhớ của giai đoạn đó.

YouTube có thể trả lỗi theo bài hoặc thay đổi cơ chế truy cập. Bản đầu kiểm tra định dạng tải được, truyền HTTP header nguồn và giới hạn số lần kết nối lại; không đảm bảo phát mọi video. Đề xuất dùng Mix/tìm kiếm liên quan, chưa cá nhân hóa theo tài khoản.

## Sửa gián đoạn với video dài

Video kiểm tra: `PBeHXaCsR6s`, thời lượng 3905 giây, audio Opus khoảng 137.73 kb/s, dung lượng 67,229,967 byte.

Trên cùng URL nguồn và kết nối, đọc thử cho thấy:

| Cách tải | Dữ liệu đọc được | Thời gian |
|---|---:|---:|
| Request không giới hạn điểm cuối như bản trước | 163,840 byte | 13.72 giây |
| Request giới hạn `range=0-1048575` | 1,048,576 byte | 0.28 giây |

Đây là bằng chứng cách tải góp phần gây cạn bộ đệm với nguồn này; không suy ra mọi gián đoạn đều có cùng nguyên nhân. Tham khảo [FAQ yt-dlp về giới hạn chunk](https://github.com/yt-dlp/yt-dlp/wiki/FAQ#i-extracted-a-video-url-but-it-does-not-play-on-another-machine--in-my-web-browser).

Bản sửa tải từng đoạn tối đa 1 MiB qua cầu HTTP chỉ lắng nghe trên loopback, giữ hỗ trợ byte-range để tua. Có giới hạn kết nối, timeout và hủy tải; khi mpv chưa cần thêm dữ liệu, cầu tải giữ đúng vị trí byte thay vì làm mất/trùng dữ liệu. mpv đọc trước tối đa 90 giây trong giới hạn 8 MiB phía trước + 1 MiB phía sau. Khi luồng đứt thật hoặc EOF sớm, engine lấy lại nguồn và nối tiếp vị trí vừa nghe, tối đa hai lần, không nhầm thành hết bài.

Kiểm chứng:

- Phát im lặng từ đầu video qua mốc 120 giây: đạt, không ghi nhận chuyển sang buffering sau khi tải ban đầu.
- Bắt đầu tại giây 1800, phát tới 1820: đạt, không ghi nhận buffering trong đoạn đó.
- Máy chủ giả cắt giữa một chunk: tải lại đúng đoạn, nội dung không thiếu/trùng byte; đọc nhiều chunk và tua theo Range: đạt.
- Hủy khi upstream đang treo: đóng kết nối và dọn worker dưới 2 giây: đạt.
- Client đọc chậm gây timeout ghi từng phần: giữ nguyên toàn bộ byte, không thiếu/trùng: đạt.
- Engine nhận EOF ở giây 60/180: lấy lại URL và gửi `start=60`, không chuyển bài hoặc phát lại từ đầu: đạt.
- Trạng thái buffering không thay đổi trạng thái pause của người dùng; playlist, replay và đề xuất vẫn qua kiểm thử toàn luồng.

Chưa chạy thử liên tục trọn 65 phút. Kết quả trên xác nhận sửa cách tải bị nghẽn và các tình huống lỗi có kiểm soát, không phải cam kết không bao giờ gián đoạn mạng.
