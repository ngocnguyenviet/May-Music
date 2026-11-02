use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    env,
    fs::{self, DirBuilder},
    io::{BufRead, BufReader, Write},
    os::unix::{fs::DirBuilderExt, net::UnixStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
pub enum PlayerEvent {
    Loaded,
    Position(f64),
    Duration(f64),
    Paused(bool),
    Buffering(bool),
    End { failed: bool, detail: String },
    Disconnected,
}

pub struct Player {
    child: Child,
    stream: UnixStream,
    directory: PathBuf,
    relay: Option<crate::relay::Relay>,
}

impl Player {
    pub fn start(volume: f64, on_event: impl Fn(PlayerEvent) + Send + 'static) -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let directory = PathBuf::from(format!("/tmp/may-{}-{nonce:x}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&directory)?;
        let socket = directory.join("mpv.sock");
        let mut command = Command::new(env::var_os("MAY_MPV").unwrap_or_else(|| "mpv".into()));
        command
            .args([
                "--no-config",
                "--idle=yes",
                "--no-video",
                "--no-audio-display",
                "--osc=no",
                "--load-scripts=no",
                "--load-auto-profiles=no",
                "--load-stats-overlay=no",
                "--osd-level=0",
                "--tls-verify=yes",
                "--network-timeout=20",
                "--no-terminal",
                "--ytdl=no",
                "--input-terminal=no",
                "--cache=yes",
                "--cache-secs=90",
                "--cache-pause-initial=yes",
                "--cache-pause-wait=5",
                "--demuxer-max-bytes=8MiB",
                "--demuxer-max-back-bytes=1MiB",
                "--stream-lavf-o=reconnect=1,reconnect_on_network_error=1,reconnect_delay_max=2,reconnect_max_retries=2,reconnect_delay_total_max=6",
                "--audio-client-name=may-music",
            ])
            .arg(format!("--volume={}", volume.clamp(0.0, 100.0)))
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(ao) = env::var("MAY_MPV_AO") {
            command.arg(format!("--ao={ao}"));
        }
        if let Ok(path) = env::var("MAY_MPV_LOG") {
            command.arg(format!("--log-file={path}"));
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(e).context("Không chạy được mpv; hãy cài mpv và mở lại app");
            }
        };
        let start = Instant::now();
        let stream = loop {
            match UnixStream::connect(&socket) {
                Ok(stream) => break stream,
                Err(_) if start.elapsed() < Duration::from_secs(4) => {
                    if child.try_wait()?.is_some() {
                        let _ = fs::remove_dir_all(&directory);
                        bail!("mpv không khởi động được; kiểm tra phiên bản và thư viện hệ thống");
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = fs::remove_dir_all(&directory);
                    return Err(e).context("Không kết nối được mpv");
                }
            }
        };
        let mut player = Self {
            child,
            stream,
            directory,
            relay: None,
        };
        player
            .stream
            .set_write_timeout(Some(Duration::from_secs(2)))?;
        let reader = BufReader::new(player.stream.try_clone()?);
        thread::spawn(move || {
            let mut last_second = -1;
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                match v["event"].as_str() {
                    Some("file-loaded") => on_event(PlayerEvent::Loaded),
                    Some("property-change") => match v["name"].as_str() {
                        Some("time-pos") => {
                            if let Some(pos) = v["data"].as_f64()
                                && pos as i64 != last_second
                            {
                                last_second = pos as i64;
                                on_event(PlayerEvent::Position(pos));
                            }
                        }
                        Some("duration") => {
                            if let Some(d) = v["data"].as_f64() {
                                on_event(PlayerEvent::Duration(d));
                            }
                        }
                        Some("pause") => {
                            if let Some(p) = v["data"].as_bool() {
                                on_event(PlayerEvent::Paused(p));
                            }
                        }
                        Some("paused-for-cache") => {
                            if let Some(buffering) = v["data"].as_bool() {
                                on_event(PlayerEvent::Buffering(buffering));
                            }
                        }
                        _ => (),
                    },
                    Some("end-file") => match v["reason"].as_str() {
                        Some("eof") => on_event(PlayerEvent::End {
                            failed: false,
                            detail: String::new(),
                        }),
                        Some("error") => on_event(PlayerEvent::End {
                            failed: true,
                            detail: v["file_error"]
                                .as_str()
                                .unwrap_or("Lỗi phát âm thanh")
                                .into(),
                        }),
                        _ => (), // stop, replace and quit must never advance the queue.
                    },
                    _ => (),
                }
            }
            on_event(PlayerEvent::Disconnected);
        });
        for (id, name) in [
            (1, "time-pos"),
            (2, "duration"),
            (3, "pause"),
            (4, "paused-for-cache"),
        ] {
            player.command(json!(["observe_property", id, name]))?;
        }
        Ok(player)
    }
    pub fn command(&mut self, args: Value) -> Result<()> {
        let data = serde_json::to_vec(&json!({"command":args}))?;
        self.stream.write_all(&data)?;
        self.stream.write_all(b"\n")?;
        Ok(())
    }
    pub fn load(&mut self, url: &str, paused: bool) -> Result<()> {
        self.load_at(url, paused, 0.0)
    }
    fn load_at(&mut self, url: &str, paused: bool, position: f64) -> Result<()> {
        self.command(json!(["set_property", "pause", paused]))?;
        self.command(
            json!(["loadfile", url, "replace", -1, {"start": position.max(0.0).to_string()}]),
        )
    }
    pub fn load_stream(
        &mut self,
        source: &crate::youtube::AudioStream,
        paused: bool,
    ) -> Result<()> {
        self.load_stream_at(source, paused, 0.0)
    }
    pub fn load_stream_at(
        &mut self,
        source: &crate::youtube::AudioStream,
        paused: bool,
        position: f64,
    ) -> Result<()> {
        self.relay.take();
        if crate::relay::supports_chunks(source) {
            let relay = crate::relay::Relay::start(source.clone())?;
            let url = relay.url();
            self.relay = Some(relay);
            self.command(json!(["set_property", "http-header-fields", []]))?;
            return self.load_at(&url, paused, position);
        }
        let mut headers = Vec::new();
        for (name, value) in &source.headers {
            if name.eq_ignore_ascii_case("User-Agent") {
                self.command(json!(["set_property", "user-agent", value]))?;
            } else {
                headers.push(format!("{name}: {value}"));
            }
        }
        // Send an array through JSON IPC: Accept headers may contain commas,
        // which must not be split as mpv command-line list separators.
        self.command(json!(["set_property", "http-header-fields", headers]))?;
        self.load_at(&source.url, paused, position)
    }
    pub fn pause(&mut self, paused: bool) -> Result<()> {
        self.command(json!(["set_property", "pause", paused]))
    }
    pub fn volume(&mut self, volume: f64) -> Result<()> {
        self.command(json!(["set_property", "volume", volume.clamp(0.0, 100.0)]))
    }
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        self.command(json!(["seek", seconds.max(0.0), "absolute"]))
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Always reap mpv, including when switching songs or exiting during load.
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.relay.take();
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_mpv_load_seek_pause_eof_and_cleanup() {
        if env::var_os("MAY_TEST_MPV").is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("tone.wav");
        let samples = 8000u32 * 2;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + samples * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((samples * 2).to_le_bytes());
        bytes.resize(44 + samples as usize * 2, 0);
        fs::write(&wav, bytes).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut p = Player::start(20.0, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let socket_dir = p.directory.clone();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(
                &fs::metadata(&socket_dir).unwrap().permissions()
            ) & 0o777,
            0o700
        );
        p.load(wav.to_str().unwrap(), true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let e = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if matches!(e, PlayerEvent::Loaded) {
                break;
            }
        }
        p.volume(30.0).unwrap();
        p.seek(1.5).unwrap();
        p.pause(false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let e = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if matches!(e, PlayerEvent::End { failed: false, .. }) {
                break;
            }
        }
        drop(p);
        assert!(!socket_dir.exists());
    }
}
