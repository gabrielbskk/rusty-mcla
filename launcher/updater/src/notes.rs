//! Release notes: GitHub markdown rendered locally to HTML that is safe to put into the page.
//!
//! Safety, in two layers: (1) the markdown parser's events are filtered before rendering: raw HTML becomes plain text, images are
//! dropped (only their alt text stays), so nothing remote is ever referenced; (2) the result goes through an allow-list sanitiser
//! (`ammonia`) that keeps only a few text tags and `http(s)` links. Links carry no `target`; the page opens them through the
//! launcher's `open_url` command, which accepts only http(s).

use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};
use std::collections::HashSet;

/// Only this much of a release body is rendered; the "Full notes on GitHub" link covers the rest.
pub const MAX_BODY_CHARS: usize = 20_000;

pub fn render(markdown: &str) -> String {
    let body: String = markdown.chars().take(MAX_BODY_CHARS).collect();
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut image_depth = 0usize;
    let events = Parser::new_ext(&body, opts).filter_map(|ev| match ev {
        Event::Html(t) | Event::InlineHtml(t) => Some(Event::Text(t)),
        Event::Start(Tag::Image { .. }) => {
            image_depth += 1;
            None
        }
        Event::End(TagEnd::Image) => {
            image_depth = image_depth.saturating_sub(1);
            None
        }
        other => Some(other),
    });
    let mut raw = String::new();
    html::push_html(&mut raw, events);
    sanitise(&raw)
}

fn sanitise(raw: &str) -> String {
    let tags: HashSet<&str> = ["h1", "h2", "h3", "h4", "h5", "h6", "p", "ul", "ol", "li", "strong", "em", "code", "pre", "blockquote", "a", "br", "hr", "del"].into_iter().collect();
    let schemes: HashSet<&str> = ["http", "https"].into_iter().collect();
    ammonia::Builder::empty()
        .tags(tags)
        .tag_attributes(std::iter::once(("a", ["href"].into_iter().collect::<HashSet<&str>>())).collect())
        .url_schemes(schemes)
        .link_rel(Some("noopener noreferrer"))
        .url_relative(ammonia::UrlRelative::Deny)
        .clean(raw)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_lists_and_links_render() {
        let h = render("## Added\n- one **bold** thing\n- two\n\nSee [the notes](https://github.com/x/y).");
        assert!(h.contains("<h2>Added</h2>"));
        assert!(h.contains("<li>one <strong>bold</strong> thing</li>"));
        assert!(h.contains("href=\"https://github.com/x/y\""));
        assert!(h.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn scripts_and_raw_html_never_survive() {
        let h = render("<script>alert(1)</script>\n\nhello <img src=x onerror=alert(1)> <b onclick=\"x()\">b</b>\n\n<iframe src=\"https://evil\"></iframe>");
        assert!(!h.contains("<script"));
        assert!(!h.contains("<img"));
        assert!(!h.contains("<iframe"));
        assert!(!h.contains("onerror=") || h.contains("&lt;"));
        assert!(!h.contains("<b "));
        assert!(h.contains("hello"));
    }

    #[test]
    fn javascript_and_data_links_are_removed() {
        let h = render("[x](javascript:alert(1)) [y](data:text/html;base64,AAAA) [z](vbscript:x) [ok](http://example.com/a)");
        assert!(!h.contains("javascript:"));
        assert!(!h.contains("data:"));
        assert!(!h.contains("vbscript:"));
        assert!(h.contains("http://example.com/a"));
    }

    #[test]
    fn relative_and_mailto_links_are_removed() {
        let h = render("[a](/etc/passwd) [b](mailto:x@y.z) [c](file:///C:/x) [d](//evil.example/x)");
        assert!(!h.contains("href=\"/etc"));
        assert!(!h.contains("mailto:"));
        assert!(!h.contains("file:"));
        assert!(!h.contains("//evil.example"));
    }

    #[test]
    fn images_are_dropped_and_nothing_remote_is_referenced() {
        let h = render("![logo](https://evil.example/pixel.png) after");
        assert!(!h.contains("<img"));
        assert!(!h.contains("pixel.png"));
        assert!(h.contains("after"));
    }

    #[test]
    fn long_bodies_are_cut() {
        let long = "word ".repeat(10_000);
        let h = render(&long);
        assert!(h.len() < MAX_BODY_CHARS * 2);
    }
}
