// The web seam — Companion's eyes on the world outside, ported from the
// proven Heimdall organ shape. Two lanes: search via SerpApi (serpapi.com —
// NOT serper.dev, a rival with a near-identical name), answer_box first when
// Google volunteers one; and fetch, a plain HTTP read with an HTML-to-text
// walker (article/main scoping, headings kept, script/style stripped). A
// JS-shell page that yields no text is reported honestly — the browser
// escalation lane (CDP) is deliberately not ported. Search's ground is the
// SERPAPI_API_KEY; fetch's ground is the network itself.

use serde::Deserialize;

const SEARCH_ENDPOINT: &str = "https://serpapi.com/search.json";
/// Fetched bodies larger than this are cut before extraction — a page this
/// size is a download, not an article.
const FETCH_BODY_CAP_BYTES: usize = 2_000_000;
/// A page with this little text but plenty of markup is a JS shell — there
/// is nothing to read without a browser.
const HOLLOW_TEXT_CHARS: usize = 200;
const HOLLOW_HTML_BYTES: usize = 5_000;

/// One search hit, already flattened to what the model needs.
#[derive(Debug, PartialEq)]
pub(crate) struct WebHit {
    pub(crate) title: String,
    pub(crate) link: String,
    pub(crate) snippet: String,
}

/// A full search harvest: Google's direct answer when one exists, plus the
/// ranked organic results.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct WebSearch {
    pub(crate) answer: Option<String>,
    pub(crate) hits: Vec<WebHit>,
}

#[derive(Deserialize)]
struct SerpResponse {
    error: Option<String>,
    answer_box: Option<AnswerBox>,
    #[serde(default)]
    organic_results: Vec<OrganicResult>,
}

#[derive(Deserialize)]
struct AnswerBox {
    title: Option<String>,
    answer: Option<String>,
    snippet: Option<String>,
    result: Option<String>,
}

#[derive(Deserialize)]
struct OrganicResult {
    title: Option<String>,
    link: Option<String>,
    snippet: Option<String>,
}

pub(crate) async fn search(query: &str, limit: u32, api_key: &str) -> Result<WebSearch, String> {
    let response = reqwest::Client::new()
        .get(SEARCH_ENDPOINT)
        .query(&[
            ("engine", "google"),
            ("q", query),
            ("num", &limit.to_string()),
            ("api_key", api_key),
        ])
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("the web could not be reached: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("web search failed: HTTP {}", status.as_u16()));
    }
    let parsed: SerpResponse = response
        .json()
        .await
        .map_err(|error| format!("the search results could not be read: {error}"))?;
    if let Some(error) = parsed.error {
        return Err(format!("the search engine refused: {error}"));
    }
    Ok(harvest(parsed, limit))
}

fn harvest(parsed: SerpResponse, limit: u32) -> WebSearch {
    let answer = parsed.answer_box.and_then(|answer_box| {
        let text = answer_box
            .answer
            .or(answer_box.snippet)
            .or(answer_box.result)?;
        Some(match answer_box.title {
            Some(title) if !title.is_empty() => format!("{title}: {text}"),
            _ => text,
        })
    });
    let hits = parsed
        .organic_results
        .into_iter()
        .filter_map(|result| {
            Some(WebHit {
                title: result.title?,
                link: result.link?,
                snippet: result.snippet.unwrap_or_default(),
            })
        })
        .take(limit as usize)
        .collect();
    WebSearch { answer, hits }
}

/// One fetched page, already reduced to readable text.
#[derive(Debug, PartialEq)]
pub(crate) struct WebPage {
    pub(crate) title: Option<String>,
    /// Extracted text — empty means the page is a JS shell with nothing
    /// readable in its HTML.
    pub(crate) text: String,
}

pub(crate) async fn fetch(url: &str) -> Result<WebPage, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("only http(s) URLs can be fetched".to_owned());
    }
    let response = reqwest::Client::new()
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64) SemantixCompanion/0.1",
        )
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("the page could not be reached: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("the page refused: HTTP {}", status.as_u16()));
    }
    let html_ish = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.contains("html"))
        // No header → assume HTML; the extractor is harmless on plain text.
        .unwrap_or(true);
    let mut body = response
        .text()
        .await
        .map_err(|error| format!("the page body could not be read: {error}"))?;
    if body.len() > FETCH_BODY_CAP_BYTES {
        let mut cut = FETCH_BODY_CAP_BYTES;
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        body.truncate(cut);
    }
    if !html_ish {
        return Ok(WebPage {
            title: None,
            text: body.trim().to_owned(),
        });
    }
    let page = extract_page(&body);
    if page.text.len() < HOLLOW_TEXT_CHARS && body.len() > HOLLOW_HTML_BYTES {
        return Ok(WebPage {
            title: page.title,
            text: String::new(),
        });
    }
    Ok(page)
}

/// HTML → readable text: <title> captured, content scoped to <article> or
/// <main> when the page marks one, script/style/head/nav-like subtrees
/// dropped whole, headings and list items kept as markdown, everything else
/// flattened to lines.
fn extract_page(html: &str) -> WebPage {
    let title = tag_content(html, "title").map(|raw| decode_entities(&raw).trim().to_owned());
    let scope = scoped_body(html);
    WebPage {
        title,
        text: walk_text(scope),
    }
}

/// The reading scope: the first <article>, else the first <main>, else the
/// whole document.
fn scoped_body(html: &str) -> &str {
    for container in ["article", "main"] {
        if let Some(inner) = tag_span(html, container) {
            return inner;
        }
    }
    html
}

/// Case-insensitive position of `needle` (ASCII) in `haystack`.
fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&start| {
        haystack[start..start + needle.len()].eq_ignore_ascii_case(needle)
    })
}

/// The inner span of the FIRST `<name ...>...</name>` element, unnested —
/// fine for the containers it is used on (title/article/main appear once).
fn tag_span<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    let open_at = find_ci(html, &format!("<{name}"), 0)?;
    let open_end = html[open_at..].find('>').map(|offset| open_at + offset + 1)?;
    // "<article" must not match "<articles..." — the next char ends the name.
    match html.as_bytes().get(open_at + 1 + name.len()) {
        Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'/') => {}
        _ => return None,
    }
    let close_at = find_ci(html, &format!("</{name}"), open_end)?;
    Some(&html[open_end..close_at])
}

fn tag_content(html: &str, name: &str) -> Option<String> {
    tag_span(html, name).map(str::to_owned)
}

/// Subtrees dropped whole — nothing inside them is prose.
const DROP_SUBTREES: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "head",
];

/// Tags that end a line of text when they open or close.
const BLOCK_TAGS: &[&str] = &[
    "p", "div", "section", "article", "main", "header", "footer", "nav",
    "aside", "ul", "ol", "table", "tr", "blockquote", "figure", "figcaption",
    "form", "br", "hr",
];

fn walk_text(html: &str) -> String {
    let mut out = String::new();
    let bytes = html.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'<' {
            let text_end = html[at..]
                .find('<')
                .map(|offset| at + offset)
                .unwrap_or(bytes.len());
            let text = decode_entities(&html[at..text_end]);
            push_inline(&mut out, &text);
            at = text_end;
            continue;
        }
        if html[at..].starts_with("<!--") {
            at = find_ci(html, "-->", at)
                .map(|found| found + 3)
                .unwrap_or(bytes.len());
            continue;
        }
        // Read the tag, honouring quotes so a '>' inside an attribute
        // (onclick="a>b", SVG paths) does not end it early.
        let mut cursor = at + 1;
        let mut quote: Option<u8> = None;
        while cursor < bytes.len() {
            match (quote, bytes[cursor]) {
                (None, b'>') => break,
                (None, b'"') | (None, b'\'') => quote = Some(bytes[cursor]),
                (Some(open), close) if open == close => quote = None,
                _ => {}
            }
            cursor += 1;
        }
        let tag = &html[at + 1..cursor.min(bytes.len())];
        at = (cursor + 1).min(bytes.len());
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        if !closing && DROP_SUBTREES.contains(&name.as_str()) {
            at = find_ci(html, &format!("</{name}"), at)
                .and_then(|found| html[found..].find('>').map(|offset| found + offset + 1))
                .unwrap_or(bytes.len());
            continue;
        }
        if let Some(level) = name.strip_prefix('h').and_then(|n| n.parse::<usize>().ok()) {
            if (1..=6).contains(&level) && !closing {
                end_line(&mut out);
                out.push_str(&"#".repeat(level));
                out.push(' ');
            } else if closing {
                end_line(&mut out);
            }
            continue;
        }
        match name.as_str() {
            "li" if !closing => {
                end_line(&mut out);
                out.push_str("- ");
            }
            "td" | "th" if !closing => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push(' ');
                }
            }
            "pre" => end_line(&mut out),
            _ if BLOCK_TAGS.contains(&name.as_str()) => end_line(&mut out),
            _ => {}
        }
    }
    // Collapse runs of blank lines and trailing space the walk left behind.
    let mut lines: Vec<&str> = Vec::new();
    let mut blank_run = 0;
    for line in out.lines().map(str::trim_end) {
        blank_run = if line.is_empty() { blank_run + 1 } else { 0 };
        if blank_run <= 1 {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_owned()
}

/// Inline text lands with its whitespace collapsed; a fresh line never
/// starts with spaces.
fn push_inline(out: &mut String, text: &str) {
    let collapsed_leading = text.starts_with(char::is_whitespace);
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return;
    }
    if collapsed_leading && !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
        out.push(' ');
    }
    out.push_str(&words.join(" "));
    if text.ends_with(char::is_whitespace) {
        out.push(' ');
    }
}

fn end_line(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// The handful of entities that actually occur in prose, plus numeric forms.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let semi = match rest[..rest.len().min(12)].find(';') {
            Some(index) => index,
            None => {
                out.push('&');
                rest = &rest[1..];
                continue;
            }
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::{decode_entities, extract_page, harvest, SerpResponse, WebHit};

    fn parse(json: &str) -> SerpResponse {
        serde_json::from_str(json).expect("test payload should parse")
    }

    #[test]
    fn a_harvest_takes_the_answer_box_and_ranked_results() {
        let harvested = harvest(
            parse(
                r#"{
                    "answer_box": {"title": "Rust release", "answer": "1.89"},
                    "organic_results": [
                        {"title": "Rust Blog", "link": "https://blog.rust-lang.org", "snippet": "Announcing Rust 1.89"},
                        {"title": "No link, dropped"},
                        {"title": "Docs", "link": "https://doc.rust-lang.org"}
                    ]
                }"#,
            ),
            10,
        );
        assert_eq!(harvested.answer.as_deref(), Some("Rust release: 1.89"));
        assert_eq!(
            harvested.hits,
            vec![
                WebHit {
                    title: "Rust Blog".to_owned(),
                    link: "https://blog.rust-lang.org".to_owned(),
                    snippet: "Announcing Rust 1.89".to_owned(),
                },
                WebHit {
                    title: "Docs".to_owned(),
                    link: "https://doc.rust-lang.org".to_owned(),
                    snippet: String::new(),
                },
            ]
        );
    }

    #[test]
    fn a_harvest_respects_the_limit_and_survives_an_empty_page() {
        let harvested = harvest(
            parse(
                r#"{"organic_results": [
                    {"title": "a", "link": "https://a", "snippet": ""},
                    {"title": "b", "link": "https://b", "snippet": ""}
                ]}"#,
            ),
            1,
        );
        assert_eq!(harvested.answer, None);
        assert_eq!(harvested.hits.len(), 1);

        let empty = harvest(parse("{}"), 5);
        assert_eq!(empty, super::WebSearch::default());
    }

    #[test]
    fn extraction_scopes_to_the_article_and_keeps_structure() {
        let page = extract_page(
            r#"<html><head><title>Rust 1.89 &amp; friends</title>
               <style>body { color: red }</style></head>
               <body><nav>Home | About | Contact</nav>
               <article><h1>Announcing Rust</h1>
               <p>The team is <b>happy</b> to announce.</p>
               <script>trackEverything("<article>");</script>
               <ul><li>faster builds</li><li>better errors</li></ul>
               </article>
               <footer>All the site chrome lives out here.</footer></body></html>"#,
        );
        assert_eq!(page.title.as_deref(), Some("Rust 1.89 & friends"));
        assert_eq!(
            page.text,
            "# Announcing Rust\nThe team is happy to announce.\n- faster builds\n- better errors"
        );
    }

    #[test]
    fn extraction_survives_quoted_brackets_and_falls_back_to_body() {
        let page = extract_page(
            r#"<body><div onclick="if (a > b) go('<x>')">visible words</div></body>"#,
        );
        assert_eq!(page.title, None);
        assert_eq!(page.text, "visible words");
    }

    #[test]
    fn entities_decode_including_numeric_forms() {
        assert_eq!(
            decode_entities("a &amp; b &lt;tag&gt; &#8212; caf&#xe9; &unknown; loose & end"),
            "a & b <tag> — café &unknown; loose & end"
        );
    }
}
