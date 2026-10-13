#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTML parse error: {0}")]
    HtmlParse(String),

    #[error("Layout error: {0}")]
    Layout(String),

    #[error("PDF generation error: {0}")]
    PdfGeneration(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Asset error: {0}")]
    Asset(String),

    #[error("Template error: {0}")]
    Template(String),

    #[error("WOFF decode error: {0}")]
    WoffDecode(String),

    #[error("Unsupported font format: {0}")]
    UnsupportedFontFormat(String),

    #[error("{0}")]
    Other(String),
}

impl From<minijinja::Error> for Error {
    fn from(e: minijinja::Error) -> Self {
        Error::Template(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_error_display() {
        let err = Error::Template("syntax error at line 3".into());
        assert!(err.to_string().contains("syntax error at line 3"));
    }

    #[test]
    fn html_parse_error_display() {
        let err = Error::HtmlParse("unexpected end of file".into());
        assert!(err.to_string().contains("unexpected end of file"));
        assert!(err.to_string().contains("HTML parse error"));
    }

    #[test]
    fn layout_error_display() {
        let err = Error::Layout("grid overflow".into());
        assert!(err.to_string().contains("grid overflow"));
        assert!(err.to_string().contains("Layout error"));
    }

    #[test]
    fn pdf_generation_error_display() {
        let err = Error::PdfGeneration("font embed failed".into());
        assert!(err.to_string().contains("font embed failed"));
        assert!(err.to_string().contains("PDF generation error"));
    }

    #[test]
    fn asset_error_display() {
        let err = Error::Asset("image not found: logo.png".into());
        assert!(err.to_string().contains("image not found: logo.png"));
        assert!(err.to_string().contains("Asset error"));
    }

    #[test]
    fn woff_decode_error_display() {
        let err = Error::WoffDecode("bad signature".into());
        assert!(err.to_string().contains("bad signature"));
        assert!(err.to_string().contains("WOFF decode error"));
    }

    #[test]
    fn unsupported_font_format_error_display() {
        let err = Error::UnsupportedFontFormat(".eot".into());
        assert!(err.to_string().contains(".eot"));
        assert!(err.to_string().contains("Unsupported font format"));
    }

    #[test]
    fn other_error_display() {
        let err = Error::Other("something went wrong".into());
        assert_eq!(err.to_string(), "something went wrong");
    }

    #[test]
    fn io_error_from_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn from_minijinja_error_produces_template_variant() {
        // From<minijinja::Error> must wrap the error message in Error::Template.
        let minijinja_err =
            minijinja::Error::new(minijinja::ErrorKind::UndefinedError, "no such variable");
        let err: Error = minijinja_err.into();
        assert!(
            matches!(err, Error::Template(_)),
            "expected Error::Template, got {err:?}"
        );
        assert!(!err.to_string().is_empty());
    }
}
