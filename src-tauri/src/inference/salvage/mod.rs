//! Rescues tool calls a model wrote as prose.
//!
//! Some models emit their tool-call markup into the content stream as literal
//! text instead of as structured calls — DeepSeek V4 did it ten times here
//! between 2026-08-27 and 2026-09-06, answering nothing and leaving the markup
//! behind as the assistant's whole turn.
//!
//! This wraps the sink at the gateway rather than living inside one provider,
//! because the fault is the model's, not the wire format's: any provider that
//! ever carries such a model inherits the rescue for free, including ones not
//! written yet.
//!
//! Prose pays nothing for it. Text ahead of a candidate opener is forwarded
//! immediately; only the bytes that might be markup are ever held back.

mod dialects;

use std::sync::Mutex;

use dialects::{DialectContext, DialectOutcome, ToolDialect, DIALECTS};

use super::{FinishReason, InferenceDelta, InferenceRequest, ToolCall, ToolCallDelta};
use crate::streaming::{DeltaSink, StreamError};

/// A block that never closes must not hold the turn hostage. Past this, the
/// buffer is released as the prose it evidently was.
const MAX_BUFFER: usize = 8 * 1024;

/// How much of a turn's text the radar keeps to describe an unknown dialect.
const RADAR_SAMPLE: usize = 240;

pub(crate) struct SalvagingSink<'a> {
    inner: &'a dyn DeltaSink<InferenceDelta>,
    tools: Vec<String>,
    model_id: String,
    state: Mutex<SalvageState>,
}

#[derive(Default)]
struct SalvageState {
    buffer: String,
    emitted_text: bool,
    salvaged: usize,
    sample: String,
}

impl<'a> SalvagingSink<'a> {
    /// `None` when the request declared no tools: a model with nothing to call
    /// cannot be miswriting a call, and prose should not even be scanned.
    pub(crate) fn wrap(
        inner: &'a dyn DeltaSink<InferenceDelta>,
        request: &InferenceRequest,
    ) -> Option<Self> {
        if request.tools.is_empty() {
            return None;
        }
        Some(Self {
            inner,
            tools: request.tools.iter().map(|tool| tool.name.clone()).collect(),
            model_id: request.target.model_id.clone(),
            state: Mutex::new(SalvageState::default()),
        })
    }

    /// Moves `upto` bytes (or the whole buffer) out as text. `upto` always
    /// comes from `find` or an opener length, so it is a char boundary.
    fn flush_text(&self, state: &mut SalvageState, upto: usize) -> Result<(), StreamError> {
        let upto = upto.min(state.buffer.len());
        if upto == 0 {
            return Ok(());
        }
        let text: String = state.buffer.drain(..upto).collect();
        state.emitted_text = true;
        self.inner.emit_delta(InferenceDelta::Text { text })
    }

    fn emit_call(&self, state: &mut SalvageState, call: ToolCall) -> Result<(), StreamError> {
        state.salvaged += 1;
        self.inner
            .emit_delta(InferenceDelta::ToolCallDelta(ToolCallDelta {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments_delta: call.arguments.clone(),
            }))?;
        self.inner.emit_delta(InferenceDelta::ToolCall(call))
    }

    fn drain(&self, state: &mut SalvageState, final_pass: bool) -> Result<(), StreamError> {
        loop {
            match scan(&state.buffer) {
                Scan::None => return self.flush_text(state, usize::MAX),
                // The tail could still grow into an opener, so it waits —
                // unless nothing more is coming.
                Scan::Prefix(index) => {
                    self.flush_text(state, index)?;
                    if final_pass {
                        self.flush_text(state, usize::MAX)?;
                    }
                    return Ok(());
                }
                Scan::Hit(index) => {
                    self.flush_text(state, index)?;
                    let ctx = DialectContext {
                        tools: &self.tools,
                        at_turn_start: !state.emitted_text && state.salvaged == 0,
                    };
                    let outcome = DIALECTS
                        .iter()
                        .filter(|dialect| opens_here(**dialect, &state.buffer))
                        .map(|dialect| (dialect.id(), dialect.parse(&state.buffer, &ctx)))
                        .find(|(_, outcome)| !matches!(outcome, DialectOutcome::NotMine));

                    match outcome {
                        Some((id, DialectOutcome::Calls { calls, consumed })) => {
                            if state.salvaged == 0 {
                                eprintln!(
                                    "tool salvage: rescued {} tool call(s) written as prose by {} \
                                     (dialect {id}) — the model is not emitting real tool calls",
                                    calls.len(),
                                    self.model_id,
                                );
                            }
                            for call in calls {
                                self.emit_call(state, call)?;
                            }
                            state.buffer.replace_range(..consumed, "");
                        }
                        Some((_, DialectOutcome::Incomplete))
                            if !final_pass && state.buffer.len() <= MAX_BUFFER =>
                        {
                            return Ok(())
                        }
                        // Unclaimed, truncated, or past the cap: it is prose.
                        // Releasing the opener guarantees forward progress and
                        // lets a real block later in the buffer still be found.
                        _ => self.release_opener(state)?,
                    }
                }
            }
        }
    }

    fn release_opener(&self, state: &mut SalvageState) -> Result<(), StreamError> {
        let length = DIALECTS
            .iter()
            .flat_map(|dialect| dialect.openers().iter())
            .filter(|opener| state.buffer.starts_with(**opener))
            .map(|opener| opener.len())
            .max()
            .unwrap_or_else(|| {
                state
                    .buffer
                    .chars()
                    .next()
                    .map_or(state.buffer.len(), char::len_utf8)
            });
        self.flush_text(state, length)
    }

    /// The anti-bottleneck half: when a turn's text looks like tool markup and
    /// no dialect claimed it, say so. The next format a model invents should
    /// arrive as a log line, not as a screenshot someone happened to notice.
    fn radar(&self, state: &SalvageState) {
        if state.salvaged > 0 {
            return;
        }
        let sample = &state.sample;
        let suspicious = sample.contains('\u{ff5c}')
            || sample.contains("invoke name=")
            || sample.contains("<tool_call")
            || sample.contains("<function")
            || (sample.contains("\"name\"")
                && self.tools.iter().any(|tool| sample.contains(tool.as_str())));
        if !suspicious {
            return;
        }
        eprintln!(
            "tool salvage: {} wrote something shaped like an unrecognised tool-call dialect \
             ({} declared tools, no dialect claimed it) — sample: {sample:?}",
            self.model_id,
            self.tools.len(),
        );
    }
}

impl DeltaSink<InferenceDelta> for SalvagingSink<'_> {
    fn emit_delta(&self, payload: InferenceDelta) -> Result<(), StreamError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StreamError::new("the tool-salvage buffer was poisoned"))?;

        match payload {
            InferenceDelta::Text { text } if !text.is_empty() => {
                if state.sample.chars().count() < RADAR_SAMPLE {
                    let room = RADAR_SAMPLE - state.sample.chars().count();
                    state.sample.extend(text.chars().take(room));
                }
                state.buffer.push_str(&text);
                self.drain(&mut state, false)
            }
            InferenceDelta::Finish(reason) => {
                self.drain(&mut state, true)?;
                self.radar(&state);
                // Salvaged calls make this a tool turn however the provider
                // labelled it — the chat loop reads the calls, but a truthful
                // reason keeps the transcript honest.
                let reason = match (state.salvaged, reason) {
                    (0, reason) => reason,
                    (_, FinishReason::Stop) => FinishReason::ToolCalls,
                    (_, reason) => reason,
                };
                drop(state);
                self.inner.emit_delta(InferenceDelta::Finish(reason))
            }
            other => {
                drop(state);
                self.inner.emit_delta(other)
            }
        }
    }
}

enum Scan {
    /// Nothing here can become markup.
    None,
    /// A partial opener sits at this index, at the tail of the buffer.
    Prefix(usize),
    /// A whole opener starts at this index.
    Hit(usize),
}

fn opens_here(dialect: &dyn ToolDialect, buffer: &str) -> bool {
    dialect
        .openers()
        .iter()
        .any(|opener| buffer.starts_with(opener))
}

fn scan(buffer: &str) -> Scan {
    let openers = || DIALECTS.iter().flat_map(|dialect| dialect.openers().iter());

    let hit = openers().filter_map(|opener| buffer.find(*opener)).min();

    // A buffer ending in "<\u{ff5c}DSM" is one chunk away from a real opener.
    let prefix = openers()
        .flat_map(|opener| {
            opener
                .char_indices()
                .skip(1)
                .filter(|(at, _)| buffer.ends_with(&opener[..*at]))
                .map(|(at, _)| buffer.len() - at)
        })
        .min();

    match (hit, prefix) {
        (Some(hit), Some(prefix)) if prefix < hit => Scan::Prefix(prefix),
        (Some(hit), _) => Scan::Hit(hit),
        (None, Some(prefix)) => Scan::Prefix(prefix),
        (None, None) => Scan::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{ModelTarget, ToolDeclaration};

    #[derive(Default)]
    struct Recorder {
        deltas: Mutex<Vec<InferenceDelta>>,
    }

    impl Recorder {
        fn text(&self) -> String {
            self.deltas
                .lock()
                .unwrap()
                .iter()
                .filter_map(|delta| match delta {
                    InferenceDelta::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        }

        fn calls(&self) -> Vec<ToolCall> {
            self.deltas
                .lock()
                .unwrap()
                .iter()
                .filter_map(|delta| match delta {
                    InferenceDelta::ToolCall(call) => Some(call.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    impl DeltaSink<InferenceDelta> for Recorder {
        fn emit_delta(&self, payload: InferenceDelta) -> Result<(), StreamError> {
            self.deltas.lock().unwrap().push(payload);
            Ok(())
        }
    }

    fn request_with_tools() -> InferenceRequest {
        InferenceRequest {
            id: "request-1".into(),
            target: ModelTarget {
                provider_id: "together".into(),
                model_id: "deepseek-ai/DeepSeek-V4-Flash-0731".into(),
            },
            messages: Vec::new(),
            tools: vec![ToolDeclaration {
                name: "search_conversations".into(),
                description: "search".into(),
                parameters: serde_json::json!({}),
            }],
            session_id: None,
        }
    }

    /// Feeds `chunks` as separate text deltas, then a Finish.
    fn run(chunks: &[&str]) -> Recorder {
        let recorder = Recorder::default();
        let request = request_with_tools();
        {
            let sink = SalvagingSink::wrap(&recorder, &request).expect("tools were declared");
            for chunk in chunks {
                sink.emit_delta(InferenceDelta::Text {
                    text: (*chunk).to_owned(),
                })
                .expect("text should stream");
            }
            sink.emit_delta(InferenceDelta::Finish(FinishReason::Stop))
                .expect("finish should stream");
        }
        recorder
    }

    /// The exact payload DeepSeek stored on 2026-09-06, fullwidth pipes and all.
    const DEEPSEEK_LEAK: &str = concat!(
        "\n\n<\u{ff5c}DSML\u{ff5c}tool_calls>\n",
        "<\u{ff5c}DSML\u{ff5c}invoke name=\"search_conversations\">\n",
        "<\u{ff5c}DSML\u{ff5c}parameter name=\"query\" string=\"true\">",
        "avatar rendered image created Moti</\u{ff5c}DSML\u{ff5c}parameter>\n",
        "</\u{ff5c}DSML\u{ff5c}invoke>\n",
        "<\u{ff5c}DSML\u{ff5c}invoke name=\"search_conversations\">\n",
        "<\u{ff5c}DSML\u{ff5c}parameter name=\"query\" string=\"true\">",
        "archivist crow avatar final</\u{ff5c}DSML\u{ff5c}parameter>\n",
        "</\u{ff5c}DSML\u{ff5c}invoke>\n",
        "</\u{ff5c}DSML\u{ff5c}tool_calls>\n",
    );

    #[test]
    fn the_stored_deepseek_leak_becomes_two_real_calls() {
        let recorder = run(&[DEEPSEEK_LEAK]);
        let calls = recorder.calls();

        assert_eq!(calls.len(), 2, "both invokes should be rescued");
        assert!(calls.iter().all(|call| call.name == "search_conversations"));
        assert_eq!(
            calls[0].arguments,
            r#"{"query":"avatar rendered image created Moti"}"#
        );
        assert_eq!(calls[1].arguments, r#"{"query":"archivist crow avatar final"}"#);
        assert_eq!(
            recorder.text().trim(),
            "",
            "no markup should survive as prose"
        );
    }

    #[test]
    fn an_opener_split_across_chunks_is_still_caught() {
        // The realistic failure: the opener lands in pieces.
        let mut chunks: Vec<String> = Vec::new();
        let mut rest = DEEPSEEK_LEAK;
        while !rest.is_empty() {
            let take = rest
                .char_indices()
                .nth(7)
                .map_or(rest.len(), |(index, _)| index);
            let (head, tail) = rest.split_at(take);
            chunks.push(head.to_owned());
            rest = tail;
        }
        let borrowed: Vec<&str> = chunks.iter().map(String::as_str).collect();

        let recorder = run(&borrowed);
        assert_eq!(recorder.calls().len(), 2);
        assert_eq!(recorder.text().trim(), "");
    }

    #[test]
    fn ascii_pipes_are_accepted_too() {
        let leak = DEEPSEEK_LEAK.replace('\u{ff5c}', "|");
        let recorder = run(&[&leak]);
        assert_eq!(recorder.calls().len(), 2);
    }

    #[test]
    fn prose_before_a_call_is_kept_and_forwarded() {
        let recorder = run(&["Let me look that up. ", DEEPSEEK_LEAK]);
        assert_eq!(recorder.calls().len(), 2);
        // The newlines wrapping the block are outside it, so they are prose and
        // survive; only the markup itself is consumed.
        assert_eq!(recorder.text(), "Let me look that up. \n\n\n");
    }

    #[test]
    fn a_truncated_block_is_released_rather_than_swallowed() {
        let truncated = "<\u{ff5c}DSML\u{ff5c}tool_calls>\n<\u{ff5c}DSML\u{ff5c}invoke name=\"sea";
        let recorder = run(&[truncated]);

        assert!(recorder.calls().is_empty());
        assert_eq!(
            recorder.text(),
            truncated,
            "an unfinished block must reach the user, not vanish"
        );
    }

    #[test]
    fn prose_that_merely_starts_with_an_angle_bracket_survives_intact() {
        let recorder = run(&["<not a tool call> and <tool_call-ish prose"]);
        assert!(recorder.calls().is_empty());
        assert_eq!(recorder.text(), "<not a tool call> and <tool_call-ish prose");
    }

    #[test]
    fn a_block_naming_an_undeclared_tool_stays_prose() {
        let leak = DEEPSEEK_LEAK.replace("search_conversations", "delete_everything");
        let recorder = run(&[&leak]);

        assert!(recorder.calls().is_empty(), "undeclared names are not calls");
        assert_eq!(recorder.text(), leak);
    }

    #[test]
    fn the_hermes_shape_is_rescued() {
        let recorder = run(&[
            "<tool_call>\n{\"name\": \"search_conversations\", ",
            "\"arguments\": {\"query\": \"crow\"}}\n</tool_call>",
        ]);
        let calls = recorder.calls();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments, r#"{"query":"crow"}"#);
    }

    #[test]
    fn bare_json_fires_only_at_the_start_of_a_turn() {
        let call = r#"{"name": "search_conversations", "arguments": {"query": "crow"}}"#;

        let opening = run(&[call]);
        assert_eq!(opening.calls().len(), 1, "a turn that is only the call");

        let mid_sentence = run(&["Here is the shape we send: ", call]);
        assert!(
            mid_sentence.calls().is_empty(),
            "JSON quoted mid-answer is prose, not a call"
        );
        assert!(mid_sentence.text().contains(call));
    }

    #[test]
    fn a_salvaged_turn_finishes_as_a_tool_turn() {
        let recorder = run(&[DEEPSEEK_LEAK]);
        let deltas = recorder.deltas.lock().unwrap();
        let finish = deltas.last().expect("a finish delta");

        assert_eq!(
            finish,
            &InferenceDelta::Finish(FinishReason::ToolCalls),
            "Stop would misreport a turn that asked for tools"
        );
    }

    #[test]
    fn calls_land_before_the_finish_delta() {
        let recorder = run(&[DEEPSEEK_LEAK]);
        let deltas = recorder.deltas.lock().unwrap();
        let last_call = deltas
            .iter()
            .rposition(|delta| matches!(delta, InferenceDelta::ToolCall(_)))
            .expect("a salvaged call");
        let finish = deltas
            .iter()
            .position(|delta| matches!(delta, InferenceDelta::Finish(_)))
            .expect("a finish");

        assert!(last_call < finish, "the chat loop reads calls before Finish");
    }

    #[test]
    fn a_tool_less_request_is_never_wrapped() {
        let recorder = Recorder::default();
        let mut request = request_with_tools();
        request.tools.clear();

        assert!(
            SalvagingSink::wrap(&recorder, &request).is_none(),
            "plain chat should not pay for scanning"
        );
    }
}
