use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

/// A pointer to a local file or a language-server-owned archive source.
///
/// Cloning this type is cheap: the internal representation uses an Arc.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Uri {
    File(Arc<Path>),
    Archive(Arc<helix_stdx::Url>),
}

impl Uri {
    // This clippy allow mirrors helix_stdx::Url::from_file_path
    #[allow(clippy::result_unit_err)]
    pub fn to_url(&self) -> Result<helix_stdx::Url, ()> {
        match self {
            Uri::File(path) => helix_stdx::Url::from_file_path(path),
            Uri::Archive(url) => Ok((**url).clone()),
        }
    }

    pub fn as_path(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path),
            Self::Archive(_) => None,
        }
    }
}

impl From<PathBuf> for Uri {
    fn from(path: PathBuf) -> Self {
        Self::File(path.into())
    }
}

impl From<&Path> for Uri {
    fn from(path: &Path) -> Self {
        Self::File(path.into())
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(path) => write!(f, "{}", path.display()),
            Self::Archive(url) => write!(f, "{url}"),
        }
    }
}

#[derive(Debug)]
pub struct UrlConversionError {
    source: helix_stdx::Url,
    kind: UrlConversionErrorKind,
}

#[derive(Debug)]
pub enum UrlConversionErrorKind {
    UnsupportedScheme,
    UnableToConvert,
}

impl fmt::Display for UrlConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            UrlConversionErrorKind::UnsupportedScheme => {
                write!(
                    f,
                    "unsupported scheme '{}' in URL {}",
                    self.source.scheme(),
                    self.source
                )
            }
            UrlConversionErrorKind::UnableToConvert => {
                write!(f, "unable to convert URL to file path: {}", self.source)
            }
        }
    }
}

impl std::error::Error for UrlConversionError {}

fn convert_url_to_uri(url: &helix_stdx::Url) -> Result<Uri, UrlConversionErrorKind> {
    if url.scheme() == "file" {
        url.to_file_path()
            .map(|path| Uri::File(helix_stdx::path::normalize(path).into()))
            .map_err(|_| UrlConversionErrorKind::UnableToConvert)
    } else if matches!(url.scheme(), "jar" | "jrt") {
        Ok(Uri::Archive(Arc::new(url.clone())))
    } else {
        Err(UrlConversionErrorKind::UnsupportedScheme)
    }
}

impl TryFrom<helix_stdx::Url> for Uri {
    type Error = UrlConversionError;

    fn try_from(url: helix_stdx::Url) -> Result<Self, Self::Error> {
        convert_url_to_uri(&url).map_err(|kind| Self::Error { source: url, kind })
    }
}

impl TryFrom<&helix_stdx::Url> for Uri {
    type Error = UrlConversionError;

    fn try_from(url: &helix_stdx::Url) -> Result<Self, Self::Error> {
        convert_url_to_uri(url).map_err(|kind| Self::Error {
            source: url.clone(),
            kind,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use helix_stdx::Url;

    #[test]
    fn archive_identity_is_not_a_filesystem_path() {
        for source in [
            "jar:file:///project%20one/library.jar!/example/Outer$Inner.java",
            "jrt:/java.base/java/lang/String.java",
        ] {
            let url = Url::parse(source).unwrap();
            let uri = Uri::try_from(&url).unwrap();
            assert!(uri.as_path().is_none());
            assert_eq!(uri.to_url().unwrap(), url);
            assert_eq!(uri.to_string(), source);
        }
    }

    #[test]
    fn unknown_scheme() {
        let url = Url::parse("csharp:/metadata/foo/bar/Baz.cs").unwrap();
        assert!(matches!(
            Uri::try_from(url),
            Err(UrlConversionError {
                kind: UrlConversionErrorKind::UnsupportedScheme,
                ..
            })
        ));
    }
}
