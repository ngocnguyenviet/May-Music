//! A bounded, seekable HTTP bridge for YouTube's direct audio streams.
//! googlevideo throttles large open-ended responses; fetch at most 1 MiB per
//! upstream request while exposing one ordinary HTTP range resource to mpv.
use crate::youtube::AudioStream;
use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CHUNK: u64 = 1024 * 1024;
const MAX_CLIENTS: usize = 3;
const HEADER_LIMIT: usize = 8192;

pub fn supports_chunks(source: &AudioStream) -> bool {
    if source.content_length.is_none_or(|n| n == 0) {
        return false;
    }
    let Ok(uri) = gtk::glib::Uri::parse(&source.url, gtk::glib::UriFlags::ENCODED) else {
        return false;
    };
    uri.scheme() == "https"
        && uri
            .host()
            .is_some_and(|host| host == "googlevideo.com" || host.ends_with(".googlevideo.com"))
        && uri.path() == "/videoplayback"
}

pub struct Relay {
    address: SocketAddr,
    path: String,
    shared: Arc<Shared>,
    listener: Option<thread::JoinHandle<()>>,
}

struct Shared {
    source: AudioStream,
    agent: ureq::Agent,
    stop: AtomicBool,
    clients: Mutex<Vec<(u64, TcpStream)>>,
    active: AtomicUsize,
    path: String,
}

impl Relay {
    pub fn start(source: AudioStream) -> Result<Self> {
        if source.content_length.is_none_or(|n| n == 0) {
            bail!("Không biết kích thước luồng âm thanh");
        }
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = format!("/audio-{}-{nonce:x}", std::process::id());
        let agent = ureq::Agent::config_builder()
            .https_only(source.url.starts_with("https://"))
            .ip_family(source_family(&source.url))
            .timeout_global(Some(Duration::from_secs(20)))
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_per_read(Some(Duration::from_secs(8)))
            .max_idle_connections(3)
            .max_redirects(3)
            .build()
            .new_agent();
        let shared = Arc::new(Shared {
            source,
            agent,
            stop: AtomicBool::new(false),
            clients: Mutex::new(vec![]),
            active: AtomicUsize::new(0),
            path: path.clone(),
        });
        let target = shared.clone();
        let worker = thread::spawn(move || {
            let mut jobs = Vec::new();
            let mut next_id = 0;
            while let Ok((mut client, _)) = listener.accept() {
                if target.stop.load(Ordering::Relaxed) {
                    break;
                }
                if target.active.load(Ordering::Relaxed) >= MAX_CLIENTS {
                    let _ = client.set_write_timeout(Some(Duration::from_millis(100)));
                    let _ = client.write_all(
                        b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                }
                next_id += 1;
                let id = next_id;
                if let Ok(copy) = client.try_clone() {
                    target.clients.lock().unwrap().push((id, copy));
                }
                target.active.fetch_add(1, Ordering::Relaxed);
                let context = target.clone();
                jobs.retain(|j: &thread::JoinHandle<()>| !j.is_finished());
                jobs.push(thread::spawn(move || {
                    let _ = serve(&mut client, &context);
                    let _ = client.shutdown(Shutdown::Both);
                    context
                        .clients
                        .lock()
                        .unwrap()
                        .retain(|(key, _)| *key != id);
                    context.active.fetch_sub(1, Ordering::Relaxed);
                }));
            }
            for job in jobs {
                let _ = job.join();
            }
        });
        Ok(Self {
            address,
            path,
            shared,
            listener: Some(worker),
        })
    }
    pub fn url(&self) -> String {
        format!("http://{}{}", self.address, self.path)
    }
}

fn source_family(url: &str) -> ureq::config::IpFamily {
    // Signed YouTube URLs can be tied to the IPv4/IPv6 address used by yt-dlp.
    let address = url
        .split_once('?')
        .and_then(|(_, query)| query.split('&').find_map(|part| part.strip_prefix("ip=")))
        .and_then(|value| gtk::glib::Uri::unescape_string(value, None))
        .and_then(|value| value.parse::<std::net::IpAddr>().ok());
    match address {
        Some(std::net::IpAddr::V4(_)) => ureq::config::IpFamily::Ipv4Only,
        Some(std::net::IpAddr::V6(_)) => ureq::config::IpFamily::Ipv6Only,
        None => ureq::config::IpFamily::Any,
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        // ureq 3.5 shutdown interrupts active network I/O as well as its pool.
        self.shared.agent.clone().shutdown();
        for (_, client) in self.shared.clients.lock().unwrap().iter() {
            let _ = client.shutdown(Shutdown::Both);
        }
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(200));
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

fn range(value: Option<&str>, size: u64) -> Result<(u64, u64)> {
    if size == 0 {
        bail!("Empty resource");
    }
    let Some(value) = value else {
        return Ok((0, size - 1));
    };
    let value = value.strip_prefix("bytes=").context("Bad range")?;
    let (left, right) = value.split_once('-').context("Bad range")?;
    if left.is_empty() {
        let suffix: u64 = right.parse()?;
        if suffix == 0 {
            bail!("Empty suffix");
        }
        return Ok((size.saturating_sub(suffix), size - 1));
    }
    let start: u64 = left.parse()?;
    let end = if right.is_empty() {
        size - 1
    } else {
        right.parse::<u64>()?.min(size - 1)
    };
    if start >= size || end < start {
        bail!("Unsatisfiable range");
    }
    Ok((start, end))
}

fn chunk_url(url: &str, start: u64, end: u64) -> String {
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let query = query
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with("range="))
        .collect::<Vec<_>>()
        .join("&");
    format!(
        "{base}?{}range={start}-{end}",
        if query.is_empty() {
            String::new()
        } else {
            format!("{query}&")
        }
    )
}

fn fetch(shared: &Shared, start: u64, end: u64) -> Result<Vec<u8>> {
    let mut last = "Không tải được đoạn âm thanh".to_string();
    for attempt in 0..3 {
        if shared.stop.load(Ordering::Relaxed) {
            bail!("Đã hủy");
        }
        if attempt > 0 {
            for _ in 0..attempt * 5 {
                if shared.stop.load(Ordering::Relaxed) {
                    bail!("Đã hủy");
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
        let result = (|| -> Result<Vec<u8>> {
            let url = chunk_url(&shared.source.url, start, end);
            let mut request = shared.agent.get(&url).header("Accept-Encoding", "identity");
            for (key, value) in &shared.source.headers {
                if !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "range" | "host" | "content-length" | "accept-encoding" | "connection"
                ) {
                    request = request.header(key, value);
                }
            }
            let mut response = request
                .call()
                .map_err(|e| anyhow::anyhow!("Upstream: {e}"))?;
            if !matches!(response.status().as_u16(), 200 | 206) {
                bail!("Phản hồi âm thanh không hợp lệ");
            }
            let expected = end - start + 1;
            let mut data = Vec::with_capacity(expected as usize);
            response
                .body_mut()
                .as_reader()
                .take(expected + 1)
                .read_to_end(&mut data)?;
            if data.len() as u64 != expected {
                bail!("Đoạn âm thanh chưa tải đủ hoặc máy chủ bỏ qua giới hạn byte");
            }
            Ok(data)
        })();
        match result {
            Ok(data) => return Ok(data),
            Err(e) => last = e.to_string(),
        }
    }
    bail!("{last}")
}

fn serve(client: &mut TcpStream, shared: &Shared) -> Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(3)))?;
    client.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(client.try_clone()?);
    let mut lines = Vec::new();
    let mut used = 0;
    loop {
        let mut line = vec![];
        let read = reader
            .by_ref()
            .take((HEADER_LIMIT - used) as u64)
            .read_until(b'\n', &mut line)?;
        used += read;
        if read == 0 || used >= HEADER_LIMIT {
            bail!("Bad request headers");
        }
        if line == b"\r\n" || line == b"\n" {
            break;
        }
        lines.push(String::from_utf8(line)?.trim_end().to_string());
    }
    let request = lines
        .first()
        .context("Missing request")?
        .split_whitespace()
        .collect::<Vec<_>>();
    if request.len() != 3 || !matches!(request[0], "GET" | "HEAD") || request[1] != shared.path {
        client.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    }
    let range_header = lines
        .iter()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("range"))
        .map(|(_, value)| value.trim());
    let size = shared.source.content_length.unwrap();
    let (mut start, end) = match range(range_header, size) {
        Ok(range) => range,
        Err(_) => {
            write!(
                client,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            return Ok(());
        }
    };
    let code = if range_header.is_some() {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    // Fetch before sending a success header; an upstream failure is not EOF.
    let mut first = if request[0] == "HEAD" {
        None
    } else {
        match fetch(shared, start, end.min(start + CHUNK - 1)) {
            Ok(data) => Some(data),
            Err(_) => {
                client.write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )?;
                return Ok(());
            }
        }
    };
    write!(
        client,
        "HTTP/1.1 {code}\r\nAccept-Ranges: bytes\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        end - start + 1
    )?;
    if range_header.is_some() {
        write!(client, "Content-Range: bytes {start}-{end}/{size}\r\n")?;
    }
    client.write_all(b"\r\n")?;
    if request[0] == "HEAD" {
        return Ok(());
    }
    while start <= end && !shared.stop.load(Ordering::Relaxed) {
        let chunk_end = end.min(start + CHUNK - 1);
        let data = match first.take() {
            Some(data) => data,
            None => fetch(shared, start, chunk_end)?,
        };
        write_audio(client, &data, &shared.stop)?;
        start = chunk_end + 1;
    }
    Ok(())
}

fn write_audio(client: &mut TcpStream, data: &[u8], stop: &AtomicBool) -> Result<()> {
    // A full mpv cache (or user pause) is normal backpressure. A timed-out
    // partial write must keep its exact byte offset, never repeat/drop bytes.
    let mut written = 0;
    while written < data.len() && !stop.load(Ordering::Relaxed) {
        match client.write(&data[written..]) {
            Ok(0) => bail!("Audio client closed"),
            Ok(count) => written += count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::mpsc, time::Instant};

    fn mock_source(url: String, size: u64) -> AudioStream {
        AudioStream {
            url,
            headers: BTreeMap::new(),
            content_length: Some(size),
        }
    }
    #[test]
    fn ranges_are_seekable_and_bounded() {
        assert_eq!(range(None, 100).unwrap(), (0, 99));
        assert_eq!(range(Some("bytes=20-"), 100).unwrap(), (20, 99));
        assert_eq!(range(Some("bytes=20-999"), 100).unwrap(), (20, 99));
        assert_eq!(range(Some("bytes=-12"), 100).unwrap(), (88, 99));
        for bad in [
            "bytes=100-",
            "bytes=30-20",
            "bytes=-0",
            "bytes=0-1,3-4",
            "other=0-",
        ] {
            assert!(range(Some(bad), 100).is_err());
        }
        assert_eq!(
            chunk_url("https://host/audio?sig=abc&range=1-5", 10, 20),
            "https://host/audio?sig=abc&range=10-20"
        );
    }
    #[test]
    fn slow_audio_client_does_not_drop_or_duplicate_partial_writes() {
        use std::os::fd::AsRawFd;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let expected = (0..1024 * 1024)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>();
        let data = expected.clone();
        let sender = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let small: libc::c_int = 4096;
            let result = unsafe {
                libc::setsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&small as *const libc::c_int).cast(),
                    std::mem::size_of_val(&small) as libc::socklen_t,
                )
            };
            assert_eq!(result, 0);
            stream
                .set_write_timeout(Some(Duration::from_millis(20)))
                .unwrap();
            write_audio(&mut stream, &data, &AtomicBool::new(false)).unwrap();
        });
        let mut receiver = TcpStream::connect(address).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        thread::sleep(Duration::from_millis(150));
        let mut actual = vec![];
        receiver.read_to_end(&mut actual).unwrap();
        sender.join().unwrap();
        assert_eq!(actual, expected);
    }
    #[test]
    fn retries_partial_chunks_preserves_bytes_and_supports_seeking() {
        let size = CHUNK * 2 + 137;
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        upstream.set_nonblocking(true).unwrap();
        let address = upstream.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let ranges = Arc::new(Mutex::new(vec![]));
        let seen = ranges.clone();
        let worker = thread::spawn(move || {
            let mut fail_once = true;
            while !shutdown.load(Ordering::Relaxed) {
                let (mut stream, _) = match upstream.accept() {
                    Ok(s) => s,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut request).unwrap();
                let bounds = request
                    .split("range=")
                    .nth(1)
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap();
                let (start, end) = bounds.split_once('-').unwrap();
                let start: u64 = start.parse().unwrap();
                let end: u64 = end.parse().unwrap();
                seen.lock().unwrap().push((start, end));
                assert!(end - start < CHUNK);
                let mut line = String::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                let length = end - start + 1;
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                let count = if fail_once { length / 2 } else { length };
                fail_once = false;
                let bytes = (start..start + count)
                    .map(|i| (i % 251) as u8)
                    .collect::<Vec<_>>();
                let _ = stream.write_all(&bytes);
            }
        });
        let relay = Relay::start(mock_source(
            format!("http://{address}/audio?sig=test"),
            size,
        ))
        .unwrap();
        let client = ureq::Agent::config_builder()
            .proxy(None)
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .new_agent();
        let mut response = client.get(relay.url()).call().unwrap();
        assert_eq!(
            response.headers().get("Content-Length").unwrap(),
            size.to_string().as_str()
        );
        let mut bytes = vec![];
        response
            .body_mut()
            .as_reader()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes.len() as u64, size);
        assert!(
            bytes
                .iter()
                .enumerate()
                .all(|(i, b)| *b == (i as u64 % 251) as u8)
        );
        let mut response = client
            .get(relay.url())
            .header("Range", "bytes=1234567-1234999")
            .call()
            .unwrap();
        assert_eq!(response.status().as_u16(), 206);
        let mut bytes = vec![];
        response
            .body_mut()
            .as_reader()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(
            bytes,
            (1234567..=1234999)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>()
        );
        let count = ranges.lock().unwrap().len();
        let response = client.head(relay.url()).call().unwrap();
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(count, ranges.lock().unwrap().len());
        assert!(
            count >= 5,
            "must retry an interrupted chunk and fetch several bounded ranges"
        );
        let address = relay.address;
        drop(relay);
        assert!(TcpStream::connect(address).is_err());
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
    }

    #[test]
    fn shutdown_interrupts_an_upstream_read_instead_of_waiting_for_timeout() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = upstream.local_addr().unwrap();
        let (started, ready) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut connection, _) = upstream.accept().unwrap();
            connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(connection.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            started.send(()).unwrap();
            let mut data = [0];
            let _ = connection.read(&mut data);
        });
        let relay = Relay::start(mock_source(format!("http://{address}/slow"), 100)).unwrap();
        let url = relay.url();
        let request = thread::spawn(move || {
            let _ = ureq::get(&url).call();
        });
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        let started = Instant::now();
        drop(relay);
        assert!(started.elapsed() < Duration::from_secs(2));
        request.join().unwrap();
        server.join().unwrap();
    }
}
