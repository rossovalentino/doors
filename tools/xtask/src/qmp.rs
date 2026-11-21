//! Bounded, synchronous control of the one local diagnostic QEMU process.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

const ORDINARY_TIMEOUT: Duration = Duration::from_secs(3);
const LINE_LIMIT: usize = 64 * 1024;
fn response_budget(command: &str) -> Duration {
    // QEMU encodes the full RAMFB PNG before returning this control response.
    // This host encoding allowance does not extend any guest readiness deadline.
    if command == "screendump" {
        Duration::from_secs(15)
    } else {
        ORDINARY_TIMEOUT
    }
}

pub struct Client {
    reader: BufReader<TcpStream>,
    id: u64,
}
impl Client {
    pub fn connect(port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}")
                .parse()
                .map_err(|e| format!("{e}"))?,
            Duration::from_secs(2),
        )
        .map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        let mut client = Self {
            reader: BufReader::new(stream),
            id: 0,
        };
        if client
            .read(Instant::now() + ORDINARY_TIMEOUT)
            .map_err(|e| format!("QMP greeting port={port} budget_ms=3000: {e}"))?["QMP"]
            .is_null()
        {
            return Err("missing QMP greeting".into());
        }
        client.execute("qmp_capabilities", json!({}))?;
        Ok(client)
    }
    fn read(&mut self, deadline: Instant) -> Result<Value, String> {
        let mut bytes = Vec::new();
        loop {
            // Recompute before each blocking refill, not merely each JSON line:
            // a trickled partial line must not restart the response deadline.
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or("QMP absolute response deadline")?;
            self.reader
                .get_mut()
                .set_read_timeout(Some(remaining))
                .map_err(|e| e.to_string())?;
            let chunk = self.reader.fill_buf().map_err(|e| e.to_string())?;
            if Instant::now() >= deadline {
                return Err("QMP absolute response deadline".into());
            }
            if chunk.is_empty() {
                return Err("QMP closed before a complete line".into());
            }
            let count = chunk
                .iter()
                .position(|b| *b == b'\n')
                .map_or(chunk.len(), |at| at + 1)
                .min(LINE_LIMIT - bytes.len());
            bytes.extend_from_slice(&chunk[..count]);
            self.reader.consume(count);
            if bytes.last() == Some(&b'\n') {
                break;
            }
            if bytes.len() == LINE_LIMIT {
                return Err("QMP exceeded line limit".into());
            }
        }
        let value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if Instant::now() >= deadline {
            return Err("QMP absolute response deadline".into());
        }
        Ok(value)
    }
    pub fn execute(&mut self, command: &str, args: Value) -> Result<Value, String> {
        self.execute_bounded(command, args, response_budget(command))
    }
    fn execute_bounded(
        &mut self,
        command: &str,
        args: Value,
        budget: Duration,
    ) -> Result<Value, String> {
        self.id = self.id.checked_add(1).ok_or("QMP command ID exhausted")?;
        let started = Instant::now();
        let deadline = started + budget;
        let result = (|| {
            let record = json!({"execute":command,"arguments":args,"id":self.id});
            let mut bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
            bytes.push(b'\n');
            self.reader
                .get_mut()
                .write_all(&bytes)
                .map_err(|e| e.to_string())?;
            for _ in 0..32 {
                let response = self.read(deadline)?;
                if response["id"] == self.id {
                    if !response["error"].is_null() {
                        return Err(format!("QMP error {}", response["error"]));
                    }
                    return Ok(response["return"].clone());
                }
            }
            Err("QMP response event limit".into())
        })();
        let restore = self
            .reader
            .get_mut()
            .set_read_timeout(Some(ORDINARY_TIMEOUT))
            .map_err(|e| format!("restore ordinary timeout: {e}"));
        result
            .and_then(|value| restore.map(|()| value))
            .map_err(|e| {
                format!(
                    "QMP command={command} id={} response elapsed_ms={} budget_ms={}: {e}",
                    self.id,
                    started.elapsed().as_millis(),
                    budget.as_millis()
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};
    fn pair() -> (Client, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        stream.set_write_timeout(Some(ORDINARY_TIMEOUT)).unwrap();
        let (peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(ORDINARY_TIMEOUT)).unwrap();
        (
            Client {
                reader: BufReader::new(stream),
                id: 0,
            },
            peer,
        )
    }
    fn request(peer: &TcpStream) -> Value {
        let mut line = String::new();
        BufReader::new(peer).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn reply(peer: &mut TcpStream, value: Value) {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        peer.write_all(&bytes).unwrap();
    }
    #[test]
    fn only_screendump_receives_encoding_time_and_ordinary_timeout_is_restored() {
        assert_eq!(response_budget("screendump"), Duration::from_secs(15));
        for name in [
            "query-status",
            "input-send-event",
            "Screendump",
            "screendump ",
        ] {
            assert_eq!(response_budget(name), ORDINARY_TIMEOUT);
        }
        let (mut client, mut peer) = pair();
        let worker = thread::spawn(move || {
            let first = request(&peer);
            assert_eq!(first["execute"], "screendump");
            thread::sleep(Duration::from_millis(3200));
            reply(&mut peer, json!({"return":{},"id":first["id"]}));
            let second = request(&peer);
            assert_eq!(second["execute"], "query-status");
            reply(
                &mut peer,
                json!({"return":{"running":true},"id":second["id"]}),
            );
        });
        client.execute("screendump", json!({})).unwrap();
        assert_eq!(
            client.reader.get_ref().read_timeout().unwrap(),
            Some(ORDINARY_TIMEOUT)
        );
        assert_eq!(
            client.execute("query-status", json!({})).unwrap()["running"],
            true
        );
        worker.join().unwrap();
    }
    #[test]
    fn absolute_deadline_covers_trickled_partial_lines_and_multiple_events() {
        for partial in [true, false] {
            let (mut client, mut peer) = pair();
            let worker = thread::spawn(move || {
                let _ = request(&peer);
                for _ in 0..100 {
                    let payload: &[u8] = if partial {
                        b" "
                    } else {
                        b"{\"event\":\"CLOCK\"}\n"
                    };
                    if peer.write_all(payload).is_err() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(30));
                }
            });
            let start = Instant::now();
            let error = client
                .execute_bounded("deadline-test", json!({}), Duration::from_millis(80))
                .unwrap_err();
            assert!(
                error.contains("command=deadline-test id=1 response")
                    && error.contains("budget_ms=80")
            );
            assert!(start.elapsed() < Duration::from_secs(2));
            assert_eq!(
                client.reader.get_ref().read_timeout().unwrap(),
                Some(ORDINARY_TIMEOUT)
            );
            drop(client);
            worker.join().unwrap();
        }
    }
    #[test]
    fn line_and_event_limits_still_fail_and_restore_timeout_without_replay() {
        for oversized in [true, false] {
            let (mut client, mut peer) = pair();
            let worker = thread::spawn(move || {
                let command = request(&peer);
                assert_eq!(command["id"], 1);
                if oversized {
                    let _ = peer.write_all(&vec![b' '; LINE_LIMIT]);
                } else {
                    for _ in 0..32 {
                        reply(&mut peer, json!({"event":"CLOCK"}));
                    }
                }
            });
            let error = client.execute("query-status", json!({})).unwrap_err();
            assert!(error.contains(if oversized {
                "line limit"
            } else {
                "event limit"
            }));
            assert_eq!(client.id, 1);
            assert_eq!(
                client.reader.get_ref().read_timeout().unwrap(),
                Some(ORDINARY_TIMEOUT)
            );
            drop(client);
            worker.join().unwrap();
        }
    }
}
