use scraper::{Html, Selector};

use crate::recipe::RecipeCard;

#[derive(Debug)]
pub struct ExtractedPage {
    pub title: String,
    pub body: String,
    pub recipe: Option<RecipeCard>,
}

pub fn extract(html: &str) -> ExtractedPage {
    let doc = Html::parse_document(html);
    let title = extract_title(&doc);
    let body = {
        let text = extract_body(&doc);
        if text.len() >= 200 {
            text
        } else {
            // Sparse visible text — page is probably JS-rendered. Pull content from
            // embedded JSON data ("shortDescription" is YouTube's pattern) or meta tags.
            extract_json_field(html, "shortDescription")
                .or_else(|| meta_description(&doc))
                .unwrap_or(text)
        }
    };
    ExtractedPage {
        title,
        body,
        recipe: None,
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
    for sel_str in &[
        "main",
        "article",
        "[role='main']",
        "#content",
        "#main",
        "body",
    ] {
        if let Ok(sel) = Selector::parse(sel_str)
            && let Some(el) = doc.select(&sel).next()
        {
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
    String::new()
}

fn meta_description(doc: &Html) -> Option<String> {
    for sel_str in &[
        r#"meta[property="og:description"]"#,
        r#"meta[name="description"]"#,
    ] {
        if let Ok(sel) = Selector::parse(sel_str)
            && let Some(el) = doc.select(&sel).next()
            && let Some(content) = el.value().attr("content")
        {
            let s = content.trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Extracts the value of a JSON string field from raw HTML (e.g. YouTube's `"shortDescription":"..."`).
/// Handles `\n`, `\t`, `\r`, `\\`, `\"`, and `\uXXXX` escapes.
fn extract_json_field(html: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\":\"", field);
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    let mut chars = rest.char_indices();
    let mut value = String::new();
    loop {
        let (_, ch) = chars.next()?;
        match ch {
            '\\' => match chars.next()?.1 {
                'n' => value.push('\n'),
                't' => value.push('\t'),
                'r' => value.push('\r'),
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                'u' => {
                    let hex: String = (0..4)
                        .filter_map(|_| chars.next())
                        .map(|(_, c)| c)
                        .collect();
                    if let Ok(n) = u32::from_str_radix(&hex, 16)
                        && let Some(c) = char::from_u32(n)
                    {
                        value.push(c);
                    }
                }
                other => {
                    value.push('\\');
                    value.push(other);
                }
            },
            '"' => break,
            other => value.push(other),
        }
    }
    if value.len() > 20 { Some(value) } else { None }
}

/// Detects login walls: redirected to a login URL or page contains a password field.
/// These are personal/private pages (email, Slack) — fallbacks can't help.
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

/// Detects paywalls: content exists but is gated behind a subscription.
/// Unlike auth walls, archived/cached copies may be available via fallbacks.
pub fn is_paywall(html: &str) -> bool {
    let lower = html.to_lowercase();
    // JSON-LD accessibility marker used by news sites
    if lower.contains("\"isaccessibleforfree\":\"false\"")
        || lower.contains("\"isaccessibleforfree\": \"false\"")
    {
        return true;
    }
    // Paywall overlay element class/id
    if [
        "class=\"paywall\"",
        "id=\"paywall\"",
        "class='paywall'",
        "id='paywall'",
        "data-paywall",
        "class=\"tp-",
        "id=\"tp-",
    ]
    .iter()
    .any(|p| lower.contains(p))
    {
        return true;
    }
    // Common subscribe-to-read phrases
    [
        "subscribe to read",
        "subscribe to continue",
        "subscribe to keep reading",
        "subscription required",
        "subscriber-only",
        "subscribers only",
        "already a subscriber?",
        "become a subscriber",
        "premium article",
        "premium content",
        "members only",
        "sign up to read",
        "register to read",
        "reading limit reached",
        "free articles remaining",
        "free article",
    ]
    .iter()
    .any(|p| lower.contains(p))
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
        let page =
            extract("<html><head><title>  Hello\n  World  </title></head><body>x</body></html>");
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
        assert!(!is_auth_wall(
            "https://example.com/about",
            "<p>No forms here</p>"
        ));
    }
}
