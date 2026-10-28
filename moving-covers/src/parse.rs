//! Apple's answers, read without I/O: the iTunes Search API's albums ([album_candidates]), the web
//! player's scripts and developer token ([bundle_paths], [token]) and the catalogue album's
//! `editorialVideo` ([square_video]). These shapes are undocumented (observed in 2026); a change yields
//! no result or an error, never a wrong video.

use serde_json::Value;

/// An answer that isn't the shape asked for: a refusal, an error page, a changed API.
#[derive(Debug)]
pub(crate) struct Unreadable(pub(crate) String);

fn unreadable(what: &str) -> Unreadable {
    Unreadable(what.to_string())
}

/// Strips the accent from a lowercase Latin letter.
fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        c => c,
    }
}

/// Lowercase words: accents folded, apostrophes dropped, `&` as "and", other punctuation runs as one space.
fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut gap = false;
    for c in s.chars().flat_map(char::to_lowercase).map(fold) {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{02bc}' | '`' | '\u{b4}') {
            continue;
        }
        if c == '&' {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str("and");
            gap = true;
        } else if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// [norm]ed title without bracketed parts and trailing edition suffixes (" - Single", " - Deluxe Edition").
fn base(s: &str) -> String {
    let mut plain = String::with_capacity(s.len());
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            c if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let mut title = plain.as_str();
    while let Some((head, tail)) = title.rsplit_once(" - ") {
        let tail = norm(tail);
        let edition = tail == "single"
            || tail == "ep"
            || ["deluxe", "edition", "remaster", "remastered", "version", "expanded", "anniversary", "bonus"]
                .iter()
                .any(|w| tail.split(' ').any(|t| t == *w));
        if !edition {
            break;
        }
        title = head;
    }
    norm(title)
}

/// The credited names in "A feat. B", "A & B", "A, B", "A x B" and the like.
fn credited(s: &str) -> Vec<String> {
    let lower = s.to_lowercase();
    let mut parts = vec![lower.as_str()];
    for sep in [",", ";", "/", " feat. ", " feat ", " ft. ", " featuring ", " & ", " and ", " x ", " \u{d7} ", " with "] {
        parts = parts.iter().flat_map(|p| p.split(sep)).collect();
    }
    parts.into_iter().map(norm).filter(|p| !p.is_empty()).collect()
}

/// 2 for the same artist name, 1 when either side's first credit appears on the other, else 0.
fn artist_match(want: &str, hit: &str) -> u8 {
    let (w, h) = (norm(want), norm(hit));
    if w.is_empty() || h.is_empty() {
        return 0;
    }
    if w == h {
        return 2;
    }
    let (wc, hc) = (credited(want), credited(hit));
    let shared = wc.first().is_some_and(|f| hc.contains(f)) || hc.first().is_some_and(|f| wc.contains(f));
    u8::from(shared)
}

/// Up to three catalogue ids from an iTunes Search answer, best first. Hits need the same artist and
/// title; a title matching only without edition words also needs a close track count (or year if the
/// count is unknown), since another edition may have a different cover. Ranked by track count and
/// year. A non-search answer is an error, so a failed request is not cached as "none".
pub(crate) fn album_candidates(json: &str, artist: &str, album: &str, tracks: u32, year: u32) -> Result<Vec<String>, Unreadable> {
    let v: Value = serde_json::from_str(json).map_err(|e| unreadable(&e.to_string()))?;
    let results = v.get("results").and_then(Value::as_array).ok_or_else(|| unreadable("no results list"))?;
    let (want_exact, want_base) = (norm(album), base(album));
    let mut ranked: Vec<(i32, usize, String)> = Vec::new();
    for (order, r) in results.iter().enumerate() {
        if r.get("wrapperType").and_then(Value::as_str).is_some_and(|w| w != "collection") {
            continue;
        }
        let id = match r.get("collectionId") {
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::String(s)) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => s.clone(),
            _ => continue,
        };
        let name = r.get("collectionName").and_then(Value::as_str).unwrap_or_default();
        let by = r.get("artistName").and_then(Value::as_str).unwrap_or_default();
        let who = artist_match(artist, by);
        if who == 0 {
            continue;
        }
        let hit_tracks = r.get("trackCount").and_then(Value::as_u64).unwrap_or(0) as i64;
        let hit_year = r
            .get("releaseDate")
            .and_then(Value::as_str)
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse::<i64>().ok())
            .unwrap_or(0);
        let counted = tracks > 0 && hit_tracks > 0;
        let dated = year > 0 && hit_year > 0;
        let track_gap = (hit_tracks - tracks as i64).abs();
        let year_gap = (hit_year - year as i64).abs();
        let exact = !want_exact.is_empty() && norm(name) == want_exact;
        if !exact {
            let same_base = !want_base.is_empty() && base(name) == want_base;
            let backed = if counted { track_gap <= 2 } else { dated && year_gap <= 1 };
            if !same_base || !backed {
                continue;
            }
        }
        let mut score = (if exact { 30 } else { 18 }) + (if who == 2 { 10 } else { 5 });
        if counted {
            score += match track_gap {
                0 => 12,
                1 | 2 => 5,
                _ => -8,
            };
        }
        if dated {
            score += match year_gap {
                0 => 6,
                1 => 3,
                _ => 0,
            };
        }
        if !ranked.iter().any(|(_, _, seen)| *seen == id) {
            ranked.push((score, order, id));
        }
    }
    // By score, then search order.
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    Ok(ranked.into_iter().take(3).map(|(_, _, id)| id).collect())
}

/// The square motion video's HLS URL from a catalogue album answer; None if the album has none. Only the
/// square one aligns with the still cover; the tall one is a different framing.
pub(crate) fn square_video(json: &str) -> Result<Option<String>, Unreadable> {
    let v: Value = serde_json::from_str(json).map_err(|e| unreadable(&e.to_string()))?;
    let album = v.get("data").and_then(Value::as_array).and_then(|d| d.first()).ok_or_else(|| unreadable("no album in the answer"))?;
    let Some(video) = album.get("attributes").and_then(|a| a.get("editorialVideo")) else {
        return Ok(None);
    };
    let url = ["motionDetailSquare", "motionSquareVideo1x1"]
        .iter()
        .filter_map(|k| video.get(*k)?.get("video")?.as_str())
        .find(|u| u.starts_with("https://"));
    Ok(url.map(str::to_string))
}

/// The `/assets/index…js` script paths in a web player page, in order, deduplicated.
pub(crate) fn bundle_paths(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("/assets/") {
        rest = &rest[at..];
        let end = rest.find(|c: char| c == '"' || c == '\'' || c == '<' || c == '>' || c == '(' || c == ')' || c.is_whitespace()).unwrap_or(rest.len());
        let path = &rest[..end];
        // Directly under /assets/ ("/assets/vendor.js/assets/index.js" is two paths).
        let file = &path["/assets/".len()..];
        if !file.contains('/') && file.starts_with("index") && file.ends_with(".js") && !out.iter().any(|p| p == path) {
            out.push(path.to_string());
        }
        rest = &rest["/assets/".len()..];
    }
    out
}

/// The web player's developer token in a script: a JWT unexpired for another minute at `now_s`, issued
/// by `AMPWebPlay` if any (other JWTs are refused by the catalogue), else the first readable one.
pub(crate) fn token(js: &[u8], now_s: i64) -> Option<String> {
    let mut fallback: Option<String> = None;
    let mut i = 0;
    while i + 3 <= js.len() {
        if &js[i..i + 3] != b"eyJ" {
            i += 1;
            continue;
        }
        let mut end = i;
        let mut dots = 0;
        while end < js.len() && (js[end].is_ascii_alphanumeric() || matches!(js[end], b'-' | b'_' | b'.')) {
            dots += usize::from(js[end] == b'.');
            end += 1;
        }
        // ASCII by construction.
        let token = std::str::from_utf8(&js[i..end]).unwrap_or_default();
        i = end.max(i + 1);
        let parts: Vec<&str> = token.split('.').collect();
        if dots != 2 || token.len() < 80 || parts.iter().any(|p| p.is_empty()) {
            continue;
        }
        let read = |p: &str| base64(p).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(Value::is_object);
        let (Some(header), Some(payload)) = (read(parts[0]), read(parts[1])) else { continue };
        if payload.get("exp").and_then(Value::as_i64).is_some_and(|exp| exp <= now_s + 60) {
            continue;
        }
        let web = payload.get("iss").and_then(Value::as_str) == Some("AMPWebPlay")
            || header.get("kid").and_then(Value::as_str).is_some_and(|k| k.contains("WebPlay"));
        if web {
            return Some(token.to_string());
        }
        fallback.get_or_insert_with(|| token.to_string());
    }
    fallback
}

/// Bytes of base64 in either alphabet (standard or URL-safe), padding optional; None for other text.
fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in s.trim_end_matches('=').bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// `s` as a form value: unreserved bytes as they are, spaces as `+`, the rest `%XX`.
pub(crate) fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn enc(bytes: &[u8]) -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
            for k in 0..=chunk.len() {
                out.push(A[(n >> (18 - 6 * k) & 63) as usize] as char);
            }
        }
        out
    }

    pub(crate) fn jwt(header: &str, payload: &str) -> String {
        format!("{}.{}.{}", enc(header.as_bytes()), enc(payload.as_bytes()), "s".repeat(43))
    }

    fn hit(id: u64, name: &str, artist: &str, tracks: u32, date: &str) -> String {
        format!(r#"{{"wrapperType":"collection","collectionType":"Album","collectionId":{id},"collectionName":"{name}","artistName":"{artist}","trackCount":{tracks},"releaseDate":"{date}"}}"#)
    }

    fn search(hits: &[String]) -> String {
        format!(r#"{{"resultCount":{},"results":[{}]}}"#, hits.len(), hits.join(","))
    }

    #[test]
    fn album_matching() {
        assert_eq!(norm("Don't Stop Me Now"), "dont stop me now");
        assert_eq!(norm("Simon & Garfunkel"), "simon and garfunkel");
        assert_eq!(norm("Beyoncé"), "beyonce");
        assert_eq!(norm("Łąki Łan"), "laki lan");
        assert_eq!(norm("  AM  (Deluxe)"), "am deluxe");
        assert_eq!(base("Abbey Road (Remastered 2019)"), "abbey road");
        assert_eq!(base("Better - Single"), "better");
        assert_eq!(base("Rumours - Deluxe Edition"), "rumours");
        // A dash in the title itself stays.
        assert_eq!(base("Songs - For the Deaf"), "songs for the deaf");

        // Exact album ranks before editions.
        let json = search(&[
            hit(1, "Better - Single", "Khalid", 1, "2018-09-14T07:00:00Z"),
            hit(2, "Better (Deluxe)", "Khalid", 14, "2019-04-01T07:00:00Z"),
            hit(3, "Better", "Khalid", 12, "2019-04-01T07:00:00Z"),
            hit(4, "Better", "Someone Else", 12, "2019-04-01T07:00:00Z"),
        ]);
        let ids = album_candidates(&json, "Khalid", "Better", 12, 2019).unwrap();
        assert_eq!(ids, vec!["3".to_string(), "2".to_string()]);

        // Edition match needs track count or year.
        let json = search(&[hit(9, "Better - Single", "Khalid", 1, "2018-09-14T07:00:00Z")]);
        assert!(album_candidates(&json, "Khalid", "Better", 12, 2019).unwrap().is_empty());
        assert_eq!(album_candidates(&json, "Khalid", "Better", 1, 2018).unwrap(), vec!["9".to_string()]);
        // Without a count, the year decides.
        assert_eq!(album_candidates(&json, "Khalid", "Better", 0, 2018).unwrap(), vec!["9".to_string()]);
        assert!(album_candidates(&json, "Khalid", "Better", 0, 0).unwrap().is_empty());

        // Other artist never matches.
        let json = search(&[hit(5, "Greatest Hits", "Queen", 17, "1981-10-26T08:00:00Z")]);
        assert!(album_candidates(&json, "ABBA", "Greatest Hits", 17, 1981).unwrap().is_empty());

        // Shared credit matches artist.
        let json = search(&[hit(6, "Watch the Throne", "JAY-Z & Kanye West", 12, "2011-08-08T07:00:00Z")]);
        let ids = album_candidates(&json, "Jay-Z", "Watch The Throne", 12, 2011).unwrap();
        assert_eq!(ids, vec!["6".to_string()]);
    }

    #[test]
    fn answers() {
        assert!(album_candidates("<html>Too many requests</html>", "a", "b", 0, 0).is_err());
        assert!(album_candidates(r#"{"errorMessage":"Invalid value(s) for key(s): [country]"}"#, "a", "b", 0, 0).is_err());
        assert!(album_candidates(r#"{"resultCount":0,"results":[]}"#, "a", "b", 0, 0).unwrap().is_empty());

        // Only square video is taken.
        let found = r#"{"data":[{"id":"1","type":"albums","attributes":{"name":"X","editorialVideo":{
            "motionDetailTall":{"video":"https://mvod.itunes.apple.com/tall.m3u8"},
            "motionDetailSquare":{"video":"https://mvod.itunes.apple.com/square.m3u8"}}}}]}"#;
        assert_eq!(square_video(found).unwrap().as_deref(), Some("https://mvod.itunes.apple.com/square.m3u8"));
        let other_name = r#"{"data":[{"attributes":{"editorialVideo":{"motionSquareVideo1x1":{"video":"https://a/b.m3u8"}}}}]}"#;
        assert_eq!(square_video(other_name).unwrap().as_deref(), Some("https://a/b.m3u8"));
        let tall_only = r#"{"data":[{"attributes":{"editorialVideo":{"motionDetailTall":{"video":"https://a/t.m3u8"}}}}]}"#;
        assert_eq!(square_video(tall_only).unwrap(), None);
        let none = r#"{"data":[{"attributes":{"name":"Plain"}}]}"#;
        assert_eq!(square_video(none).unwrap(), None);
        // A refusal is an error, not "none".
        assert!(square_video(r#"{"errors":[{"status":"401"}]}"#).is_err());
        assert!(square_video("").is_err());
    }

    #[test]
    fn bundle_paths_are_index_scripts() {
        let html = r#"<link rel="modulepreload" href="/assets/index~8f2a1c.js"><script type="module" crossorigin src="/assets/index~8f2a1c.js"></script>
            <script nomodule src="/assets/index-legacy~77aa.js"></script><img src="/assets/logo.svg"> /assets/vendor~1.js/assets/index~z.js"#;
        assert_eq!(bundle_paths(html), vec!["/assets/index~8f2a1c.js", "/assets/index-legacy~77aa.js", "/assets/index~z.js"]);
        assert_eq!(bundle_paths("x /assets/index~a.js\u{a0}y"), vec!["/assets/index~a.js"]);
    }

    #[test]
    fn tokens() {
        let now = 1_790_000_000;
        let other = jwt(r#"{"alg":"ES256","kid":"OTHER"}"#, &format!(r#"{{"iss":"SomethingElse","exp":{}}}"#, now + 90_000));
        let web = jwt(r#"{"alg":"ES256","typ":"JWT","kid":"WebPlayKid"}"#, &format!(r#"{{"iss":"AMPWebPlay","iat":{now},"exp":{}}}"#, now + 90_000));
        let js = format!(r#"const a="{other}";function f(){{return"{web}"}}"#);
        assert_eq!(token(js.as_bytes(), now).as_deref(), Some(web.as_str()));
        let js = format!(r#"x="{other}""#);
        assert_eq!(token(js.as_bytes(), now).as_deref(), Some(other.as_str()));

        // Expired or malformed token rejected.
        let now = 1_790_000_000;
        let stale = jwt(r#"{"kid":"WebPlayKid"}"#, &format!(r#"{{"iss":"AMPWebPlay","exp":{}}}"#, now + 30));
        assert_eq!(token(stale.as_bytes(), now), None);
        assert_eq!(token(b"eyJshort.abc.def", now), None);
        assert_eq!(token(format!("eyJ{}", "a".repeat(200)).as_bytes(), now), None);
        assert_eq!(token(format!("eyJ{}.{}.{}", "a".repeat(40), "b".repeat(40), "c".repeat(40)).as_bytes(), now), None);
    }


    #[test]
    fn base64_reads_both_alphabets() {
        assert_eq!(base64("aGk_Pz8-").as_deref(), Some(&b"hi???>"[..]));
        assert_eq!(base64("aGk/Pz8+").as_deref(), Some(&b"hi???>"[..]));
        assert_eq!(base64("aGk=").as_deref(), Some(&b"hi"[..]));
        assert_eq!(base64("a b"), None);
    }

    #[test]
    fn form_values_are_escaped() {
        assert_eq!(form_encode("Simon & Garfunkel Bookends"), "Simon+%26+Garfunkel+Bookends");
        assert_eq!(form_encode("Beyoncé"), "Beyonc%C3%A9");
    }
}
