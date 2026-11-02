use crate::{
    model::{AfterEnd, Library, Queue, Track, remember},
    player::{Player, PlayerEvent},
    store::{Store, data_dir},
    youtube::{self, AudioStream, Cancel, Youtube},
};
use anyhow::Result;
use std::{
    collections::VecDeque,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Stopped,
    Loading,
    Playing,
    Paused,
    Recommending,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub library: Library,
    pub revision: u64,
    pub current: Option<Track>,
    pub phase: Phase,
    pub position: f64,
    pub duration: f64,
    pub queue_remaining: usize,
    pub status: String,
}

#[derive(Clone, Debug)]
pub enum Command {
    Search(String),
    Add(Track),
    Select(u64),
    Create(String),
    Rename(String),
    DeletePlaylist,
    Remove(String),
    Move { index: usize, delta: i32 },
    Play(Track),
    PlayPlaylist(usize),
    Toggle,
    Pause(bool),
    Next,
    Previous,
    Stop,
    Autoplay(bool),
    Volume(f64),
    Seek(f64),
    SeekRelative(f64),
    Quit,
}

#[derive(Debug)]
pub enum UiEvent {
    State(Box<Snapshot>),
    Progress(f64, f64),
    SearchStarted,
    SearchResults(Vec<Track>),
    SearchError(String),
    Message(String),
}

enum Event {
    User(Command),
    SearchDone(u64, std::result::Result<Vec<Track>, String>),
    Resolved(u64, std::result::Result<AudioStream, String>),
    RetryStream(u64, AudioStream),
    Related(u64, std::result::Result<Track, String>),
    Player(u64, PlayerEvent),
}

#[derive(Clone)]
pub struct Handle {
    sender: mpsc::Sender<Event>,
}
impl Handle {
    pub fn send(&self, command: Command) {
        let _ = self.sender.send(Event::User(command));
    }
}

pub struct Worker {
    thread: Option<thread::JoinHandle<()>>,
    handle: Handle,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.handle.send(Command::Quit);
        if let Some(join) = self.thread.take() {
            let _ = join.join();
        }
    }
}

pub fn launch() -> Result<(Handle, async_channel::Receiver<UiEvent>, Worker)> {
    let store = Store::open(&data_dir().join("library.db"))?;
    let mut library = store.load()?;
    if !library.volume.is_finite() {
        library.volume = 70.0;
    }
    library.volume = library.volume.clamp(0.0, 100.0);
    let (tx, rx) = mpsc::channel();
    let (ui, events) = async_channel::unbounded();
    let handle = Handle { sender: tx.clone() };
    let engine = Engine {
        store,
        library,
        queue: Queue::default(),
        phase: Phase::Stopped,
        position: 0.0,
        duration: 0.0,
        paused: false,
        status: "Chọn một bài để bắt đầu".into(),
        revision: 0,
        generation: 0,
        search_generation: 0,
        player: None,
        youtube: Youtube::default(),
        search_cancel: youtube::cancellation(),
        play_cancel: youtube::cancellation(),
        jobs: vec![],
        recent: VecDeque::new(),
        cached: None,
        stream_retries: 0,
        resume_position: 0.0,
        buffering: false,
        sender: tx,
        ui,
    };
    let thread = thread::spawn(move || engine.run(rx));
    Ok((
        handle.clone(),
        events,
        Worker {
            thread: Some(thread),
            handle,
        },
    ))
}

struct Engine {
    store: Store,
    library: Library,
    queue: Queue,
    phase: Phase,
    position: f64,
    duration: f64,
    paused: bool,
    status: String,
    revision: u64,
    generation: u64,
    search_generation: u64,
    player: Option<Player>,
    youtube: Youtube,
    search_cancel: Cancel,
    play_cancel: Cancel,
    jobs: Vec<thread::JoinHandle<()>>,
    recent: VecDeque<String>,
    cached: Option<(String, AudioStream, Instant)>,
    stream_retries: u8,
    resume_position: f64,
    buffering: bool,
    sender: mpsc::Sender<Event>,
    ui: async_channel::Sender<UiEvent>,
}

impl Engine {
    fn emit(&self, event: UiEvent) {
        let _ = self.ui.send_blocking(event);
    }
    fn snapshot(&self) {
        self.emit(UiEvent::State(Box::new(Snapshot {
            library: self.library.clone(),
            revision: self.revision,
            current: self
                .queue
                .current()
                .cloned()
                .or_else(|| self.library.last_track.clone()),
            phase: self.phase,
            position: self.position,
            duration: self.duration,
            queue_remaining: self.queue.items.len().saturating_sub(self.queue.cursor + 1),
            status: self.status.clone(),
        })));
    }
    fn save(&mut self) {
        self.revision += 1;
        if let Err(e) = self.store.save(&self.library) {
            self.emit(UiEvent::Message(format!("Không lưu được playlist: {e}")));
        }
    }
    fn run(mut self, rx: mpsc::Receiver<Event>) {
        self.snapshot();
        while let Ok(event) = rx.recv() {
            self.jobs.retain(|j| !j.is_finished());
            match event {
                Event::User(Command::Quit) => break,
                Event::User(command) => {
                    self.control(command);
                    self.snapshot();
                }
                Event::SearchDone(g, result) if g == self.search_generation => match result {
                    Ok(tracks) => self.emit(UiEvent::SearchResults(tracks)),
                    Err(e) => self.emit(UiEvent::SearchError(e)),
                },
                Event::Resolved(g, result) if g == self.generation => {
                    match result {
                        Ok(url) => self.open_stream(url),
                        Err(error) => self.failed(error),
                    }
                    self.snapshot();
                }
                Event::RetryStream(g, stream) if g == self.generation => {
                    self.open_stream(stream);
                    self.snapshot();
                }
                Event::Related(g, result) if g == self.generation => {
                    // A song added during recommendation lookup still takes priority.
                    if self.queue.has_next() {
                        self.queue.cursor += 1;
                        self.begin();
                    } else if !self.library.autoplay {
                        self.begin();
                    } else {
                        match result {
                            Ok(track) => {
                                self.queue.items.push(track);
                                self.queue.cursor += 1;
                                self.queue.origin = None; // Radio must not mutate the saved playlist.
                                if self.queue.items.len() > 100 {
                                    self.queue.items.remove(0);
                                    self.queue.cursor -= 1;
                                }
                                self.begin();
                            }
                            Err(error) => {
                                self.emit(UiEvent::Message(format!(
                                    "{error}. Phát lại bài hiện tại."
                                )));
                                self.begin();
                            }
                        }
                    }
                    self.snapshot();
                }
                Event::Player(g, event) if g == self.generation => self.player_event(event),
                _ => (), // Results from canceled jobs / replaced players are discarded.
            }
        }
        youtube::cancel(&self.search_cancel);
        youtube::cancel(&self.play_cancel);
        self.player.take();
        self.save();
        for job in self.jobs {
            let _ = job.join();
        }
    }

    fn control(&mut self, command: Command) {
        match command {
            Command::Search(query) => {
                self.search_generation += 1;
                let generation = self.search_generation;
                youtube::cancel(&self.search_cancel);
                self.search_cancel = youtube::cancellation();
                let token = self.search_cancel.clone();
                let yt = self.youtube.clone();
                let tx = self.sender.clone();
                self.emit(UiEvent::SearchStarted);
                self.jobs.push(thread::spawn(move || {
                    let result = yt.search(&query, &token).map_err(|e| e.to_string());
                    let _ = tx.send(Event::SearchDone(generation, result));
                }));
            }
            Command::Add(track) => {
                if self.library.add(track.clone()) {
                    if self.queue.origin == Some(self.library.selected)
                        && !self.queue.items.iter().any(|t| t.id == track.id)
                    {
                        self.queue.items.push(track);
                    }
                    self.save();
                    self.emit(UiEvent::Message("Đã thêm vào playlist".into()));
                } else {
                    self.emit(UiEvent::Message("Bài này đã có trong playlist".into()));
                }
            }
            Command::Select(id) => {
                if self.library.playlists.iter().any(|p| p.id == id) {
                    self.library.selected = id;
                    self.save();
                }
            }
            Command::Create(name) => {
                if self.library.create(&name) {
                    self.save();
                }
            }
            Command::Rename(name) => {
                let name: String = name.trim().chars().take(80).collect();
                if !name.is_empty()
                    && let Some(p) = self
                        .library
                        .playlists
                        .iter_mut()
                        .find(|p| p.id == self.library.selected)
                {
                    p.name = name;
                    self.save();
                }
            }
            Command::DeletePlaylist => {
                if self.library.playlists.len() > 1 {
                    self.library
                        .playlists
                        .retain(|p| p.id != self.library.selected);
                    self.library.selected = self.library.playlists[0].id;
                    self.save();
                } else {
                    self.emit(UiEvent::Message("Giữ lại ít nhất một playlist".into()));
                }
            }
            Command::Remove(id) => {
                if let Some(p) = self
                    .library
                    .playlists
                    .iter_mut()
                    .find(|p| p.id == self.library.selected)
                {
                    p.tracks.retain(|t| t.id != id);
                }
                if self.queue.origin == Some(self.library.selected) {
                    self.queue.remove_upcoming(&id);
                }
                self.save();
            }
            Command::Move { index, delta } => {
                if let Some(p) = self
                    .library
                    .playlists
                    .iter_mut()
                    .find(|p| p.id == self.library.selected)
                {
                    let to = index as i64 + delta as i64;
                    if index < p.tracks.len() && to >= 0 && to < p.tracks.len() as i64 {
                        p.tracks.swap(index, to as usize);
                        // Reordering a saved playlist takes effect on its next play.
                        self.save();
                        self.emit(UiEvent::Message(
                            "Đã đổi thứ tự; áp dụng khi phát lại playlist".into(),
                        ));
                    }
                }
            }
            Command::Play(track) => {
                self.queue = Queue {
                    items: vec![track],
                    ..Default::default()
                };
                self.paused = false;
                self.begin();
            }
            Command::PlayPlaylist(index) => {
                if let Some(p) = self.library.selected()
                    && index < p.tracks.len()
                {
                    self.queue = Queue {
                        items: p.tracks.clone(),
                        cursor: index,
                        origin: Some(p.id),
                    };
                    self.paused = false;
                    self.begin();
                }
            }
            Command::Toggle => {
                if self.phase == Phase::Stopped {
                    if self.queue.current().is_none()
                        && let Some(track) = self.library.last_track.clone()
                    {
                        self.queue = Queue {
                            items: vec![track],
                            ..Default::default()
                        };
                    }
                    self.paused = false;
                    self.begin();
                } else {
                    self.set_pause(!self.paused);
                }
            }
            Command::Pause(paused) => {
                if !paused && self.phase == Phase::Stopped {
                    if self.queue.current().is_none() && self.library.last_track.is_none() {
                        if self
                            .library
                            .selected()
                            .is_some_and(|p| !p.tracks.is_empty())
                        {
                            self.control(Command::PlayPlaylist(0));
                        } else {
                            self.emit(UiEvent::Message("Chưa có bài để phát. Tìm một bài hoặc thêm bài vào playlist trước.".into()));
                        }
                    } else {
                        self.control(Command::Toggle);
                    }
                } else {
                    self.set_pause(paused);
                }
            }
            Command::Next => {
                if self.queue.current().is_some() {
                    self.paused = false;
                    self.after_end(false);
                }
            }
            Command::Previous => {
                if self.queue.current().is_some() {
                    if self.position < 3.0 && self.queue.cursor > 0 {
                        self.queue.cursor -= 1;
                    }
                    self.paused = false;
                    self.begin();
                }
            }
            Command::Stop => {
                self.generation += 1;
                youtube::cancel(&self.play_cancel);
                self.player.take();
                self.phase = Phase::Stopped;
                self.position = 0.0;
                self.status = "Đã dừng".into();
            }
            Command::Autoplay(on) => {
                self.library.autoplay = on;
                self.save();
                if !on && self.phase == Phase::Recommending {
                    self.begin();
                }
            }
            Command::Volume(volume) if volume.is_finite() => {
                self.library.volume = volume.clamp(0.0, 100.0);
                if let Some(p) = self.player.as_mut()
                    && let Err(e) = p.volume(self.library.volume)
                {
                    self.emit(UiEvent::Message(e.to_string()));
                }
                self.save();
            }
            Command::Seek(pos) if pos.is_finite() => {
                if let Some(p) = self.player.as_mut() {
                    let pos = pos.clamp(0.0, self.duration.max(0.0));
                    if let Err(e) = p.seek(pos) {
                        self.emit(UiEvent::Message(e.to_string()));
                    }
                }
            }
            Command::SeekRelative(delta) => self.control(Command::Seek(self.position + delta)),
            _ => (),
        }
    }
    fn set_pause(&mut self, paused: bool) {
        self.paused = paused;
        if let Some(p) = self.player.as_mut()
            && let Err(e) = p.pause(paused)
        {
            self.emit(UiEvent::Message(e.to_string()));
        }
        if matches!(self.phase, Phase::Playing | Phase::Paused) {
            self.phase = if paused {
                Phase::Paused
            } else {
                Phase::Playing
            };
            self.playback_status();
        }
    }
    fn playback_status(&mut self) {
        self.status = if self.paused {
            "Đã tạm dừng"
        } else if self.buffering {
            "Đang nạp thêm âm thanh…"
        } else {
            "Đang phát · chỉ âm thanh"
        }
        .into();
    }
    fn begin(&mut self) {
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.generation += 1;
        self.stream_retries = 0;
        self.resume_position = 0.0;
        self.buffering = false;
        let generation = self.generation;
        youtube::cancel(&self.play_cancel);
        self.play_cancel = youtube::cancellation();
        self.player.take();
        self.position = 0.0;
        self.duration = track.duration.unwrap_or(0) as f64;
        self.phase = Phase::Loading;
        self.status = "Đang lấy luồng âm thanh…".into();
        self.library.last_track = Some(track.clone());
        self.save();
        if let Some((id, url, fetched)) = &self.cached
            && id == &track.id
            && fetched.elapsed() < Duration::from_secs(15 * 60)
        {
            let url = url.clone();
            self.open_stream(url);
            return;
        }
        let token = self.play_cancel.clone();
        let yt = self.youtube.clone();
        let tx = self.sender.clone();
        self.jobs.push(thread::spawn(move || {
            let result = yt.resolve(&track, &token).map_err(|e| e.to_string());
            let _ = tx.send(Event::Resolved(generation, result));
        }));
    }
    fn open_stream(&mut self, url: AudioStream) {
        let generation = self.generation;
        let tx = self.sender.clone();
        let result = Player::start(self.library.volume, move |event| {
            let _ = tx.send(Event::Player(generation, event));
        })
        .and_then(|mut p| {
            p.load_stream_at(&url, self.paused, self.resume_position)?;
            Ok(p)
        });
        match result {
            Ok(player) => {
                if let Some(track) = self.queue.current() {
                    self.cached = Some((track.id.clone(), url, Instant::now()));
                }
                self.player = Some(player);
            }
            Err(e) => self.failed(e.to_string()),
        }
    }
    fn failed(&mut self, error: String) {
        self.cached = None;
        self.emit(UiEvent::Message(format!("Không phát được bài: {error}")));
        if self.queue.has_next() {
            self.queue.cursor += 1;
            self.begin();
        } else {
            self.generation += 1;
            self.player.take();
            self.phase = Phase::Stopped;
            self.status = format!("Không phát được: {error}");
        }
    }
    fn retry_connection(&mut self) -> bool {
        if !matches!(self.phase, Phase::Loading | Phase::Playing | Phase::Paused)
            || self.stream_retries >= 2
        {
            return false;
        }
        let Some((_, source, _)) = &self.cached else {
            return false;
        };
        let source = source.clone();
        let refresh = self.phase != Phase::Loading;
        if refresh {
            self.resume_position = self.position;
            self.cached = None;
        }
        self.stream_retries += 1;
        self.generation += 1;
        let generation = self.generation;
        self.player.take();
        self.phase = Phase::Loading;
        self.buffering = false;
        self.status = format!("Đang kết nối lại âm thanh ({}/2)…", self.stream_retries);
        let token = self.play_cancel.clone();
        let tx = self.sender.clone();
        let steps = 10 * self.stream_retries as u64;
        let yt = self.youtube.clone();
        let track = self.queue.current().cloned();
        self.jobs.push(thread::spawn(move || {
            for _ in 0..steps {
                if token.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
            if refresh {
                if let Some(track) = track {
                    let result = yt.resolve(&track, &token).map_err(|e| e.to_string());
                    let _ = tx.send(Event::Resolved(generation, result));
                }
            } else {
                let _ = tx.send(Event::RetryStream(generation, source));
            }
        }));
        true
    }
    fn after_end(&mut self, failed: bool) {
        match self.queue.after_end(self.library.autoplay, failed) {
            AfterEnd::Next => {
                self.queue.cursor += 1;
                self.begin();
            }
            AfterEnd::Replay => self.begin(),
            AfterEnd::Stop => {
                self.generation += 1;
                self.player.take();
                self.phase = Phase::Stopped;
            }
            AfterEnd::Recommend => {
                self.generation += 1;
                let generation = self.generation;
                youtube::cancel(&self.play_cancel);
                self.play_cancel = youtube::cancellation();
                self.player.take();
                self.phase = Phase::Recommending;
                self.status = "Đang tìm bài đề xuất…".into();
                let seed = self.queue.current().unwrap().clone();
                let recent = self.recent.clone();
                let token = self.play_cancel.clone();
                let yt = self.youtube.clone();
                let tx = self.sender.clone();
                self.jobs.push(thread::spawn(move || {
                    let result = yt
                        .related(&seed, &recent, &token)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Event::Related(generation, result));
                }));
            }
        }
    }
    fn player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Loaded => {
                self.phase = if self.paused {
                    Phase::Paused
                } else {
                    Phase::Playing
                };
                self.playback_status();
                if let Some(track) = self.queue.current() {
                    remember(&mut self.recent, &track.id);
                }
                self.snapshot();
            }
            PlayerEvent::Position(pos) => {
                self.position = pos;
                self.emit(UiEvent::Progress(pos, self.duration));
            }
            PlayerEvent::Duration(d) => {
                self.duration = d;
                self.emit(UiEvent::Progress(self.position, d));
            }
            PlayerEvent::Paused(p) => {
                if matches!(self.phase, Phase::Playing | Phase::Paused) && self.paused != p {
                    // Property notifications acknowledge the player; do not send
                    // another set_property back and create a feedback loop.
                    self.paused = p;
                    self.phase = if p { Phase::Paused } else { Phase::Playing };
                    self.playback_status();
                    self.snapshot();
                }
            }
            PlayerEvent::Buffering(buffering) => {
                if self.buffering != buffering {
                    self.buffering = buffering;
                    if matches!(self.phase, Phase::Playing | Phase::Paused) {
                        self.playback_status();
                        self.snapshot();
                    }
                }
            }
            PlayerEvent::End {
                failed: true,
                detail,
            } => {
                if !self.retry_connection() {
                    self.failed(detail);
                }
                self.snapshot();
            }
            PlayerEvent::End { failed: false, .. } => {
                if premature_end(self.position, self.duration) {
                    if !self.retry_connection() {
                        self.failed("Luồng âm thanh kết thúc sớm; không phải hết bài".into());
                    }
                } else {
                    self.after_end(false);
                }
                self.snapshot();
            }
            PlayerEvent::Disconnected => {
                self.generation += 1;
                self.player.take();
                self.phase = Phase::Stopped;
                self.status = "mpv đã dừng bất ngờ. Nhấn phát để thử lại.".into();
                self.snapshot();
            }
        }
    }
}

fn premature_end(position: f64, duration: f64) -> bool {
    duration.is_finite() && position.is_finite() && duration > 5.0 && position + 3.0 < duration
}

#[cfg(test)]
mod tests {
    use super::premature_end;
    #[test]
    fn incomplete_stream_is_not_end_of_song() {
        assert!(premature_end(60.0, 3600.0));
        assert!(premature_end(0.0, 3600.0));
        assert!(!premature_end(3599.0, 3600.0));
        assert!(!premature_end(2.0, 2.5));
        assert!(!premature_end(10.0, 0.0));
    }
}
