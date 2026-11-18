//! File names: what the server calls a file, and keeping that name safe
//! as one path segment inside the download folder.

use std::path::Path;
use std::time::Duration;

use crate::model::now_secs;

pub fn unique_download_name(dir: &Path, filename: &str) -> String {
    let filename = sanitize_file_name(filename);
    if !dir.join(&filename).exists() {
        return filename;
    }
    let path = Path::new(&filename);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "download".to_string());
    let extension = path
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    for index in 1..10_000 {
        let candidate = format!("{stem} ({index}){extension}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{stem}-{}{extension}", now_secs())
}

pub fn suggest_name(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .filter(|name| !name.is_empty())
        .unwrap_or("download")
        .to_string()
}

pub fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string())
}

/// Asks the server for the real file name, falling back to the URL path.
pub async fn resolve_filename(url: &str) -> String {
    resolve_filename_with(url, &[], None).await
}

/// Like [`resolve_filename`], with the request headers and proxy of the
/// download itself (cookies matter for file hosts).
///
/// Tries `HEAD` first; many hosts reject it, so a one-byte ranged `GET`
/// follows. After redirects the final URL usually names the file better
/// than the original (`/get?id=1` → `/files/report.pdf`).
pub async fn resolve_filename_with(
    url: &str,
    headers: &[(String, String)],
    proxy: Option<&str>,
) -> String {
    let mut builder = reqwest::Client::builder()
        .http1_only()
        .timeout(Duration::from_secs(8));
    if let Some(proxy) = proxy.filter(|value| !value.is_empty())
        && let Ok(proxy) = reqwest::Proxy::all(proxy)
    {
        builder = builder.proxy(proxy);
    }
    let Ok(client) = builder.build() else {
        return from_url(url);
    };
    let mut map = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::try_from(name.as_str()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            map.insert(name, value);
        }
    }

    let mut final_url = None;
    let head = client.head(url).headers(map.clone()).send().await;
    let response = match head {
        Ok(resp) if resp.status().is_success() => Some(resp),
        other => {
            if let Ok(resp) = &other {
                final_url = Some(resp.url().to_string());
            }
            client
                .get(url)
                .headers(map)
                .header(reqwest::header::RANGE, "bytes=0-0")
                .send()
                .await
                .ok()
                .filter(|resp| resp.status().is_success())
        }
    };
    if let Some(resp) = response {
        if let Some(name) = resp
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .and_then(filename_from_disposition)
        {
            return name;
        }
        final_url = Some(resp.url().to_string());
    }

    // the redirect target names the file, unless it is a bare path like "/"
    if let Some(final_url) = final_url.filter(|value| value != url) {
        let name = from_url(&final_url);
        if name != "download" && Path::new(&name).extension().is_some() {
            return name;
        }
    }
    from_url(url)
}

/// Reads `filename*=` (RFC 5987) and falls back to the plain `filename=`.
pub fn filename_from_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    for part in split_params(value) {
        let Some((key, raw)) = part.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                let raw = raw.trim().trim_matches('"');
                // charset'lang'value
                let encoded = raw.splitn(3, '\'').nth(2).unwrap_or(raw);
                extended = Some(percent_decode(encoded));
            }
            "filename" => {
                let raw = raw.trim();
                let unquoted = raw
                    .strip_prefix('"')
                    .and_then(|rest| rest.strip_suffix('"'))
                    .map(|inner| inner.replace("\\\"", "\""))
                    .unwrap_or_else(|| raw.to_string());
                plain = Some(unquoted);
            }
            _ => {}
        }
    }
    let name = extended.or(plain)?;
    // a server may send a path; only its last segment is a name
    let name = name.rsplit(['/', '\\']).next().unwrap_or("").to_string();
    let name = sanitize_file_name(&name);
    if name.is_empty() || name == "download" {
        None
    } else {
        Some(name)
    }
}

/// Splits header parameters on `;` outside double quotes.
fn split_params(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (index, ch) in value.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                parts.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

/// Keeps a remote name usable as one path segment, so a crafted URL cannot
/// escape the download folder.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned.chars().take(180).collect()
    }
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

pub fn from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("").trim();
    if name.is_empty() {
        "download".to_string()
    } else {
        sanitize_file_name(&percent_decode(name))
    }
}

/// Parses a `dwv://add?url=<encoded>` deep link into the target URL.
pub fn parse_deep_link(link: &str) -> Option<String> {
    let link = link.trim();
    let rest = link.strip_prefix("dwv://")?;
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p.trim_end_matches('/'), q),
        None => (rest.trim_end_matches('/'), ""),
    };
    if !path.is_empty() && path != "add" {
        return None;
    }
    for param in query.split('&') {
        if param.is_empty() {
            continue;
        }
        let (key, value) = param.split_once('=')?;
        if key == "url" {
            return Some(percent_decode(value));
        }
    }
    // bare dwv:// opens the app without adding anything
    if query.is_empty() {
        return Some(String::new());
    }
    None
}

pub fn deep_link_type(link: &str) -> Option<String> {
    let link = link.trim();
    let rest = link.strip_prefix("dwv://")?;
    let query = rest.split_once('?').map(|x| x.1).unwrap_or("");
    for param in query.split('&') {
        if param.is_empty() {
            continue;
        }
        if let Some((key, value)) = param.split_once('=')
            && key == "type"
        {
            return Some(percent_decode(value));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_quoted_disposition_names() {
        assert_eq!(
            filename_from_disposition("attachment; filename=\"report 2026.zip\""),
            Some("report 2026.zip".to_string())
        );
        assert_eq!(
            filename_from_disposition("attachment;filename=data.csv"),
            Some("data.csv".to_string())
        );
        assert_eq!(
            filename_from_disposition("attachment; filename=\"a;b.txt\"; size=10"),
            Some("a;b.txt".to_string())
        );
    }

    #[test]
    fn extended_name_wins_over_plain() {
        let value = "attachment; filename=\"fallback.pdf\"; filename*=UTF-8''%D0%9E%D1%82%D1%87%D0%B5%D1%82.pdf";
        assert_eq!(
            filename_from_disposition(value),
            Some("Отчет.pdf".to_string())
        );
    }

    #[test]
    fn disposition_paths_cannot_escape() {
        assert_eq!(
            filename_from_disposition("attachment; filename=\"../../evil.exe\""),
            Some("evil.exe".to_string())
        );
        assert_eq!(filename_from_disposition("inline"), None);
        assert_eq!(filename_from_disposition("attachment; filename=\"\""), None);
    }
}
