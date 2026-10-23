//! `pwpage` protocol: rendered page images for the viewer.
//!
//! URL path: `/{doc_id}/{page_index}/{scale_permille}` → PNG. Served from a worker thread. The
//! response is cacheable because a URL fully identifies the pixels for an unchanged open document.

use crate::state::AppState;
use tauri::http::{header, Response, StatusCode, Uri};

pub const MIN_SCALE_PERMILLE: u32 = 50;
pub const MAX_SCALE_PERMILLE: u32 = 8000;

#[derive(Debug, PartialEq, Eq)]
pub struct PageRequest {
    pub doc_id: u64,
    pub page: u32,
    pub scale_permille: u32,
}

/// Decode `%XX` escapes. The UI builds URLs with `convertFileSrc`, which encodes `/` as `%2F`.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn parse(path: &str) -> Option<PageRequest> {
    let path = percent_decode(path)?;
    let mut parts = path.trim_start_matches('/').split('/');
    let doc_id = parts.next()?.parse().ok()?;
    let page = parts.next()?.parse().ok()?;
    let scale_permille: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(MIN_SCALE_PERMILLE..=MAX_SCALE_PERMILLE).contains(&scale_permille) {
        return None;
    }
    Some(PageRequest { doc_id, page, scale_permille })
}

fn text(status: StatusCode, body: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body.as_bytes().to_vec())
        .unwrap_or_default()
}

pub fn serve(state: &AppState, uri: &Uri) -> Response<Vec<u8>> {
    let Some(req) = parse(uri.path()) else {
        return text(StatusCode::BAD_REQUEST, "bad page request");
    };
    let doc = match state.doc(req.doc_id) {
        Ok(d) => d,
        Err(e) => return text(StatusCode::NOT_FOUND, &e.code),
    };
    let call = pw_pdf::service::ReadCall::Render {
        path: doc.path.clone(),
        page: req.page,
        scale: req.scale_permille as f32 / 1000.0,
    };
    match state.read_bytes(&call, doc.password.as_deref()) {
        Ok(png) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/png")
            .header(header::CACHE_CONTROL, "private, max-age=3600")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(png)
            .unwrap_or_default(),
        Err(e) => text(StatusCode::UNPROCESSABLE_ENTITY, &e.code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_paths_only() {
        assert_eq!(parse("/3/0/1500"), Some(PageRequest { doc_id: 3, page: 0, scale_permille: 1500 }));
        assert_eq!(parse("3/12/1000"), Some(PageRequest { doc_id: 3, page: 12, scale_permille: 1000 }));
        // Regression: convertFileSrc encodes the separators.
        assert_eq!(parse("/1%2F0%2F1500"), Some(PageRequest { doc_id: 1, page: 0, scale_permille: 1500 }));
        assert_eq!(parse("/1%2f0%2f1500"), Some(PageRequest { doc_id: 1, page: 0, scale_permille: 1500 }));
        assert_eq!(parse("/1%2F0%2"), None);
        assert_eq!(parse("/1%ZZ0/1000"), None);
        for bad in
            ["", "/", "/a/0/1000", "/1/-1/1000", "/1/0/10", "/1/0/99999", "/1/0/1000/x", "/1/0", "/../../etc/passwd"]
        {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn serves_png_and_errors() {
        let dir = tempfile::tempdir().expect("tmp");
        let st = AppState::new(&dir.path().join("c"), &dir.path().join("d"), None);
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf/five-pages.pdf");
        let doc = st.open(fixture, None).expect("open");
        let ok = serve(&st, &format!("pwpage://localhost/{}/1/500", doc.doc_id).parse().expect("uri"));
        assert_eq!(ok.status(), StatusCode::OK);
        assert!(ok.body().starts_with(b"\x89PNG"));
        let missing = serve(&st, &"pwpage://localhost/999/0/1000".parse().expect("uri"));
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let oob = serve(&st, &format!("pwpage://localhost/{}/99/1000", doc.doc_id).parse().expect("uri"));
        assert_eq!(oob.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
