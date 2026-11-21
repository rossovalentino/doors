//! door-webdav: NoneDrive's protocol and sync logic, sans-IO.
//!
//! NoneDrive keeps `%USERPROFILE%\NoneDrive` in step with a folder on a
//! server the user owns: a NAS, Nextcloud, ownCloud, Synology or any other
//! WebDAV server (RFC 4918), over HTTPS only.
//!
//! - this module: the server address and the request heads (PROPFIND,
//!   GET, PUT, MKCOL, DELETE, MOVE) with Basic authentication;
//! - `href`: percent-encoding of relative paths and decoding of the hrefs
//!   a server returns;
//! - `xml`: the `207 Multi-Status` answer to PROPFIND;
//! - `index`: what both sides held at the last successful sync;
//! - `sync`: the three-way planner (local listing, remote listing, index)
//!   that decides every upload, download, deletion and conflict copy.
//!
//! The session's sync engine drives these over door.network and door-tls;
//! everything here is host-tested.
#![no_std]
extern crate alloc;

pub mod account;
pub mod href;
pub mod index;
pub mod status;
pub mod sync;
pub mod xml;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
pub use door_http::Headers;

/// The properties NoneDrive asks for in every listing.
pub const PROPFIND_BODY: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
    "<d:propfind xmlns:d=\"DAV:\"><d:prop>",
    "<d:resourcetype/><d:getetag/><d:getcontentlength/><d:getlastmodified/>",
    "</d:prop></d:propfind>"
);
/// The client's name in requests.
pub const USER_AGENT: &str = "NoneDrive/1.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not an `https://` address with a host.
    Address,
    /// User names may not contain `:` (Basic authentication).
    UserName,
    /// A path that is not a valid relative path.
    Path,
    /// A malformed server answer.
    Answer,
}

/// The sync folder on the server: `https://host[:port]/base/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
    /// The folder's path, percent-encoded, starting and ending with `/`.
    pub base: String,
    authorization: String,
}
impl Server {
    /// `address` as the user typed it (`https://nas.home/remote.php/dav/
    /// files/me/` or `https://192.168.1.10:5006/NoneDrive`), with the user
    /// name and app password for Basic authentication (over TLS only).
    pub fn new(address: &str, user: &str, password: &str) -> Result<Self, Error> {
        let url = door_http::parse_url(address).map_err(|_| Error::Address)?;
        if url.scheme != door_http::Scheme::Https || url.host.is_empty() {
            return Err(Error::Address);
        }
        if user.contains(':') {
            return Err(Error::UserName);
        }
        // Query strings and fragments have no place in a folder address.
        let path = url.path.split('?').next().unwrap_or("/");
        let mut base = String::from(path);
        if !base.ends_with('/') {
            base.push('/');
        }
        Ok(Self {
            host: url.host,
            port: url.port,
            base,
            authorization: format!(
                "Basic {}",
                door_http::base64(format!("{user}:{password}").as_bytes())
            ),
        })
    }
    /// The Host header's value.
    pub fn host_header(&self) -> String {
        if self.port == 443 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
    /// The request target of `rel` (`/`-separated, no leading slash; empty
    /// for the sync folder itself); folders end with `/`.
    pub fn target(&self, rel: &str, folder: bool) -> Result<String, Error> {
        let mut t = self.base.clone();
        if !rel.is_empty() {
            t.push_str(&href::encode(rel)?);
            if folder {
                t.push('/');
            }
        }
        Ok(t)
    }
    /// The absolute URL of `rel` (MOVE's Destination).
    pub fn url(&self, rel: &str, folder: bool) -> Result<String, Error> {
        Ok(format!(
            "https://{}{}",
            self.host_header(),
            self.target(rel, folder)?
        ))
    }
    fn head(
        &self,
        method: &str,
        target: &str,
        extra: &[(&str, &str)],
        body: Option<u64>,
    ) -> Vec<u8> {
        let mut h = Headers::default();
        h.add("Host", &self.host_header());
        h.add("Authorization", &self.authorization);
        h.add("User-Agent", USER_AGENT);
        for (name, value) in extra {
            h.add(name, value);
        }
        if let Some(n) = body {
            h.add("Content-Length", &n.to_string());
        }
        door_http::request_head(method, target, "HTTP/1.1", &self.host_header(), &h)
    }
    /// List folder `rel` and its children (Depth 1): the whole request.
    pub fn propfind(&self, rel: &str) -> Result<Vec<u8>, Error> {
        self.propfind_depth(rel, true, "1")
    }
    /// One item's properties (Depth 0): a file's new entity tag after an
    /// upload whose answer carried none.
    pub fn propfind_one(&self, rel: &str, folder: bool) -> Result<Vec<u8>, Error> {
        self.propfind_depth(rel, folder, "0")
    }
    fn propfind_depth(&self, rel: &str, folder: bool, depth: &str) -> Result<Vec<u8>, Error> {
        let target = self.target(rel, folder)?;
        let mut r = self.head(
            "PROPFIND",
            &target,
            &[
                ("Depth", depth),
                ("Content-Type", "application/xml; charset=utf-8"),
            ],
            Some(PROPFIND_BODY.len() as u64),
        );
        r.extend_from_slice(PROPFIND_BODY.as_bytes());
        Ok(r)
    }
    pub fn get(&self, rel: &str) -> Result<Vec<u8>, Error> {
        Ok(self.head("GET", &self.target(rel, false)?, &[], None))
    }
    /// The head of an upload of `length` bytes (the body follows), under a
    /// precondition so a concurrent change on the server is never
    /// overwritten (412 instead).
    pub fn put(&self, rel: &str, length: u64, when: Precondition<'_>) -> Result<Vec<u8>, Error> {
        let target = self.target(rel, false)?;
        let content = ("Content-Type", "application/octet-stream");
        Ok(match when {
            Precondition::Matches(tag) => {
                self.head("PUT", &target, &[content, ("If-Match", tag)], Some(length))
            }
            Precondition::Absent => self.head(
                "PUT",
                &target,
                &[content, ("If-None-Match", "*")],
                Some(length),
            ),
        })
    }
    pub fn mkcol(&self, rel: &str) -> Result<Vec<u8>, Error> {
        Ok(self.head("MKCOL", &self.target(rel, true)?, &[], Some(0)))
    }
    pub fn delete(&self, rel: &str, folder: bool) -> Result<Vec<u8>, Error> {
        if rel.is_empty() {
            // Never the sync folder itself.
            return Err(Error::Path);
        }
        Ok(self.head("DELETE", &self.target(rel, folder)?, &[], None))
    }
    /// Rename `from` to `to` on the server, never replacing what is there.
    pub fn move_to(&self, from: &str, to: &str, folder: bool) -> Result<Vec<u8>, Error> {
        if from.is_empty() || to.is_empty() {
            return Err(Error::Path);
        }
        let destination = self.url(to, folder)?;
        Ok(self.head(
            "MOVE",
            &self.target(from, folder)?,
            &[("Destination", &destination), ("Overwrite", "F")],
            None,
        ))
    }
}

/// The server-side state an upload requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precondition<'a> {
    /// The file still has this entity tag (an update).
    Matches(&'a str),
    /// Nothing is there yet (a new file).
    Absent,
}

/// What a status code means to the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// 401: the user name or app password is wrong (Settings asks again).
    SignIn,
    /// 403.
    Denied,
    /// 404 / 410.
    Missing,
    /// 409 (a parent folder is missing) or 412 (changed since listed):
    /// list again before retrying.
    Changed,
    /// 423: locked by another client; retry later.
    Locked,
    /// 507: the server is full.
    Full,
    /// Anything else (5xx, unexpected codes): retry later.
    Retry,
}
pub fn outcome(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Done,
        401 => Outcome::SignIn,
        403 => Outcome::Denied,
        404 | 410 => Outcome::Missing,
        409 | 412 => Outcome::Changed,
        423 => Outcome::Locked,
        507 => Outcome::Full,
        _ => Outcome::Retry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(bytes: &[u8]) -> &str {
        core::str::from_utf8(bytes).unwrap()
    }
    #[test]
    fn addresses_are_https_folders() {
        let s = Server::new("https://nas.home/remote.php/dav/files/me", "me", "pw").unwrap();
        assert_eq!(s.base, "/remote.php/dav/files/me/");
        assert_eq!(s.host_header(), "nas.home");
        let s = Server::new("https://10.0.0.5:5006/", "me", "pw").unwrap();
        assert_eq!((s.port, s.host_header().as_str()), (5006, "10.0.0.5:5006"));
        assert_eq!(
            Server::new("http://nas.home/", "me", "pw"),
            Err(Error::Address)
        );
        assert_eq!(
            Server::new("https://nas.home/", "a:b", "pw"),
            Err(Error::UserName)
        );
    }
    #[test]
    fn requests_carry_basic_authentication_and_encoded_targets() {
        let s = Server::new("https://nas.home/dav/", "me", "secret").unwrap();
        let r = s.propfind("Photos 2026").unwrap();
        let t = text(&r);
        assert!(t.starts_with("PROPFIND /dav/Photos%202026/ HTTP/1.1\r\n"));
        assert!(t.contains("Host: nas.home\r\n"));
        // base64("me:secret")
        assert!(t.contains("Authorization: Basic bWU6c2VjcmV0\r\n"));
        assert!(t.contains("Depth: 1\r\n"));
        assert!(t.ends_with(PROPFIND_BODY));
        assert!(t.contains(&format!("Content-Length: {}\r\n", PROPFIND_BODY.len())));
        let root = s.propfind("").unwrap();
        assert!(text(&root).starts_with("PROPFIND /dav/ HTTP/1.1\r\n"));
        let put = s.put("a/b.txt", 12, Precondition::Matches("\"x1\"")).unwrap();
        let t = text(&put);
        assert!(t.starts_with("PUT /dav/a/b.txt HTTP/1.1\r\n"));
        assert!(t.contains("If-Match: \"x1\"\r\n") && t.contains("Content-Length: 12\r\n"));
        let mv = s.move_to("a.txt", "b c.txt", false).unwrap();
        let t = text(&mv);
        assert!(t.starts_with("MOVE /dav/a.txt HTTP/1.1\r\n"));
        assert!(t.contains("Destination: https://nas.home/dav/b%20c.txt\r\n"));
        assert!(t.contains("Overwrite: F\r\n"));
        assert_eq!(s.delete("", true), Err(Error::Path));
        assert!(text(&s.mkcol("New").unwrap()).starts_with("MKCOL /dav/New/ HTTP/1.1\r\n"));
    }
    #[test]
    fn statuses_map_to_engine_outcomes() {
        assert_eq!(outcome(207), Outcome::Done);
        assert_eq!(outcome(201), Outcome::Done);
        assert_eq!(outcome(401), Outcome::SignIn);
        assert_eq!(outcome(412), Outcome::Changed);
        assert_eq!(outcome(507), Outcome::Full);
        assert_eq!(outcome(502), Outcome::Retry);
    }
}
