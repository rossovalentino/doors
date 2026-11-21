//! The `207 Multi-Status` body of a PROPFIND (RFC 4918 section 14): one
//! `response` per resource with its `href` and `propstat` groups. Only the
//! elements NoneDrive reads are interpreted, by local name in the `DAV:`
//! namespace whatever prefix the server picked (`d:`, `D:`, `lp1:` or a
//! default namespace); everything else is skipped.
use crate::{Error, href};
use alloc::{string::String, vec::Vec};

/// One resource of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Relative to the sync folder ("" for the listed folder itself).
    pub path: String,
    pub folder: bool,
    /// The strong or weak entity tag as sent (quotes included).
    pub etag: String,
    pub length: u64,
    /// Unix seconds (`getlastmodified`).
    pub modified: Option<i64>,
}

/// Largest listing parsed (a folder of some ten thousand entries).
pub const MAX_BODY: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct Props {
    folder: bool,
    etag: Option<String>,
    length: Option<u64>,
    modified: Option<i64>,
}
#[derive(Default)]
struct Response {
    href: Option<String>,
    /// A `status` directly in `response` (no propstat): the whole
    /// resource's status.
    status: Option<u16>,
    props: Props,
}

/// The entries under `base` (the sync folder's encoded path) in a
/// multistatus body. Resources outside the folder, with a failed status
/// or a malformed href are left out.
pub fn parse_multistatus(body: &[u8], base: &str) -> Result<Vec<Entry>, Error> {
    if body.len() > MAX_BODY {
        return Err(Error::Answer);
    }
    let text = core::str::from_utf8(body).map_err(|_| Error::Answer)?;
    let mut out = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut response: Option<Response> = None;
    let mut propstat: Option<(Props, Option<u16>)> = None;
    let mut chars = String::new();
    let mut multistatus = false;
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let Some(open) = rest.find('<') else {
            break;
        };
        chars.push_str(&rest[..open]);
        let tag_start = at + open;
        let tag = &text[tag_start..];
        // Comments, CDATA, declarations and processing instructions.
        if let Some(r) = tag.strip_prefix("<!--") {
            at = tag_start + 4 + r.find("-->").ok_or(Error::Answer)? + 3;
            continue;
        }
        if let Some(r) = tag.strip_prefix("<![CDATA[") {
            let end = r.find("]]>").ok_or(Error::Answer)?;
            chars.push_str(&r[..end]);
            at = tag_start + 9 + end + 3;
            continue;
        }
        if tag.starts_with("<?") || tag.starts_with("<!") {
            at = tag_start + tag.find('>').ok_or(Error::Answer)? + 1;
            continue;
        }
        let close = tag_end(tag).ok_or(Error::Answer)?;
        let inner = &tag[1..close];
        at = tag_start + close + 1;
        if let Some(name) = inner.strip_prefix('/') {
            let local = local_name(name.trim());
            let open_name = stack.pop().ok_or(Error::Answer)?;
            if open_name != local {
                return Err(Error::Answer);
            }
            let value = unescape(chars.trim()).ok_or(Error::Answer)?;
            chars.clear();
            end_element(
                &local,
                &stack,
                value,
                &mut response,
                &mut propstat,
                &mut out,
                base,
            );
            continue;
        }
        let empty = inner.ends_with('/');
        let inner = inner.trim_end_matches('/');
        let name = inner.split(|c: char| c.is_whitespace()).next().unwrap_or("");
        let local = local_name(name);
        chars.clear();
        match local.as_str() {
            "multistatus" => multistatus = true,
            "response" if parent_is(&stack, "multistatus") => response = Some(Response::default()),
            "propstat" if response.is_some() => propstat = Some((Props::default(), None)),
            "collection" if parent_is(&stack, "resourcetype") => {
                if let Some((p, _)) = propstat.as_mut() {
                    p.folder = true;
                }
            }
            _ => (),
        }
        if empty {
            end_element(
                &local,
                &stack,
                String::new(),
                &mut response,
                &mut propstat,
                &mut out,
                base,
            );
        } else {
            stack.push(local);
        }
    }
    if !multistatus || !stack.is_empty() {
        return Err(Error::Answer);
    }
    Ok(out)
}
fn parent_is(stack: &[String], name: &str) -> bool {
    stack.last().is_some_and(|s| s == name)
}
/// The end of a start or end tag (`>` outside quoted attribute values).
fn tag_end(tag: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in tag.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => (),
        }
    }
    None
}
fn local_name(name: &str) -> String {
    let local = name.rsplit(':').next().unwrap_or(name);
    local.to_ascii_lowercase()
}
fn status_code(line: &str) -> Option<u16> {
    // "HTTP/1.1 200 OK"
    line.split_whitespace().nth(1)?.parse().ok()
}
fn end_element(
    local: &str,
    stack: &[String],
    value: String,
    response: &mut Option<Response>,
    propstat: &mut Option<(Props, Option<u16>)>,
    out: &mut Vec<Entry>,
    base: &str,
) {
    let in_prop = parent_is(stack, "prop");
    match local {
        "href" if parent_is(stack, "response") => {
            if let Some(r) = response.as_mut() {
                r.href = Some(value);
            }
        }
        "status" if parent_is(stack, "propstat") => {
            if let Some((_, s)) = propstat.as_mut() {
                *s = status_code(&value);
            }
        }
        "status" if parent_is(stack, "response") => {
            if let Some(r) = response.as_mut() {
                r.status = status_code(&value);
            }
        }
        "getetag" if in_prop => {
            if let Some((p, _)) = propstat.as_mut() {
                p.etag = Some(value);
            }
        }
        "getcontentlength" if in_prop => {
            if let Some((p, _)) = propstat.as_mut() {
                p.length = value.parse().ok();
            }
        }
        "getlastmodified" if in_prop => {
            if let Some((p, _)) = propstat.as_mut() {
                p.modified = door_http::parse_http_date(&value);
            }
        }
        "propstat" => {
            // Only the properties the server could read (200).
            if let (Some((p, Some(200))), Some(r)) = (propstat.take(), response.as_mut()) {
                r.props.folder |= p.folder;
                if p.etag.is_some() {
                    r.props.etag = p.etag;
                }
                if p.length.is_some() {
                    r.props.length = p.length;
                }
                if p.modified.is_some() {
                    r.props.modified = p.modified;
                }
            }
        }
        "response" => {
            let Some(r) = response.take() else {
                return;
            };
            if r.status.is_some_and(|s| s != 200) {
                return;
            }
            let Some(path) = r.href.as_deref().and_then(|h| href::relative(h, base)) else {
                return;
            };
            out.push(Entry {
                path,
                folder: r.props.folder,
                etag: r.props.etag.unwrap_or_default(),
                length: if r.props.folder {
                    0
                } else {
                    r.props.length.unwrap_or(0)
                },
                modified: r.props.modified,
            });
        }
        _ => (),
    }
}
/// XML's predefined entities and character references.
fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(';')?;
        let entity = &after[..end];
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    entity.strip_prefix('#')?.parse().ok()?
                };
                char::from_u32(code)?
            }
        };
        out.push(c);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: &str = "/remote.php/dav/files/me/NoneDrive/";
    /// As Nextcloud answers (prefix `d:`, a 404 propstat for a property
    /// collections lack).
    const NEXTCLOUD: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns">
 <d:response>
  <d:href>/remote.php/dav/files/me/NoneDrive/</d:href>
  <d:propstat><d:prop>
    <d:resourcetype><d:collection/></d:resourcetype>
    <d:getetag>&quot;6731f0a1&quot;</d:getetag>
    <d:getlastmodified>Wed, 13 Nov 2026 08:00:00 GMT</d:getlastmodified>
   </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:getcontentlength/></d:prop>
   <d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/remote.php/dav/files/me/NoneDrive/Report%20Q3.docx</d:href>
  <d:propstat><d:prop>
    <d:resourcetype/>
    <d:getetag>&quot;a1b2&quot;</d:getetag>
    <d:getcontentlength>48213</d:getcontentlength>
    <d:getlastmodified>Tue, 12 Nov 2026 17:45:10 GMT</d:getlastmodified>
   </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/remote.php/dav/files/me/NoneDrive/Photos/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>
   <d:getetag>"f00d"</d:getetag></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
</d:multistatus>"#;
    #[test]
    fn nextcloud_listing() {
        let e = parse_multistatus(NEXTCLOUD.as_bytes(), BASE).unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!((e[0].path.as_str(), e[0].folder), ("", true));
        assert_eq!(e[1].path, "Report Q3.docx");
        assert!(!e[1].folder);
        assert_eq!(e[1].etag, "\"a1b2\"");
        assert_eq!(e[1].length, 48213);
        assert_eq!(
            e[1].modified,
            door_http::parse_http_date("Tue, 12 Nov 2026 17:45:10 GMT")
        );
        assert_eq!((e[2].path.as_str(), e[2].folder), ("Photos", true));
        assert_eq!(e[2].etag, "\"f00d\"");
    }
    /// Apache mod_dav and IIS: upper-case or default-namespace prefixes,
    /// full URLs, CDATA, comments, a resource-level 404.
    #[test]
    fn other_servers_prefixes_and_forms() {
        let apache = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:ns0="DAV:">
<!-- listing -->
<D:response xmlns:lp1="DAV:">
<D:href>https://nas.home/NoneDrive/a&amp;b.txt</D:href>
<D:propstat><D:prop>
<lp1:resourcetype/><lp1:getcontentlength>5</lp1:getcontentlength>
<lp1:getetag><![CDATA["5-abc"]]></lp1:getetag>
</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
</D:response>
<D:response><D:href>/NoneDrive/gone.txt</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>
<D:response><D:href>/Other/x.txt</D:href><D:propstat><D:prop><D:resourcetype/></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#;
        let e = parse_multistatus(apache.as_bytes(), "/NoneDrive/").unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].path, "a&b.txt");
        assert_eq!((e[0].length, e[0].etag.as_str()), (5, "\"5-abc\""));
        let default_ns = r#"<multistatus xmlns="DAV:"><response><href>/d/f</href><propstat><prop><resourcetype><collection/></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;
        let e = parse_multistatus(default_ns.as_bytes(), "/d/").unwrap();
        assert_eq!((e[0].path.as_str(), e[0].folder), ("f", true));
    }
    #[test]
    fn malformed_answers_are_refused() {
        for bad in [
            "<d:multistatus xmlns:d=\"DAV:\"><d:response>",
            "<d:multistatus xmlns:d=\"DAV:\"></d:response></d:multistatus>",
            "<html><body>Login</body></html>",
            "<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>&bogus;</d:href></d:response></d:multistatus>",
        ] {
            assert_eq!(
                parse_multistatus(bad.as_bytes(), "/").map(|_| ()),
                Err(Error::Answer),
                "{bad}"
            );
        }
        assert_eq!(unescape("a&#x41;&#66;").as_deref(), Some("aAB"));
        assert_eq!(unescape("&#xD800;"), None);
    }
}
