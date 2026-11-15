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
}

/// Writes a whole answer and ends the connection.
pub fn respond(phone: &mut TcpStream, status: &str, kind: &str, body: &str) -> io::Result<()> {
    phone.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
}
