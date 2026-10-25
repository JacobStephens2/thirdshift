//! Counted and mutating filesystem adapter for capture regressions.

use std::cell::RefCell;
use std::collections::BTreeMap;

use super::*;

type OnSample = Box<dyn FnMut(&str, &Path)>;

#[derive(Default)]
pub(in crate::evals::experiment) struct Observed {
    pub counts: RefCell<BTreeMap<(&'static str, PathBuf), usize>>,
    pub after: RefCell<Option<OnSample>>,
    pub read_error: Option<(PathBuf, io::ErrorKind)>,
}

impl Observed {
    fn sampled<T>(&self, operation: &'static str, path: &Path, result: T) -> T {
        *self
            .counts
            .borrow_mut()
            .entry((operation, path.into()))
            .or_default() += 1;
        if let Some(after) = self.after.borrow_mut().as_mut() {
            after(operation, path);
        }
        result
    }
}

impl Files for Observed {
    fn entries(&self, path: &Path) -> io::Result<Vec<Entry>> {
        self.sampled("entries", path, Native.entries(path))
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let result = match &self.read_error {
            Some((failed, kind)) if failed == path => Err(io::Error::from(*kind)),
            _ => Native.read(path),
        };
        self.sampled("read", path, result)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.sampled("canonicalize", path, Native.canonicalize(path))
    }

    fn open(&self, path: &Path) -> io::Result<File> {
        self.sampled("open", path, Native.open(path))
    }
}
