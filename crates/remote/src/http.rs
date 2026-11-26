//! Just enough HTTP for one page, one stream of events and one kind of
//! post: a request read from a socket, an answer written to it.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// The most a request's head or body may be. A command is a line of JSON.
const MOST: u64 = 64 * 1024;

/// A request as a phone sent it.
pub struct Request {
    pub method: String,
    /// The path, without its query.
    pub path: String,
    query: String,
    pub body: Vec<u8>,
}

impl Request {
    /// Reads a request, or `None` for anything that isn't one.
    pub fn read(phone: &TcpStream) -> Option<Request> {
        let mut reader = BufReader::new(phone.take(2 * MOST));
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let mut parts = line.split_whitespace();
        let method = parts.next()?.to_string();
        let target = parts.next()?;
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let mut length = 0u64;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).ok()? == 0 {
                return None;
            }
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().ok()?;
            }
        }
        if length > MOST {
            return None;
        }
        let mut body = vec![0; length as usize];
        reader.read_exact(&mut body).ok()?;
        Some(Request {
            method,
            path: path.to_string(),
            query: query.to_string(),
            body,
        })
    }

    /// The value of `name` in the query, as written.
    pub fn query(&self, name: &str) -> Option<&str> {
        self.query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }

    /// [`query`](Request::query) with its `%xx` escapes undone, for a
    /// value that can hold anything, such as a result's id: a path with
    /// spaces, a `#`.
    pub fn query_text(&self, name: &str) -> Option<String> {
        let value = self.query(name)?.as_bytes();
        let mut bytes = Vec::with_capacity(value.len());
        let mut at = 0;
        while at < value.len() {
            let escaped = value
                .get(at + 1..at + 3)
                .filter(|_| value[at] == b'%')
                .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
            match escaped {
                Some(byte) => {
                    bytes.push(byte);
                    at += 3;
                }
                None => {
                    bytes.push(value[at]);
                    at += 1;
                }
            }
        }
        String::from_utf8(bytes).ok()
    }
}

/// Writes a whole answer and ends the connection.
pub fn respond(phone: &mut TcpStream, status: &str, kind: &str, body: &str) -> io::Result<()> {
    respond_bytes(phone, status, kind, body.as_bytes())
}

/// [`respond`], for a body that isn't text.
pub fn respond_bytes(
    phone: &mut TcpStream,
    status: &str,
    kind: &str,
    body: &[u8],
) -> io::Result<()> {
    write(phone, status, kind, "no-store", body)
}

/// A body that never changes while the app runs, such as a font: the
/// phone keeps it.
pub fn respond_kept(phone: &mut TcpStream, kind: &str, body: &[u8]) -> io::Result<()> {
    write(phone, "200 OK", kind, "max-age=604800", body)
}

fn write(
    phone: &mut TcpStream,
    status: &str,
    kind: &str,
    cache: &str,
    body: &[u8],
) -> io::Result<()> {
    phone.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: {cache}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )?;
    phone.write_all(body)
}
