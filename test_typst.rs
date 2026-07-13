
use typst::World;
use typst::diag::{FileResult, FileError};
use typst::foundations::{Bytes, Datetime, Prehashed};
use typst::syntax::{FileId, Source};
use typst::text::{Font, FontBook};
use typst::Library;

struct MinimalWorld {
    library: Prehashed<Library>,
    book: Prehashed<FontBook>,
    font: Font,
    source: Source,
}

impl MinimalWorld {
    fn new(text: &str) -> Self {
        let font_data = include_bytes!("assets/bank_font.ttf");
        let font = Font::new(Bytes::from_static(font_data), 0).unwrap();
        let mut book = FontBook::new();
        book.push(font.info().clone());
        let library = Prehashed::new(Library::builder().build());
        let source = Source::detached(text);
        Self {
            library,
            book: Prehashed::new(book),
            font,
            source,
        }
    }
}

impl World for MinimalWorld {
    fn library(&self) -> &Prehashed<Library> { &self.library }
    fn book(&self) -> &Prehashed<FontBook> { &self.book }
    fn main(&self) -> Source { self.source.clone() }
    fn source(&self, _id: FileId) -> FileResult<Source> { Ok(self.source.clone()) }
    fn book(&self) -> &Prehashed<FontBook> { &self.book }
    fn file(&self, _id: FileId) -> FileResult<Bytes> { Err(FileError::NotFound(std::path::PathBuf::new())) }
    fn font(&self, _index: usize) -> Option<Font> { Some(self.font.clone()) }
    fn today(&self, _offset: Option<i64>) -> Option<Datetime> { None }
}

fn main() {
    let world = MinimalWorld::new("Hello");
    let document = typst::compile(&world).unwrap();
    let pdf = typst_pdf::pdf(&document, &Default::default(), None);
    println!("Compiled to {} bytes", pdf.len());
}
