use scraper::{Html, Selector};

#[derive(Debug)]
pub struct ExtractedPage {
    pub title: String,
    pub body: String,
}

pub fn extract(html: &str) -> ExtractedPage {
    let doc = Html::parse_document(html);
    ExtractedPage {
        title: extract_title(&doc),
        body: extract_body(&doc),
    }
}

fn extract_title(doc: &Html) -> String {
    let sel = Selector::parse("title").unwrap();
    doc.select(&sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_body(doc: &Html) -> String {
    // Prefer semantic content containers; fall back to full body.
    for sel_str in &["main", "article", "[role='main']", "#content", "#main", "body"] {
        if let Ok(sel) = Selector::parse(sel_str) {
            if let Some(el) = doc.select(&sel).next() {
                let text = el
                    .text()
                    .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.len() > 50 {
                    return text;
                }
            }
        }
    }
    String::new()
}

/// Detects login walls by final URL path and presence of a password input.
pub fn is_auth_wall(final_url: &str, html: &str) -> bool {
    let url_lower = final_url.to_lowercase();
    if ["/login", "/signin", "/sign-in", "/auth/", "/authenticate"]
        .iter()
        .any(|p| url_lower.contains(p))
    {
        return true;
    }
    let html_lower = html.to_lowercase();
    html_lower.contains("type=\"password\"") || html_lower.contains("type='password'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title() {
        let page = extract("<html><head><title>Hello World</title></head><body>x</body></html>");
        assert_eq!(page.title, "Hello World");
    }

    #[test]
    fn normalises_whitespace_in_title() {
        let page = extract("<html><head><title>  Hello\n  World  </title></head><body>x</body></html>");
        assert_eq!(page.title, "Hello World");
    }

    #[test]
    fn prefers_main_over_body() {
        let page = extract(
            "<html><body><nav>Nav junk</nav>\
             <main>Main content about the Rust programming language and its features</main>\
             </body></html>",
        );
        assert!(
            page.body.contains("Rust programming language"),
            "got: {}",
            page.body
        );
        assert!(!page.body.contains("Nav junk"), "nav should be excluded");
    }

    #[test]
    fn falls_back_to_body() {
        let page = extract(
            "<html><body><p>Some body text here and more words that go well past fifty characters</p></body></html>",
        );
        assert!(page.body.contains("body text"));
    }

    #[test]
    fn auth_wall_detected_by_login_url() {
        assert!(is_auth_wall("https://example.com/login", ""));
        assert!(is_auth_wall("https://example.com/signin", ""));
        assert!(!is_auth_wall("https://example.com/about", ""));
    }

    #[test]
    fn auth_wall_detected_by_password_field() {
        assert!(is_auth_wall(
            "https://example.com/secure",
            r#"<form><input type="password" name="pw"></form>"#
        ));
        assert!(!is_auth_wall("https://example.com/about", "<p>No forms here</p>"));
    }
}
