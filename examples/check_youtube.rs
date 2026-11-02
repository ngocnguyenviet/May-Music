//! Live diagnostic: cargo run --example check_youtube -- 'artist song'
//! Use MAY_MPV_AO=null for a silent network test.
use anyhow::{Context, Result, bail};
use may_music::{
    player::{Player, PlayerEvent},
    youtube::{AudioStream, Youtube, cancellation},
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

fn main() -> Result<()> {
    let query = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let query = if query.is_empty() {
        "lofi chill"
    } else {
        &query
    };
    let yt = Youtube::default();
    let token = cancellation();
    let url = if let Ok(path) = std::env::var("MAY_CHECK_SOURCE_JSON") {
        let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        AudioStream {
            url: data["url"].as_str().context("Missing URL")?.to_string(),
            headers: serde_json::from_value(data["http_headers"].clone())?,
            content_length: data["filesize"].as_u64(),
        }
    } else {
        let tracks = yt.search(query, &token)?;
        println!("Search: {} results", tracks.len());
        let track = tracks.first().context("No search results")?;
        println!("Track: {} — {}", track.title, track.channel);
        yt.resolve(track, &token)?
    };
    println!("Resolved audio stream (URL omitted)");
    if let Ok(path) = std::env::var("MAY_CHECK_SAVE_JSON") {
        std::fs::write(
            path,
            serde_json::to_vec(
                &serde_json::json!({"url": url.url, "http_headers": url.headers, "filesize":url.content_length}),
            )?,
        )?;
    }
    let (tx, rx) = mpsc::channel();
    let mut player = Player::start(50.0, move |event| {
        let _ = tx.send(event);
    })?;
    let seconds = std::env::var("MAY_CHECK_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(2);
    let start = std::env::var("MAY_CHECK_START")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    player.load_stream_at(&url, false, start)?;
    let deadline = Instant::now() + Duration::from_secs(seconds + 30);
    let mut loaded = false;
    let mut retries = 0;
    let mut buffering = 0;
    let mut started_audio = false;
    let mut last_report = start;
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))? {
            PlayerEvent::Loaded => {
                loaded = true;
                println!("mpv: file loaded");
            }
            PlayerEvent::Buffering(true) if started_audio => {
                buffering += 1;
            }
            PlayerEvent::Position(pos) if loaded && pos >= start + seconds as f64 => {
                println!("mpv: audio advanced to {pos:.1}s");
                println!("Buffering transitions after load: {buffering}");
                break;
            }
            PlayerEvent::Position(pos) if loaded && pos > start + 0.1 => {
                started_audio = true;
                if pos - last_report >= 30.0 {
                    println!("mpv: progress {pos:.1}s");
                    last_report = pos;
                }
            }
            PlayerEvent::End {
                failed: true,
                detail,
            } => {
                if retries >= 2 {
                    bail!("mpv: {detail}");
                }
                retries += 1;
                println!("Retrying initial audio connection ({retries}/2)");
                std::thread::sleep(Duration::from_millis(500 * retries));
                player.load_stream(&url, false)?;
            }
            PlayerEvent::Disconnected => bail!("mpv disconnected"),
            _ => (),
        }
    }
    drop(player);
    println!("Live playback check passed; player stopped and reaped.");
    Ok(())
}
