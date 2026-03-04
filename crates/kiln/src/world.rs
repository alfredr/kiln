use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Datelike, Timelike};
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Features, Library, LibraryExt, World};
use typst_kit::fonts::{FontSearcher, FontSlot, Fonts};

pub struct KilnWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<FontSlot>,
    main_id: FileId,
    root: PathBuf,
    sources: Mutex<HashMap<FileId, Source>>,
    files: Mutex<HashMap<FileId, Bytes>>,
}

impl KilnWorld {
    pub fn new(main_path: &Path, root: &Path) -> anyhow::Result<Self> {
        let main_path = main_path.canonicalize()?;
        let root = root.canonicalize()?;

        let relative = main_path
            .strip_prefix(&root)
            .map_err(|_| anyhow::anyhow!("main file must be under root"))?;
        let vpath = VirtualPath::new(relative);
        let main_id = FileId::new(None, vpath);

        let library = Library::builder()
            .with_features(Features::from_iter([Feature::Html]))
            .build();

        let Fonts { book, fonts } = FontSearcher::new().search();

        Ok(Self {
            library: LazyHash::new(library),
            book: LazyHash::new(book),
            fonts,
            main_id,
            root,
            sources: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
        })
    }

    fn resolve_path(&self, id: FileId) -> FileResult<PathBuf> {
        if let Some(package) = id.package() {
            return Err(FileError::Package(typst::diag::PackageError::NotFound(
                package.clone(),
            )));
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
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
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
        self.fonts.get(index)?.get()
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
