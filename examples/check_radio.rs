//! Live, best-effort recommendation check. Does not play any audio.
use anyhow::{Context, Result};
use may_music::youtube::{Youtube, cancellation};
use std::collections::VecDeque;

fn main() -> Result<()> {
    let query = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let query = if query.is_empty() {
        "Kevin MacLeod Carefree"
    } else {
        &query
    };
    let yt = Youtube::default();
    let cancel = cancellation();
    let tracks = yt.search(query, &cancel)?;
    let seed = tracks.first().context("No search results")?;
    let recent = VecDeque::from([seed.id.clone()]);
    let next = yt.related(seed, &recent, &cancel)?;
    assert_ne!(next.id, seed.id);
    println!("Seed: {} — {}", seed.title, seed.channel);
    println!("Next: {} — {}", next.title, next.channel);
    println!("Live recommendation check passed.");
    Ok(())
}
