use crate::model::{Track, choose_recommendation};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    env,
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub type Cancel = Arc<AtomicBool>;

#[derive(Clone, Debug)]
pub struct AudioStream {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub content_length: Option<u64>,
}
pub fn cancellation() -> Cancel {
    Arc::new(AtomicBool::new(false))
}
pub fn cancel(token: &Cancel) {
    token.store(true, Ordering::Relaxed);
}

#[derive(Clone)]
pub struct Youtube {
    pub binary: String,
}
impl Default for Youtube {
    fn default() -> Self {
        Self {
            binary: env::var("MAY_YTDLP").unwrap_or_else(|_| "yt-dlp".into()),
        }
    }
}

// Each job has a deadline, bounded output, and a cancellation token. yt-dlp is
// spawned directly with argv, never through a shell or a user-supplied command.
fn run(mut command: Command, token: &Cancel, timeout: Duration) -> Result<String> {
    if token.load(Ordering::Relaxed) {
        bail!("Đã hủy");
    }
    let mut child = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Không chạy được yt-dlp; kiểm tra cài đặt và PATH")?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut b = Vec::new();
        stdout
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut b)
            .map(|_| b)
    });
    let err = thread::spawn(move || {
        let mut b = Vec::new();
        stderr.take(64 * 1024).read_to_end(&mut b).map(|_| b)
    });
    let start = Instant::now();
    let status = loop {
        if token.load(Ordering::Relaxed) || start.elapsed() > timeout {
            // Also stop short-lived JS runtimes spawned by yt-dlp.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            bail!(if token.load(Ordering::Relaxed) {
                "Đã hủy"
            } else {
                "YouTube phản hồi quá lâu; thử lại sau"
            });
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let bytes = out
        .join()
        .map_err(|_| anyhow::anyhow!("Lỗi đọc kết quả yt-dlp"))??;
    let errors = err
        .join()
        .map_err(|_| anyhow::anyhow!("Lỗi đọc thông báo yt-dlp"))??;
    if !status.success() {
        let message: String = String::from_utf8_lossy(&errors)
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(500)
            .collect();
        bail!(
            "YouTube: {}",
            if message.trim().is_empty() {
                "không lấy được bài"
            } else {
                message.trim()
            }
        );
    }
    if bytes.len() > 4 * 1024 * 1024 {
        bail!("Kết quả YouTube quá lớn");
    }
    Ok(String::from_utf8(bytes)?)
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

// Return a single canonical video ID, or None for ordinary search keywords.
// Never forward arbitrary URLs to yt-dlp: watch links with `list=` still mean
// just the explicitly selected video, not an entire playlist.
pub fn youtube_video_id(input: &str) -> Result<Option<String>> {
    let input = input.trim();
    let lower = input.to_ascii_lowercase();
    let bare = [
        "youtube.com/",
        "www.youtube.com/",
        "m.youtube.com/",
        "music.youtube.com/",
        "youtu.be/",
        "www.youtu.be/",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix));
    if !input.contains("://") && !input.starts_with("//") && !bare {
        return Ok(None);
    }
    if input.len() > 4096 {
        bail!("Link YouTube quá dài");
    }
    let normalized = if input.starts_with("//") {
        format!("https:{input}")
    } else if bare {
        format!("https://{input}")
    } else {
        input.to_string()
    };
    let uri = gtk::glib::Uri::parse(&normalized, gtk::glib::UriFlags::ENCODED)
        .context("Link không hợp lệ; hãy dán URL video YouTube đầy đủ")?;
    let scheme = uri.scheme().to_ascii_lowercase();
    let host = uri.host().unwrap_or_default().to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https")
        || uri.userinfo().is_some()
        || !matches!(uri.port(), -1 | 80 | 443)
    {
        bail!("Chỉ hỗ trợ link video YouTube HTTP/HTTPS");
    }
    if !matches!(
        host.as_str(),
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtu.be"
            | "www.youtu.be"
            | "youtube-nocookie.com"
            | "www.youtube-nocookie.com"
    ) {
        bail!("Link này không thuộc YouTube; dùng youtube.com hoặc youtu.be");
    }
    let path = uri.path();
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    let id = if matches!(host.as_str(), "youtu.be" | "www.youtu.be") && parts.len() == 1 {
        Some(parts[0].to_string())
    } else if parts.as_slice() == ["watch"] {
        let mut ids = vec![];
        for pair in uri.query().unwrap_or_default().split('&') {
            if let Some((key, value)) = pair.split_once('=')
                && gtk::glib::Uri::unescape_string(key, None).is_some_and(|key| key == "v")
            {
                ids.push(
                    gtk::glib::Uri::unescape_string(value, None)
                        .unwrap_or_default()
                        .to_string(),
                );
            }
        }
        if ids.len() == 1 { ids.pop() } else { None }
    } else if parts.len() == 2 && matches!(parts[0], "shorts" | "embed" | "live" | "v") {
        gtk::glib::Uri::unescape_string(parts[1], None).map(|s| s.to_string())
    } else {
        None
    };
    match id.filter(|id| valid_id(id)) {
        Some(id) => Ok(Some(id)),
        None => bail!(
            "Chưa thấy ID bài trong link. Hãy dùng link một video, ví dụ youtube.com/watch?v=… hoặc youtu.be/…; bản này chưa nhập cả playlist bằng URL."
        ),
    }
}

pub fn parse_tracks(json: &str) -> Result<Vec<Track>> {
    let data: Value = serde_json::from_str(json).context("Không đọc được kết quả YouTube")?;
    let entries = data
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![data]);
    let mut tracks = Vec::new();
    for v in entries {
        let Some(id) = v["id"].as_str().filter(|id| valid_id(id)) else {
            continue;
        };
        if matches!(v["live_status"].as_str(), Some("is_live" | "is_upcoming")) {
            continue;
        }
        if matches!(
            v["availability"].as_str(),
            Some("private" | "premium_only" | "subscriber_only" | "needs_auth")
        ) {
            continue;
        }
        if tracks.iter().any(|t: &Track| t.id == id) {
            continue;
        }
        let Some(title) = v["title"].as_str() else {
            continue;
        };
        if matches!(title, "[Deleted video]" | "[Private video]") {
            continue;
        }
        tracks.push(Track {
            id: id.into(),
            title: title.chars().take(300).collect(),
            channel: v["channel"]
                .as_str()
                .or_else(|| v["uploader"].as_str())
                .unwrap_or("YouTube")
                .chars()
                .take(120)
                .collect(),
            duration: v["duration"]
                .as_f64()
                .filter(|d| d.is_finite() && *d > 0.0)
                .map(|d| d as u64),
        });
        if tracks.len() == 20 {
            break;
        }
    }
    Ok(tracks)
}

impl Youtube {
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.args([
            "--ignore-config",
            "--no-warnings",
            "--no-progress",
            "--socket-timeout",
            "12",
            "--retries",
            "1",
            "--extractor-retries",
            "1",
            "--js-runtimes",
            "node",
        ]);
        if let Ok(path) = env::var("MAY_YTDLP_CONFIG") {
            cmd.args(["--config-locations", &path]);
        }
        cmd
    }
    fn list(&self, query: &str, token: &Cancel) -> Result<Vec<Track>> {
        let mut cmd = self.command();
        cmd.args([
            "--flat-playlist",
            "--dump-single-json",
            "--skip-download",
            "--playlist-end",
            "20",
            "--",
            query,
        ]);
        parse_tracks(&run(cmd, token, Duration::from_secs(45))?)
    }
    pub fn search(&self, query: &str, token: &Cancel) -> Result<Vec<Track>> {
        if let Some(id) = youtube_video_id(query)? {
            let mut cmd = self.command();
            let url = format!("https://www.youtube.com/watch?v={id}");
            cmd.args([
                "--no-playlist",
                "--skip-download",
                "--dump-single-json",
                "--ignore-no-formats-error",
                "--",
                &url,
            ]);
            let mut tracks = parse_tracks(&run(cmd, token, Duration::from_secs(45))?)?;
            tracks.retain(|track| track.id == id);
            if tracks.is_empty() {
                bail!(
                    "Không lấy được bài từ link này; video có thể không công khai hoặc không khả dụng"
                );
            }
            return Ok(tracks);
        }
        let query: String = query.trim().chars().take(160).collect();
        if query.is_empty() {
            return Ok(vec![]);
        }
        self.list(&format!("ytsearch16:{query}"), token)
    }
    pub fn resolve(&self, track: &Track, token: &Cancel) -> Result<AudioStream> {
        if !valid_id(&track.id) {
            bail!("ID YouTube không hợp lệ");
        }
        let mut cmd = self.command();
        cmd.args([
            "--no-playlist",
            "--format",
            "bestaudio",
            "--check-formats",
            "--dump-single-json",
            "--skip-download",
            "--",
            &track.url(),
        ]);
        let result = run(cmd, token, Duration::from_secs(50))?;
        let data: Value =
            serde_json::from_str(&result).context("Không đọc được thông tin luồng nhạc")?;
        let url = data["url"].as_str().unwrap_or_default().trim();
        if !url.starts_with("https://") || url.chars().any(char::is_whitespace) {
            bail!("Không tìm được luồng âm thanh HTTPS");
        }
        let mut headers = BTreeMap::new();
        if let Some(values) = data["http_headers"].as_object() {
            for (name, value) in values {
                if let Some(value) = value.as_str()
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !value.contains(['\r', '\n'])
                {
                    headers.insert(name.clone(), value.to_string());
                }
            }
        }
        Ok(AudioStream {
            url: url.into(),
            headers,
            content_length: data["filesize"].as_u64(),
        })
    }
    pub fn related(
        &self,
        seed: &Track,
        recent: &VecDeque<String>,
        token: &Cancel,
    ) -> Result<Track> {
        // YouTube's public Mix is best-effort; it is not a personal account feed.
        let mix = format!(
            "https://www.youtube.com/watch?v={}&list=RD{}",
            seed.id, seed.id
        );
        if let Ok(items) = self.list(&mix, token)
            && let Some(track) = choose_recommendation(items, &seed.id, recent)
        {
            return Ok(track);
        }
        if token.load(Ordering::Relaxed) {
            bail!("Đã hủy");
        }
        let query = if seed.channel == "YouTube" {
            seed.title.clone()
        } else {
            format!("{} music", seed.channel)
        };
        choose_recommendation(self.search(&query, token)?, &seed.id, recent)
            .context("Không có bài đề xuất mới; phát lại bài hiện tại")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_video_links_and_canonicalizes_exact_id() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PLignored&t=45",
            "https://youtu.be/dQw4w9WgXcQ?si=shared-link",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "youtu.be/dQw4w9WgXcQ",
            "//www.youtube.com/watch?v=%64Qw4w9WgXcQ#details",
            "HTTPS://WWW.YOUTUBE.COM/watch?v=dQw4w9WgXcQ",
        ] {
            assert_eq!(
                youtube_video_id(url).unwrap(),
                Some("dQw4w9WgXcQ".into()),
                "{url}"
            );
        }
        assert_eq!(youtube_video_id("lofi chill buổi tối").unwrap(), None);
    }
    #[test]
    fn rejects_unrelated_or_ambiguous_urls_and_playlist_only_links() {
        for url in [
            "https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ",
            "https://youtube.com@evil.test/watch?v=dQw4w9WgXcQ",
            "https://evil.test@youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com:9999/watch?v=dQw4w9WgXcQ",
            "file:///etc/passwd",
            "https://www.youtube.com/playlist?list=PLtest",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&v=music000001",
            "https://www.youtube.com/watch?v=bad-id",
            "https://www.youtube.com/redirect?q=https://example.test",
        ] {
            assert!(youtube_video_id(url).is_err(), "{url}");
        }
    }
    #[test]
    fn parses_flat_results_and_filters_live_private_duplicates() {
        let json = r#"{"entries":[null,{"id":"abcdefghijk","title":"Nhạc ☁","channel":"Ca sĩ","duration":123.5},{"id":"abcdefghijk","title":"Duplicate"},{"id":"12345678901","title":"Live","live_status":"is_live"},{"id":"12345678902","title":"Private","availability":"private"},{"id":"bad/url","title":"Bad"}]}"#;
        let result = parse_tracks(json).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duration, Some(123));
        assert_eq!(result[0].title, "Nhạc ☁");
    }
    #[test]
    fn rejects_untrusted_video_ids() {
        assert!(!valid_id("; rm -rf /"));
        assert!(!valid_id("../test.mp3"));
        assert!(valid_id("dQw4w9WgXcQ"));
    }
    #[test]
    fn canceled_job_does_not_start() {
        let token = cancellation();
        cancel(&token);
        assert!(
            run(
                Command::new("does-not-exist"),
                &token,
                Duration::from_secs(1)
            )
            .unwrap_err()
            .to_string()
            .contains("Đã hủy")
        );
    }
}
