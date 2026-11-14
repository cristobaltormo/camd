use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use regex_lite::{Captures, Regex};
use sha1::{Digest, Sha1};

const BINARY_SUFFIXES: [&str; 5] = ["css", "webmanifest", "svg", "png", "woff2"];

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("web directory")
        .map(|e| e.expect("web entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn parts(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

struct Stamper {
    asset: Regex,
    cache: Regex,
    build: Regex,
}

impl Stamper {
    fn new() -> Stamper {
        Stamper {
            asset: Regex::new(r#"((?:\./[a-z]+\.js)|(?:/style\.css)|(?:/theme\.js)|(?:/js/main\.js))(\?v=[0-9a-z.]+)?(["'])"#)
                .unwrap(),
            cache: Regex::new(r#"const CACHE = "cam-shell-[0-9a-z.]+";"#).unwrap(),
            build: Regex::new(r#"export const BUILD = "[0-9a-z.]+";"#).unwrap(),
        }
    }

    fn apply(&self, text: &str, stamp: Option<&str>) -> String {
        let suffix = stamp.map(|b| format!("?v={b}")).unwrap_or_default();
        let text = self
            .asset
            .replace_all(text, |c: &Captures| format!("{}{}{}", &c[1], suffix, &c[3]));
        let label = stamp.unwrap_or("X");
        let text = self.cache.replace_all(
            &text,
            format!("const CACHE = \"cam-shell-{label}\";").as_str(),
        );
        self.build
            .replace_all(&text, format!("export const BUILD = \"{label}\";").as_str())
            .into_owned()
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let web = root.join("web");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=web");

    let mut all = Vec::new();
    files(&web, &mut all);
    let stamped: Vec<PathBuf> = all
        .iter()
        .filter(|p| {
            let rel = p.strip_prefix(&web).unwrap();
            rel == Path::new("index.html")
                || rel == Path::new("sw.js")
                || (rel.parent() == Some(Path::new("js"))
                    && rel.extension().is_some_and(|e| e == "js"))
        })
        .cloned()
        .collect();
    let mut hashed: Vec<PathBuf> = all
        .iter()
        .filter(|p| {
            stamped.contains(p)
                || p.extension()
                    .is_some_and(|e| BINARY_SUFFIXES.contains(&e.to_string_lossy().as_ref()))
        })
        .cloned()
        .collect();
    hashed.sort_by_key(|p| parts(p));

    let stamper = Stamper::new();
    let mut digest = Sha1::new();
    for path in &hashed {
        let raw = std::fs::read(path).unwrap();
        digest.update(path.file_name().unwrap().to_string_lossy().as_bytes());
        let textual = stamped.contains(path) || path.extension().is_some_and(|e| e == "css");
        if textual {
            digest.update(
                stamper
                    .apply(&String::from_utf8(raw).unwrap(), None)
                    .as_bytes(),
            );
        } else {
            digest.update(&raw);
        }
    }
    let build: String = hex(&digest.finalize())[..10].to_string();

    let mut table = String::new();
    writeln!(table, "pub const BUILD: &str = {build:?};").unwrap();
    writeln!(table, "pub static FILES: &[(&str, &[u8], &str)] = &[").unwrap();
    for path in &all {
        let rel = path
            .strip_prefix(&web)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let source = if stamped.contains(path) {
            let target = out.join("web").join(&rel);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            let text = std::fs::read_to_string(path).unwrap();
            std::fs::write(&target, stamper.apply(&text, Some(&build))).unwrap();
            target
        } else {
            path.clone()
        };
        let etag = hex(&Sha1::digest(std::fs::read(&source).unwrap()))[..16].to_string();
        writeln!(
            table,
            "    ({rel:?}, include_bytes!({:?}), {etag:?}),",
            source.to_string_lossy()
        )
        .unwrap();
    }
    writeln!(table, "];").unwrap();
    std::fs::write(out.join("assets.rs"), table).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
