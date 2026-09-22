//! The catalog shipped inside the binary.
//!
//! M1 has no network: every pack the binary can install is compiled in. That also means
//! the privileged step only ever writes content that was in the binary the user ran —
//! never content read from a user-writable file.

use handrail_core::catalog::{Catalog, Source};
use include_dir::{include_dir, Dir};

static CATALOG: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../catalog");

struct Embedded;

impl Source for Embedded {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        CATALOG.get_file(path).map(|f| f.contents().to_vec())
    }
    fn entries(&self, dir: &str) -> Vec<String> {
        let Some(d) = CATALOG.get_dir(dir) else {
            return vec![];
        };
        let mut v: Vec<String> = d
            .entries()
            .iter()
            .filter_map(|e| {
                e.path()
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .filter(|n| !n.starts_with('.'))
            .collect();
        v.sort();
        v
    }
}

/// The built-in catalog. It is validated by the test suite, so a failure here is a bug.
pub fn catalog() -> Catalog {
    Catalog::load(&Embedded).unwrap_or_else(|problems| {
        let list: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
        panic!("the built-in catalog is invalid:\n{}", list.join("\n"))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_embedded_catalog_loads() {
        let c = super::catalog();
        assert_eq!(c.packs.len(), 8);
        assert!(c.profiles.contains_key("baseline"));
    }
}
