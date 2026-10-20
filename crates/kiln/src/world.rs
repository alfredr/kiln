use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{Datelike, Timelike};
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Features, Library, LibraryExt, World};
use typst_kit::fonts::{FontSearcher, FontSlot, Fonts};
use typst_kit::package::PackageStorage;

pub struct KilnWorld {
    shared: &'static Shared,
    main_id: FileId,
    root: PathBuf,
    sources: Mutex<HashMap<FileId, Source>>,
    files: Mutex<HashMap<FileId, Bytes>>,
    packages: PackageStorage,
}

struct NoopProgress;
impl typst_kit::download::Progress for NoopProgress {
    fn print_start(&mut self) {}
    fn print_progress(&mut self, _: &typst_kit::download::DownloadState) {}
    fn print_finish(&mut self, _: &typst_kit::download::DownloadState) {}
}

/// The parts of a world that are the same for every compile. Searching system
/// fonts is slow, and a page can hold dozens of compiled nodes, so these load
/// once per process.
struct Shared {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<FontSlot>,
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| {
        let library = Library::builder()
            .with_features(Features::from_iter([Feature::Html]))
            .build();
        let Fonts { book, fonts } = FontSearcher::new().search();
        Shared {
            library: LazyHash::new(library),
            book: LazyHash::new(book),
            fonts,
        }
    })
}

impl KilnWorld {
    /// Create a world from an in-memory source string.
    pub fn from_source(source_text: &str, root: &Path) -> anyhow::Result<Self> {
        // Use the path as-is; canonicalize() would fail on virtual/temp paths.
        let root = root.to_path_buf();
        let vpath = VirtualPath::new("__inline__.typ");
        let main_id = FileId::new(None, vpath);

        let packages = PackageStorage::new(
            None,
            None,
            typst_kit::download::Downloader::new("kiln/0.1"),
        );

        let mut sources = HashMap::new();
        sources.insert(main_id, Source::new(main_id, source_text.to_string()));

        Ok(Self {
            shared: shared(),
            main_id,
            root,
            sources: Mutex::new(sources),
            files: Mutex::new(HashMap::new()),
            packages,
        })
    }

    fn resolve_path(&self, id: FileId) -> FileResult<PathBuf> {
        if let Some(package) = id.package() {
            let pkg_dir = self
                .packages
                .prepare_package(package, &mut NoopProgress)
                .map_err(FileError::Package)?;
            return id.vpath().resolve(&pkg_dir).ok_or(FileError::AccessDenied);
        }

        let vpath = id.vpath();
        let resolved = vpath.resolve(&self.root).ok_or(FileError::AccessDenied)?;

        if !resolved.starts_with(&self.root) {
            return Err(FileError::AccessDenied);
        }

        Ok(resolved)
    }
}

impl World for KilnWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.shared.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.shared.book
    }

    fn main(&self) -> FileId {
        self.main_id
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        let mut cache = self.sources.lock().expect("sources cache lock poisoned");
        if let Some(source) = cache.get(&id) {
            return Ok(source.clone());
        }

        let path = self.resolve_path(id)?;
        let text = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => FileError::NotFound(path.clone()),
            std::io::ErrorKind::PermissionDenied => FileError::AccessDenied,
            _ => FileError::Other(Some(ecow::eco_format!("{e}"))),
        })?;

        let source = Source::new(id, text);
        cache.insert(id, source.clone());
        Ok(source)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let mut cache = self.files.lock().expect("files cache lock poisoned");
        if let Some(bytes) = cache.get(&id) {
            return Ok(bytes.clone());
        }

        let path = self.resolve_path(id)?;
        let data = std::fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => FileError::NotFound(path.clone()),
            std::io::ErrorKind::PermissionDenied => FileError::AccessDenied,
            _ => FileError::Other(Some(ecow::eco_format!("{e}"))),
        })?;

        let bytes = Bytes::new(data);
        cache.insert(id, bytes.clone());
        Ok(bytes)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.shared.fonts.get(index)?.get()
    }

    fn today(&self, offset: Option<i64>) -> Option<Datetime> {
        let now = if let Some(hours) = offset {
            let offset = chrono::FixedOffset::east_opt(hours as i32 * 3600)?;
            chrono::Utc::now().with_timezone(&offset).naive_local()
        } else {
            chrono::Local::now().naive_local()
        };

        Datetime::from_ymd_hms(
            now.year(),
            now.month() as u8,
            now.day() as u8,
            now.hour() as u8,
            now.minute() as u8,
            now.second() as u8,
        )
    }
}
