include!(concat!(env!("OUT_DIR"), "/assets.rs"));

pub struct Asset {
    pub path: &'static str,
    pub body: &'static [u8],
    pub etag: &'static str,
    pub content_type: &'static str,
}

pub fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("ico") => "image/vnd.microsoft.icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn get(path: &str) -> Option<Asset> {
    let path = path.trim_start_matches('/');
    FILES
        .iter()
        .find(|(p, _, _)| *p == path)
        .map(|(p, body, etag)| Asset {
            path: p,
            body,
            etag,
            content_type: content_type(p),
        })
}

pub fn text(path: &str) -> Option<&'static str> {
    get(path).and_then(|a| std::str::from_utf8(a.body).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_is_embedded_and_stamped() {
        assert_eq!(BUILD.len(), 10);
        let index = text("index.html").unwrap();
        assert!(index.contains(&format!("/js/main.js?v={BUILD}\"")));
        assert!(index.contains(&format!("/style.css?v={BUILD}\"")));
        assert!(index.contains("{{site_name}}"));
        let main = text("/js/main.js").unwrap();
        assert!(main.contains(&format!("export const BUILD = \"{BUILD}\";")));
        assert!(main.contains(&format!("./lib.js?v={BUILD}\"")));
        let sw = text("sw.js").unwrap();
        assert!(sw.contains(&format!("const CACHE = \"cam-shell-{BUILD}\";")));
        assert!(sw.contains("\"/js/lib.js\""));
    }

    #[test]
    fn binary_assets_are_byte_identical() {
        let font = get("fonts/inter.woff2").unwrap();
        assert_eq!(font.content_type, "font/woff2");
        assert_eq!(font.body.len(), 58416);
        assert!(
            get("style.css").unwrap().body.starts_with(b":root")
                || !get("style.css").unwrap().body.is_empty()
        );
        assert!(get("nope.css").is_none());
        assert_eq!(get("icon.svg").unwrap().content_type, "image/svg+xml");
        assert_eq!(
            get("theme.js").unwrap().content_type,
            "text/javascript; charset=utf-8"
        );
    }
}
