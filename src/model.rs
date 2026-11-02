use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: Option<u64>,
}

impl Track {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Playlist {
    pub id: u64,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub playlists: Vec<Playlist>,
    pub selected: u64,
    pub next_id: u64,
    pub autoplay: bool,
    pub volume: f64,
    pub last_track: Option<Track>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            playlists: vec![Playlist {
                id: 1,
                name: "Nhạc của tớ".into(),
                tracks: vec![],
            }],
            selected: 1,
            next_id: 2,
            autoplay: false,
            volume: 70.0,
            last_track: None,
        }
    }
}

impl Library {
    pub fn selected(&self) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == self.selected)
    }

    pub fn add(&mut self, track: Track) -> bool {
        if let Some(p) = self.playlists.iter_mut().find(|p| p.id == self.selected)
            && !p.tracks.iter().any(|t| t.id == track.id)
        {
            p.tracks.push(track);
            return true;
        }
        false
    }

    pub fn create(&mut self, name: &str) -> bool {
        let name: String = name.trim().chars().take(80).collect();
        if name.is_empty() {
            return false;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.playlists.push(Playlist {
            id,
            name,
            tracks: vec![],
        });
        self.selected = id;
        true
    }
}

#[derive(Clone, Debug, Default)]
pub struct Queue {
    pub items: Vec<Track>,
    pub cursor: usize,
    pub origin: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub enum AfterEnd {
    Next,
    Recommend,
    Replay,
    Stop,
}

impl Queue {
    pub fn current(&self) -> Option<&Track> {
        self.items.get(self.cursor)
    }
    pub fn has_next(&self) -> bool {
        self.cursor + 1 < self.items.len()
    }
    pub fn after_end(&self, autoplay: bool, failed: bool) -> AfterEnd {
        if self.current().is_none() {
            AfterEnd::Stop
        } else if self.has_next() {
            AfterEnd::Next
        } else if failed {
            AfterEnd::Stop
        } else if autoplay {
            AfterEnd::Recommend
        } else {
            AfterEnd::Replay
        }
    }
    pub fn remove_upcoming(&mut self, id: &str) {
        // Keep the sounding track, but remove deleted tracks from the rest of the queue.
        let old = self.cursor;
        let removed_before = self.items[..old.min(self.items.len())]
            .iter()
            .filter(|t| t.id == id)
            .count();
        let mut index = 0;
        self.items.retain(|t| {
            let keep = index == old || t.id != id;
            index += 1;
            keep
        });
        self.cursor = old.saturating_sub(removed_before);
    }
}

pub fn remember(recent: &mut VecDeque<String>, id: &str) {
    recent.retain(|x| x != id);
    recent.push_back(id.into());
    while recent.len() > 50 {
        recent.pop_front();
    }
}

pub fn choose_recommendation(
    candidates: Vec<Track>,
    current: &str,
    recent: &VecDeque<String>,
) -> Option<Track> {
    candidates
        .into_iter()
        .find(|t| t.id != current && !recent.contains(&t.id))
}

pub fn clock(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0) as u64
    } else {
        0
    };
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
pub fn track(id: &str) -> Track {
    Track {
        id: id.into(),
        title: format!("Track {id}"),
        channel: "Test".into(),
        duration: Some(120),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_precedes_radio_and_replay() {
        let mut q = Queue {
            items: vec![track("a"), track("b")],
            cursor: 0,
            origin: Some(1),
        };
        assert_eq!(q.after_end(true, false), AfterEnd::Next);
        q.cursor = 1;
        assert_eq!(q.after_end(false, false), AfterEnd::Replay);
        assert_eq!(q.after_end(true, false), AfterEnd::Recommend);
        assert_eq!(q.after_end(true, true), AfterEnd::Stop);
        assert_eq!(Queue::default().after_end(false, false), AfterEnd::Stop);
    }
    #[test]
    fn failed_track_advances_but_never_repeats_forever() {
        let q = Queue {
            items: vec![track("a"), track("b")],
            ..Default::default()
        };
        assert_eq!(q.after_end(true, true), AfterEnd::Next);
    }
    #[test]
    fn deletion_preserves_sounding_track_and_cursor() {
        let mut q = Queue {
            items: vec![track("a"), track("b"), track("c")],
            cursor: 1,
            origin: Some(1),
        };
        q.remove_upcoming("a");
        assert_eq!(q.current().unwrap().id, "b");
        q.remove_upcoming("b");
        assert_eq!(q.current().unwrap().id, "b");
        q.remove_upcoming("c");
        assert!(!q.has_next());
    }
    #[test]
    fn playlists_are_unique_and_named() {
        let mut l = Library::default();
        assert!(l.add(track("a")));
        assert!(!l.add(track("a")));
        assert!(!l.create("  "));
        assert!(l.create("  Tập trung  "));
        assert_eq!(l.selected().unwrap().name, "Tập trung");
    }
    #[test]
    fn radio_excludes_recent_and_current() {
        let mut recent = VecDeque::new();
        remember(&mut recent, "b");
        let result = choose_recommendation(vec![track("a"), track("b"), track("c")], "a", &recent);
        assert_eq!(result.unwrap().id, "c");
        for i in 0..100 {
            remember(&mut recent, &i.to_string());
        }
        assert_eq!(recent.len(), 50);
    }
}
