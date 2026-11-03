#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FileKind { Text, Config }

impl FileKind {
    pub fn from_path(path: &str) -> Self {
        match path.rsplit_once('.') {
            Some((_, ext)) if ext.eq_ignore_ascii_case("config")
                           || ext.eq_ignore_ascii_case("conf") => FileKind::Config,
            _ => FileKind::Text, // .txt and anything unknown
        }
    }
}