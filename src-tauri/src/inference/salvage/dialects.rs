//! The dialects — one per vendor way of writing a tool call in prose.
//!
//! A model that means to call a tool but writes its markup into the content
//! stream has not made a *provider* mistake, so no provider adapter is the
//! right place to fix it. Each dialect here recognises one such shape and
//! turns it back into a real `ToolCall`. Adding the next format a model
//! invents is one struct plus one line in `DIALECTS`.
//!
//! Every dialect is deliberately conservative: a block whose tool name is not
//! one the request actually declared is `NotMine`, and released as the prose
//! it probably was.

use serde_json::{Map, Value};

use crate::inference::ToolCall;

/// What a request declared, so a dialect can tell a real call from prose that
/// merely looks like one.
pub(crate) struct DialectContext<'a> {
    pub(crate) tools: &'a [String],
    /// No text has been emitted yet this turn. Only the loosest dialect
    /// (bare JSON) cares — it will not fire mid-sentence.
    pub(crate) at_turn_start: bool,
}

impl DialectContext<'_> {
    fn declares(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool == name)
    }
}

pub(crate) enum DialectOutcome {
    /// Opener matched by luck; this is prose. Release it.
    NotMine,
    /// Opener matched and the block has not closed yet. More text may finish
    /// it, so the caller keeps buffering.
    Incomplete,
    /// Parsed. `consumed` counts bytes from the start of the slice handed in.
    Calls { calls: Vec<ToolCall>, consumed: usize },
}

pub(crate) trait ToolDialect: Send + Sync {
    /// Stable label, used in the radar log when a sibling dialect misses.
    fn id(&self) -> &'static str;

    /// Literal strings that can begin a block. Kept separate from `parse` so
    /// the scanner can find candidates cheaply and, just as importantly,
    /// recognise a *partial* opener parked at the end of the buffer.
    fn openers(&self) -> &'static [&'static str];

    /// `text` begins at one of `openers`.
    fn parse(&self, text: &str, ctx: &DialectContext) -> DialectOutcome;
}

pub(crate) static DIALECTS: &[&dyn ToolDialect] =
    &[&DeepSeekDsml, &HermesToolCall, &BareJsonCall];

/// A salvaged call has no provider-issued id — the provider never saw it as a
/// call at all. Minting is therefore correct here, unlike the s389 bug where
/// a real id was thrown away and replaced.
fn mint_id() -> String {
    format!("call_{}", uuid::Uuid::new_v4().simple())
}

/// Reads `key="value"` out of an opening tag.
fn attribute(tag: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

/// A parameter body is text. Anything that also happens to be valid JSON
/// (a number, a bool, an object) is worth more to the tool as that value —
/// unless the tag itself said to keep it a string.
fn json_value(raw: &str, forced_string: bool) -> Value {
    if !forced_string {
        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            return value;
        }
    }
    Value::String(raw.to_owned())
}

// ---------------------------------------------------------------------------
// DeepSeek — `<｜DSML｜invoke name="…">` with `<｜DSML｜parameter …>` children.
// ---------------------------------------------------------------------------

/// The delimiter is DeepSeek's FULLWIDTH VERTICAL LINE (U+FF5C), not ASCII
/// `|`. Both are accepted: the model is not reliably consistent about which
/// it emits, and a mismatch here reads as "the fix did nothing".
const PIPES: [&str; 2] = ["\u{ff5c}", "|"];

struct DeepSeekDsml;

impl DeepSeekDsml {
    /// Which pipe width this particular block opened with. A block does not
    /// mix them, so detecting once keeps every later tag byte-offset exact —
    /// normalising the text instead would shift `consumed` by 2 bytes a pipe.
    fn pipe(text: &str) -> Option<&'static str> {
        PIPES
            .into_iter()
            .find(|pipe| text.starts_with(&format!("<{pipe}DSML{pipe}")))
    }

    fn invokes(body: &str, pipe: &str, ctx: &DialectContext) -> Vec<ToolCall> {
        let open = format!("<{pipe}DSML{pipe}invoke");
        let close = format!("</{pipe}DSML{pipe}invoke>");
        let mut calls = Vec::new();
        let mut cursor = 0usize;

        while let Some(offset) = body[cursor..].find(&open) {
            let start = cursor + offset;
            let Some(tag_end) = body[start..].find('>').map(|end| start + end + 1) else {
                break;
            };
            let (inner, next) = match body[tag_end..].find(&close) {
                Some(end) => (&body[tag_end..tag_end + end], tag_end + end + close.len()),
                None => (&body[tag_end..], body.len()),
            };
            cursor = next;

            let Some(name) = attribute(&body[start..tag_end], "name") else {
                continue;
            };
            if !ctx.declares(&name) {
                continue;
            }
            calls.push(ToolCall {
                id: mint_id(),
                name,
                arguments: Self::parameters(inner, pipe),
            });
        }
        calls
    }

    fn parameters(inner: &str, pipe: &str) -> String {
        let open = format!("<{pipe}DSML{pipe}parameter");
        let close = format!("</{pipe}DSML{pipe}parameter>");
        let mut arguments = Map::new();
        let mut cursor = 0usize;

        while let Some(offset) = inner[cursor..].find(&open) {
            let start = cursor + offset;
            let Some(tag_end) = inner[start..].find('>').map(|end| start + end + 1) else {
                break;
            };
            let (raw, next) = match inner[tag_end..].find(&close) {
                Some(end) => (&inner[tag_end..tag_end + end], tag_end + end + close.len()),
                None => (&inner[tag_end..], inner.len()),
            };
            cursor = next;

            let tag = &inner[start..tag_end];
            let Some(name) = attribute(tag, "name") else {
                continue;
            };
            let forced_string = attribute(tag, "string").as_deref() == Some("true");
            arguments.insert(name, json_value(raw.trim(), forced_string));
        }
        Value::Object(arguments).to_string()
    }
}

impl ToolDialect for DeepSeekDsml {
    fn id(&self) -> &'static str {
        "deepseek_dsml"
    }

    fn openers(&self) -> &'static [&'static str] {
        &[
            "<\u{ff5c}DSML\u{ff5c}tool_calls>",
            "<\u{ff5c}DSML\u{ff5c}invoke",
            "<|DSML|tool_calls>",
            "<|DSML|invoke",
        ]
    }

    fn parse(&self, text: &str, ctx: &DialectContext) -> DialectOutcome {
        let Some(pipe) = Self::pipe(text) else {
            return DialectOutcome::NotMine;
        };
        let open_calls = format!("<{pipe}DSML{pipe}tool_calls>");
        let close_calls = format!("</{pipe}DSML{pipe}tool_calls>");
        let open_invoke = format!("<{pipe}DSML{pipe}invoke");
        let close_invoke = format!("</{pipe}DSML{pipe}invoke>");

        // Either the wrapper or a lone invoke; the wrapper is the common shape
        // but a model that skips it still means the same thing.
        let (body, consumed) = if let Some(rest) = text.strip_prefix(&open_calls) {
            match rest.find(&close_calls) {
                Some(end) => (
                    &rest[..end],
                    open_calls.len() + end + close_calls.len(),
                ),
                None => return DialectOutcome::Incomplete,
            }
        } else if text.starts_with(&open_invoke) {
            match text.find(&close_invoke) {
                Some(end) => {
                    let consumed = end + close_invoke.len();
                    (&text[..consumed], consumed)
                }
                None => return DialectOutcome::Incomplete,
            }
        } else {
            return DialectOutcome::NotMine;
        };

        let calls = Self::invokes(body, pipe, ctx);
        if calls.is_empty() {
            return DialectOutcome::NotMine;
        }
        DialectOutcome::Calls { calls, consumed }
    }
}

// ---------------------------------------------------------------------------
// Hermes / Qwen / most local models — `<tool_call>{json}</tool_call>`.
// ---------------------------------------------------------------------------

struct HermesToolCall;

impl ToolDialect for HermesToolCall {
    fn id(&self) -> &'static str {
        "hermes_toolcall"
    }

    fn openers(&self) -> &'static [&'static str] {
        &["<tool_call>"]
    }

    fn parse(&self, text: &str, ctx: &DialectContext) -> DialectOutcome {
        const OPEN: &str = "<tool_call>";
        const CLOSE: &str = "</tool_call>";

        let Some(rest) = text.strip_prefix(OPEN) else {
            return DialectOutcome::NotMine;
        };
        let Some(end) = rest.find(CLOSE) else {
            return DialectOutcome::Incomplete;
        };
        let consumed = OPEN.len() + end + CLOSE.len();

        let Ok(value) = serde_json::from_str::<Value>(rest[..end].trim()) else {
            return DialectOutcome::NotMine;
        };
        match call_from_json(&value, ctx) {
            Some(call) => DialectOutcome::Calls {
                calls: vec![call],
                consumed,
            },
            None => DialectOutcome::NotMine,
        }
    }
}

// ---------------------------------------------------------------------------
// Bare JSON — the loosest shape, and the only one that can eat real prose.
// ---------------------------------------------------------------------------

/// Llama- and Mistral-family models sometimes answer with nothing but the call
/// object. `{` is far too common an opener to trust on its own, so this fires
/// only at the very start of a turn and only when the name is a declared tool.
struct BareJsonCall;

impl ToolDialect for BareJsonCall {
    fn id(&self) -> &'static str {
        "bare_json_call"
    }

    fn openers(&self) -> &'static [&'static str] {
        &["{"]
    }

    fn parse(&self, text: &str, ctx: &DialectContext) -> DialectOutcome {
        if !ctx.at_turn_start {
            return DialectOutcome::NotMine;
        }
        let mut stream = serde_json::Deserializer::from_str(text).into_iter::<Value>();
        let value = match stream.next() {
            Some(Ok(value)) => value,
            // A JSON object still arriving is worth waiting for; anything else
            // is prose that merely began with a brace.
            Some(Err(error)) if error.is_eof() => return DialectOutcome::Incomplete,
            _ => return DialectOutcome::NotMine,
        };
        let consumed = stream.byte_offset();
        match call_from_json(&value, ctx) {
            Some(call) => DialectOutcome::Calls {
                calls: vec![call],
                consumed,
            },
            None => DialectOutcome::NotMine,
        }
    }
}

/// The `{"name": …, "arguments": {…}}` object both JSON dialects land on.
/// `parameters` is accepted as an alias — the split is per model family, not
/// per vendor, and guessing wrong costs a silent miss.
fn call_from_json(value: &Value, ctx: &DialectContext) -> Option<ToolCall> {
    let name = value.get("name")?.as_str()?;
    if !ctx.declares(name) {
        return None;
    }
    let arguments = value
        .get("arguments")
        .or_else(|| value.get("parameters"))
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    // A model that stringified its own arguments is still asking for the call.
    let arguments = match arguments {
        Value::String(raw) => raw,
        other => other.to_string(),
    };
    Some(ToolCall {
        id: mint_id(),
        name: name.to_owned(),
        arguments,
    })
}
