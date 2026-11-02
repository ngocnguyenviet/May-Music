use may_music::{
    engine::{self, Command, Handle, Phase, Snapshot, UiEvent},
    model::Track,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};

#[track_caller]
fn wait_state(
    rx: &async_channel::Receiver<UiEvent>,
    predicate: impl Fn(&Snapshot) -> bool,
) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Ok(UiEvent::State(snapshot)) = rx.try_recv()
            && predicate(&snapshot)
        {
            return *snapshot;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("Timed out waiting for engine state");
}
fn wait_search(rx: &async_channel::Receiver<UiEvent>) -> Vec<Track> {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Ok(UiEvent::SearchResults(tracks)) = rx.try_recv() {
            return tracks;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("Timed out waiting for search");
}
fn eof(dir: &Path) {
    fs::write(dir.join("player-event"), "eof").unwrap();
}
fn current(s: &Snapshot, id: &str) -> bool {
    s.phase == Phase::Playing && s.current.as_ref().is_some_and(|t| t.id == id)
}
fn add(handle: &Handle, rx: &async_channel::Receiver<UiEvent>, track: Track, count: usize) {
    handle.send(Command::Add(track));
    wait_state(rx, |s| s.library.selected().unwrap().tracks.len() == count);
}

// One test owns the process environment before launching any worker threads.
#[test]
fn search_playlist_background_engine_next_replay_radio_cancel_errors_and_persistence() {
    let temp = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["yt-dlp.py", "mpv.py"] {
        fs::set_permissions(fixtures.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
    unsafe {
        std::env::set_var("MAY_TEST_DIR", temp.path());
        std::env::set_var("MAY_DATA_DIR", temp.path().join("data"));
        std::env::set_var("MAY_YTDLP", fixtures.join("yt-dlp.py"));
        std::env::set_var("MAY_MPV", fixtures.join("mpv.py"));
    }
    let (h, rx, worker) = engine::launch().unwrap();
    wait_state(&rx, |s| s.phase == Phase::Stopped);
    h.send(Command::Search(
        "https://youtu.be/music000002?list=PLignored&t=10".into(),
    ));
    let linked = wait_search(&rx);
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].id, "music000002");
    let log = fs::read_to_string(temp.path().join("youtube.log")).unwrap();
    assert!(log.contains("https://www.youtube.com/watch?v=music000002"));
    assert!(log.contains("--no-playlist"));
    assert!(!log.contains("PLignored"));
    h.send(Command::Search("slow old query".into()));
    h.send(Command::Search(
        "new query; $(touch SHOULD_NOT_EXIST)".into(),
    ));
    let tracks = wait_search(&rx);
    assert_eq!(tracks.len(), 2);
    add(&h, &rx, tracks[0].clone(), 1);
    add(&h, &rx, tracks[1].clone(), 2);
    // Explicit Play starts the selected playlist if nothing has played yet.
    h.send(Command::Pause(false));
    wait_state(&rx, |s| current(s, "music000001"));
    let count = fs::read_to_string(temp.path().join("loads.log"))
        .unwrap()
        .lines()
        .count();
    h.send(Command::Pause(false));
    wait_state(&rx, |s| current(s, "music000001"));
    assert_eq!(
        fs::read_to_string(temp.path().join("loads.log"))
            .unwrap()
            .lines()
            .count(),
        count
    );
    h.send(Command::Pause(true));
    wait_state(&rx, |s| s.phase == Phase::Paused);
    h.send(Command::Pause(false));
    wait_state(&rx, |s| current(s, "music000001"));
    h.send(Command::Stop);
    let stopped = wait_state(&rx, |s| s.phase == Phase::Stopped);
    assert_eq!(stopped.position, 0.0);
    // Simulate a transient refusal during the first connection.
    fs::write(temp.path().join("fail-on-load"), "").unwrap();
    h.send(Command::PlayPlaylist(0));
    wait_state(&rx, |s| current(s, "music000001"));
    h.send(Command::Pause(true));
    wait_state(&rx, |s| s.phase == Phase::Paused);
    h.send(Command::Toggle);
    wait_state(&rx, |s| s.phase == Phase::Playing);
    eof(temp.path());
    wait_state(&rx, |s| current(s, "music000002"));
    // End of playlist with radio OFF repeats the current track.
    eof(temp.path());
    wait_state(&rx, |s| s.phase == Phase::Loading);
    wait_state(&rx, |s| current(s, "music000002"));
    // Radio does not change the saved playlist, and avoids recently heard items.
    h.send(Command::Autoplay(true));
    wait_state(&rx, |s| s.library.autoplay);
    eof(temp.path());
    let radio = wait_state(&rx, |s| current(s, "radio000001"));
    assert_eq!(radio.library.selected().unwrap().tracks.len(), 2);
    eof(temp.path());
    wait_state(&rx, |s| current(s, "radio000002"));
    // No new recommendation falls back to replay without a request loop.
    fs::write(temp.path().join("no-radio"), "").unwrap();
    eof(temp.path());
    wait_state(&rx, |s| s.phase == Phase::Recommending);
    wait_state(&rx, |s| current(s, "radio000002"));
    // Latest playback request wins, even if old extraction completes later.
    let mut slow = tracks[0].clone();
    slow.id = "slow0000001".into();
    h.send(Command::Play(slow));
    h.send(Command::Play(tracks[0].clone()));
    wait_state(&rx, |s| current(s, "music000001"));
    std::thread::sleep(Duration::from_millis(250));
    h.send(Command::Autoplay(false));
    wait_state(&rx, |s| current(s, "music000001") && !s.library.autoplay);
    // Cache stalls have their own status, without turning into a user pause.
    fs::write(temp.path().join("player-event"), "buffering").unwrap();
    wait_state(&rx, |s| {
        current(s, "music000001") && s.status.contains("nạp thêm")
    });
    fs::write(temp.path().join("player-event"), "buffered").unwrap();
    wait_state(&rx, |s| {
        current(s, "music000001") && s.status.contains("Đang phát")
    });
    // A truncated response must refresh the URL and resume, not replay/advance.
    fs::write(temp.path().join("player-event"), "premature").unwrap();
    wait_state(&rx, |s| {
        s.phase == Phase::Loading && s.status.contains("kết nối lại")
    });
    let recovered = wait_state(&rx, |s| current(s, "music000001") && s.position >= 60.0);
    assert_eq!(recovered.position, 60.0);
    let loads = fs::read_to_string(temp.path().join("loads.log")).unwrap();
    let last: serde_json::Value = serde_json::from_str(loads.lines().last().unwrap()).unwrap();
    assert_eq!(last[4]["start"], "60");
    // Even persistent stream errors get at most two reconnect attempts.
    let before = fs::read_to_string(temp.path().join("mpv.log"))
        .unwrap()
        .lines()
        .count();
    fs::write(temp.path().join("always-fail-on-load"), "").unwrap();
    h.send(Command::Play(tracks[0].clone()));
    wait_state(&rx, |s| {
        s.phase == Phase::Stopped && s.status.contains("Không phát được")
    });
    let after = fs::read_to_string(temp.path().join("mpv.log"))
        .unwrap()
        .lines()
        .count();
    assert_eq!(after - before, 3);
    fs::remove_file(temp.path().join("always-fail-on-load")).unwrap();
    // Failed final song stops; it must not replay or request radio forever.
    let mut bad = tracks[0].clone();
    bad.id = "badbadbad00".into();
    h.send(Command::Play(bad));
    wait_state(&rx, |s| {
        s.phase == Phase::Stopped && s.status.contains("Không phát được")
    });
    h.send(Command::Volume(35.0));
    wait_state(&rx, |s| s.library.volume == 35.0);
    drop(worker);
    for line in fs::read_to_string(temp.path().join("mpv.log"))
        .unwrap()
        .lines()
    {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(
            !Path::new(&format!("/proc/{}", v["pid"].as_u64().unwrap())).exists(),
            "orphan mpv"
        );
        assert!(!Path::new(v["socket"].as_str().unwrap()).exists());
    }
    let (_, rx, worker) = engine::launch().unwrap();
    let s = wait_state(&rx, |_| true);
    assert_eq!(s.library.selected().unwrap().tracks.len(), 2);
    assert_eq!(s.library.volume, 35.0);
    assert!(!s.library.autoplay);
    assert_eq!(s.phase, Phase::Stopped); // Never starts playing without consent.
    drop(worker);
}
