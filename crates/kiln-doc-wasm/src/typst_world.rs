use std::collections::HashMap;
use std::sync::Mutex;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Features, Library, LibraryExt, World};

/// A `typst::World` implementation suitable for `wasm32-unknown-unknown`.
///
/// Differences from the native `KilnWorld`:
/// - Fonts loaded eagerly from `typst-assets` embedded bytes (no OS font scan).
/// - All file I/O is virtual -- no `std::fs` calls.
/// - Packages served from an in-memory store populated by the JS caller.
/// - `today()` uses `js_sys::Date` on WASM; returns a fixed date on native targets
///   so unit tests can run without `wasm-bindgen-test`.
pub struct WasmTypstWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main_id: FileId,
    sources: Mutex<HashMap<FileId, Source>>,
    files: Mutex<HashMap<FileId, Bytes>>,
    /// Virtual package FS. Key: "namespace/name/version/relative/path",
    /// e.g. "preview/fletcher/0.5.7/lib.typ".
    packages: Mutex<HashMap<String, Vec<u8>>>,
}

impl WasmTypstWorld {
    /// Create a world whose main source is `source_text`.
    /// Call `add_package_file` to register package files before `typst::compile`.
    pub fn from_source(source_text: &str) -> Self {
        let vpath = VirtualPath::new("__inline__.typ");
        let main_id = FileId::new(None, vpath);

        let library = Library::builder()
            .with_features(Features::from_iter([Feature::Html]))
            .build();

        let mut book = FontBook::new();
        let mut fonts = Vec::new();
        for font_data in typst_assets::fonts() {
            let bytes = Bytes::new(font_data);
            for font in Font::iter(bytes) {
                book.push(font.info().clone());
                fonts.push(font);
            }
        }

        let mut sources = HashMap::new();
        sources.insert(main_id, Source::new(main_id, source_text.to_string()));

        Self {
            library: LazyHash::new(library),
            book: LazyHash::new(book),
            fonts,
            main_id,
            sources: Mutex::new(sources),
            files: Mutex::new(HashMap::new()),
            packages: Mutex::new(HashMap::new()),
        }
    }

    /// Register a package file into the virtual package store.
    ///
    /// `pkg_path` must be `"namespace/name/version/relative/path"`,
    /// e.g. `"preview/fletcher/0.5.7/lib.typ"`.
    pub fn add_package_file(&self, pkg_path: String, data: Vec<u8>) {
        self.packages.lock().expect("packages lock").insert(pkg_path, data);
    }

    fn package_key(id: FileId) -> Option<String> {
        let pkg = id.package()?;
        let relative = id.vpath().as_rootless_path().to_string_lossy().into_owned();
        Some(format!("{}/{}/{}/{}", pkg.namespace, pkg.name, pkg.version, relative))
    }

    fn resolve_package_text(&self, id: FileId) -> FileResult<String> {
        let key = Self::package_key(id).ok_or(FileError::AccessDenied)?;
        let store = self.packages.lock().expect("packages lock");
        let data = store.get(&key).ok_or_else(|| {
            FileError::NotFound(id.vpath().as_rootless_path().to_path_buf())
        })?;
        String::from_utf8(data.clone()).map_err(|_| FileError::InvalidUtf8)
    }

    fn resolve_package_bytes(&self, id: FileId) -> FileResult<Bytes> {
        let key = Self::package_key(id).ok_or(FileError::AccessDenied)?;
        let data = {
            let store = self.packages.lock().expect("packages lock");
            store
                .get(&key)
                .ok_or_else(|| {
                    FileError::NotFound(id.vpath().as_rootless_path().to_path_buf())
                })?
                .clone()
        };
        Ok(Bytes::new(data))
    }
}

impl World for WasmTypstWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main_id
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        let mut cache = self.sources.lock().expect("sources lock");
        if let Some(s) = cache.get(&id) {
            return Ok(s.clone());
        }
        let text = self.resolve_package_text(id)?;
        let source = Source::new(id, text);
        cache.insert(id, source.clone());
        Ok(source)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let mut cache = self.files.lock().expect("files lock");
        if let Some(b) = cache.get(&id) {
            return Ok(b.clone());
        }
        let bytes = self.resolve_package_bytes(id)?;
        cache.insert(id, bytes.clone());
        Ok(bytes)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, offset: Option<i64>) -> Option<Datetime> {
        #[cfg(target_arch = "wasm32")]
        {
            let d = js_sys::Date::new_0();
            let (year, month, day, hour, min, sec) = if let Some(hours) = offset {
                let offset_ms = (hours as f64) * 3_600_000.0;
                let shifted = js_sys::Date::new(
                    &wasm_bindgen::JsValue::from_f64(d.get_time() + offset_ms),
                );
                (
                    shifted.get_utc_full_year() as i32,
                    (shifted.get_utc_month() + 1) as u8,
                    shifted.get_utc_date() as u8,
                    shifted.get_utc_hours() as u8,
                    shifted.get_utc_minutes() as u8,
                    shifted.get_utc_seconds() as u8,
                )
            } else {
                (
                    d.get_full_year() as i32,
                    (d.get_month() + 1) as u8,
                    d.get_date() as u8,
                    d.get_hours() as u8,
                    d.get_minutes() as u8,
                    d.get_seconds() as u8,
                )
            };
            Datetime::from_ymd_hms(year, month, day, hour, min, sec)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = offset;
            Datetime::from_ymd_hms(2024, 1, 1, 0, 0, 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use typst::layout::PagedDocument;

    #[test]
    fn wasm_world_compiles_simple_typst() {
        let source =
            "#set page(width: auto, height: auto, margin: 0pt)\nHello, *world*!";
        let world = WasmTypstWorld::from_source(source);
        let result = typst::compile::<PagedDocument>(&world);
        assert!(result.output.is_ok(), "simple typst should compile: {:?}", result.output);
    }

    #[test]
    fn wasm_world_package_file_roundtrip() {
        use typst::syntax::package::{PackageSpec, PackageVersion};

        let world =
            WasmTypstWorld::from_source("#set page(width: auto, height: auto, margin: 0pt)");
        world.add_package_file(
            "preview/test/0.1.0/lib.typ".to_string(),
            b"#let greet(name) = [Hello, #name!]".to_vec(),
        );

        let version: PackageVersion = "0.1.0".parse().unwrap();
        let spec = PackageSpec {
            namespace: "preview".into(),
            name: "test".into(),
            version,
        };
        let id = FileId::new(Some(spec), VirtualPath::new("lib.typ"));
        let source = world.source(id).unwrap();
        assert_eq!(source.text(), "#let greet(name) = [Hello, #name!]");
    }
}
