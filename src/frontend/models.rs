//! Read-only presentation input, independent of Webex and bridge protocol types.

use color_eyre::{eyre::eyre, Result};
use log::warn;

/// Available body forms. Keep legacy display/copy preferences until the bridge
/// contract can faithfully project the source format; never fetch HTML resources.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MessageBody<'a> {
    pub(crate) text: Option<&'a str>,
    pub(crate) markdown: Option<&'a str>,
    pub(crate) html: Option<&'a str>,
}

/// The preferred form for display. Conversion/wrapping remains renderer-owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DisplayBody<'a> {
    Html(&'a str),
    Markdown(&'a str),
    Text(&'a str),
    Missing,
}

impl<'a> MessageBody<'a> {
    /// Preserve current display precedence: HTML, then Markdown, then text.
    pub(crate) fn display(&self) -> DisplayBody<'a> {
        match (self.html, self.markdown, self.text) {
            (Some(html), _, _) => DisplayBody::Html(html),
            (_, Some(markdown), _) => DisplayBody::Markdown(markdown),
            (_, _, Some(text)) => DisplayBody::Text(text),
            _ => DisplayBody::Missing,
        }
    }

    /// Preserve current clipboard precedence: text, then Markdown, then HTML
    /// converted locally. Presence matters: an empty text field is still text.
    pub(crate) fn copy_text(&self) -> Result<String> {
        match (self.text, self.markdown, self.html) {
            (Some(text), _, _) => Ok(text.to_owned()),
            (None, Some(markdown), _) => Ok(markdown.to_owned()),
            (None, None, Some(html)) => html2text::from_read(html.as_bytes(), usize::MAX)
                .map_err(|error| {
                    warn!("Failed to convert HTML to plain text: {error}");
                    eyre!("Failed to convert HTML to plain text: {error}")
                })
                .or_else(|_| {
                    warn!("HTML conversion failed, using raw HTML as fallback");
                    Ok(html.to_owned())
                }),
            _ => Err(eyre!("Message has no content")),
        }
    }
}

/// Only the fields a message row needs, borrowed from its presentation cache.
/// Attachment URLs/content are deliberately not part of this view.
#[derive(Clone, Copy, Default)]
pub(crate) struct MessagePresentation<'a> {
    pub(crate) id: Option<&'a str>,
    pub(crate) parent_id: Option<&'a str>,
    pub(crate) sender_id: Option<&'a str>,
    pub(crate) sender_email: Option<&'a str>,
    pub(crate) created_at: Option<&'a str>,
    pub(crate) updated_at: Option<&'a str>,
    pub(crate) body: MessageBody<'a>,
    /// `None` preserves unknown/absent metadata, distinct from a known zero.
    pub(crate) attachment_count: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_prioritizes_text() {
        let body = MessageBody {
            text: Some("Plain text content"),
            markdown: Some("**Markdown** content"),
            html: Some("<p>HTML content</p>"),
        };
        assert_eq!(body.copy_text().unwrap(), "Plain text content");
    }

    #[test]
    fn copy_falls_back_to_markdown() {
        let body = MessageBody {
            markdown: Some("**Markdown** content"),
            html: Some("<p>HTML content</p>"),
            ..Default::default()
        };
        assert_eq!(body.copy_text().unwrap(), "**Markdown** content");
    }

    #[test]
    fn copy_converts_html_locally() {
        let body = MessageBody {
            html: Some("<p>Hello <strong>world</strong>!</p>"),
            ..Default::default()
        };
        let text = body.copy_text().unwrap();
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("<p>"));
        assert!(!text.contains("<strong>"));
    }

    #[test]
    fn copy_handles_malformed_html() {
        let body = MessageBody {
            html: Some("<p>Malformed HTML <strong>without closing"),
            ..Default::default()
        };
        assert!(!body.copy_text().unwrap().is_empty());
    }

    #[test]
    fn copy_fails_without_content() {
        assert!(MessageBody::default()
            .copy_text()
            .unwrap_err()
            .to_string()
            .contains("Message has no content"));
    }

    #[test]
    fn copy_preserves_empty_text() {
        let body = MessageBody {
            text: Some(""),
            markdown: Some("Markdown content"),
            ..Default::default()
        };
        assert_eq!(body.copy_text().unwrap(), "");
    }

    #[test]
    fn copy_handles_html_whitespace() {
        let body = MessageBody {
            html: Some("  <p>  Content with spaces  </p>  "),
            ..Default::default()
        };
        assert!(body.copy_text().unwrap().contains("Content with spaces"));
    }

    #[test]
    fn display_prioritizes_html_independently_of_copy() {
        let body = MessageBody {
            text: Some("text"),
            markdown: Some("markdown"),
            html: Some("html"),
        };
        assert_eq!(body.display(), DisplayBody::Html("html"));
        assert_eq!(body.copy_text().unwrap(), "text");
    }

    #[test]
    fn display_falls_back_through_markdown_text_and_missing() {
        let mut body = MessageBody {
            text: Some("text"),
            markdown: Some("markdown"),
            ..Default::default()
        };
        assert_eq!(body.display(), DisplayBody::Markdown("markdown"));
        body.markdown = None;
        assert_eq!(body.display(), DisplayBody::Text("text"));
        body.text = None;
        assert_eq!(body.display(), DisplayBody::Missing);
    }

    #[test]
    fn display_preserves_empty_preferred_field() {
        let body = MessageBody {
            html: Some(""),
            markdown: Some("markdown"),
            ..Default::default()
        };
        assert_eq!(body.display(), DisplayBody::Html(""));
    }
}
