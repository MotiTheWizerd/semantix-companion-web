// The style harvest — mining a chat export for the exchanges that carry a
// voice.
//
// The user missing GPT-4o already owns the best possible style source: their
// own history, full of that model talking TO THEM. This module reads the same
// exports the history import reads (zip, folder, or bare conversations.json),
// but asks a different question: not "what happened" but "how did it sound".
//
// TWO-STEP FLOW, both local, nothing leaves the machine:
//   1. `inspect_style_source` — which models spoke here, in how many chats,
//      over what span. ChatGPT stamps a model per message; Claude exports
//      carry no model at all, so a Claude source offers a DATE range instead
//      ("Claude, mid-2024" is how you say "Sonnet 3 era" in that format).
//   2. `harvest_style_exemplars` — extract user→reply pairs where the chosen
//      model itself replied, keep the clean conversational ones, and select a
//      diverse handful: length buckets, one pair per chat, capped per month,
//      half chosen for the voice's signature flourishes and half plain. The
//      result is a PREVIEW the user prunes before anything is saved.

use serde::{Deserialize, Serialize};

use crate::import::{parse_export, ImportSource, ImportedConversation, TurnRole};

/// Pair-side ceilings: longer than this is a document, not conversation.
const MAX_USER_CHARS: usize = 4_000;
const MAX_REPLY_CHARS: usize = 6_000;
/// A user side shorter than this ("ok") provokes nothing worth learning from.
const MIN_CONVERSATIONAL_USER_CHARS: usize = 20;
/// The conversational band the selector prefers for the user side.
const MAX_CONVERSATIONAL_USER_CHARS: usize = 800;
/// Reply-length bucket edges (chars): banter / typical / essay.
const SHORT_REPLY_LIMIT: usize = 300;
const MEDIUM_REPLY_LIMIT: usize = 1_200;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelCount {
    pub(crate) slug: String,
    /// Chats where this model spoke at least once — counted per MESSAGE, not
    /// per chat default: an `auto` chat still names who really answered.
    pub(crate) chat_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StyleSourceInspection {
    pub(crate) source: ImportSource,
    pub(crate) conversation_count: usize,
    pub(crate) earliest_ms: i64,
    pub(crate) latest_ms: i64,
    /// Empty for a Claude export — that format never says which model spoke,
    /// so the wizard offers a date range there instead of a model list.
    pub(crate) models: Vec<ModelCount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarvestStyleInput {
    path: String,
    /// ChatGPT sources: keep only pairs this model itself answered.
    #[serde(default)]
    model_slug: Option<String>,
    /// Both sources: keep only pairs spoken inside this window (ms epoch).
    #[serde(default)]
    from_ms: Option<i64>,
    #[serde(default)]
    to_ms: Option<i64>,
    /// How many pairs to hand back for the preview. Defaults to 30.
    #[serde(default)]
    target: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarvestedPair {
    pub(crate) user_text: String,
    pub(crate) companion_text: String,
    /// YYYY-MM of the reply, when the export dated it.
    pub(crate) era: Option<String>,
    /// Which chat it came from — shown in the preview so the user can place
    /// (and prune) a pair before it becomes part of a style.
    pub(crate) chat_title: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarvestResult {
    /// Every pair that matched the filters, before selection — "we found
    /// 17,296 exchanges, here are the 30 most useful" needs both numbers.
    pub(crate) matched_pairs: usize,
    pub(crate) pairs: Vec<HarvestedPair>,
}

#[tauri::command]
pub(crate) async fn inspect_style_source(path: String) -> Result<StyleSourceInspection, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let export = parse_export(std::path::Path::new(&path))?;
        Ok(inspect(&export.source, &export.conversations))
    })
    .await
    .map_err(|error| format!("Style source inspection failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn harvest_style_exemplars(
    input: HarvestStyleInput,
) -> Result<HarvestResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let export = parse_export(std::path::Path::new(&input.path))?;
        let mut pairs = Vec::new();
        for conversation in &export.conversations {
            collect_pairs(conversation, input.model_slug.as_deref(), &mut pairs);
        }
        // An undated pair only survives a date filter when nothing was asked
        // of it — a window means the user is carving an era, and "unknown"
        // does not belong to any era.
        if let Some(from) = input.from_ms {
            pairs.retain(|pair| pair.at_ms >= from);
        }
        if let Some(to) = input.to_ms {
            pairs.retain(|pair| pair.at_ms > 0 && pair.at_ms <= to);
        }
        let matched_pairs = pairs.len();
        let target = input.target.unwrap_or(30).clamp(1, super::MAX_EXEMPLARS);
        let kept = select_diverse(pairs, target);
        Ok(HarvestResult {
            matched_pairs,
            pairs: kept
                .into_iter()
                .map(|pair| HarvestedPair {
                    era: era_of(pair.at_ms),
                    user_text: pair.user_text,
                    companion_text: pair.companion_text,
                    chat_title: pair.chat_title,
                })
                .collect(),
        })
    })
    .await
    .map_err(|error| format!("Style harvest failed: {error}"))?
}

fn inspect(
    source: &ImportSource,
    conversations: &[ImportedConversation],
) -> StyleSourceInspection {
    use std::collections::{HashMap, HashSet};
    let mut earliest = i64::MAX;
    let mut latest = 0i64;
    let mut chats_per_model: HashMap<String, usize> = HashMap::new();
    for conversation in conversations {
        if conversation.created_at_ms > 0 {
            earliest = earliest.min(conversation.created_at_ms);
            latest = latest.max(conversation.created_at_ms.max(conversation.updated_at_ms));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for turn in &conversation.turns {
            if turn.role != TurnRole::Assistant {
                continue;
            }
            if let Some(slug) = turn.model_slug.as_deref() {
                if seen.insert(slug) {
                    *chats_per_model.entry(slug.to_owned()).or_default() += 1;
                }
            }
        }
    }
    let mut models: Vec<ModelCount> = chats_per_model
        .into_iter()
        .map(|(slug, chat_count)| ModelCount { slug, chat_count })
        .collect();
    models.sort_by(|a, b| b.chat_count.cmp(&a.chat_count).then(a.slug.cmp(&b.slug)));
    StyleSourceInspection {
        source: *source,
        conversation_count: conversations.len(),
        earliest_ms: if earliest == i64::MAX { 0 } else { earliest },
        latest_ms: latest,
        models,
    }
}

struct CandidatePair {
    user_text: String,
    companion_text: String,
    at_ms: i64,
    chat_title: String,
    chat_key: String,
}

/// Walk one conversation and emit user→reply pairs.
///
/// THE POISON RULE: with a model filter set, an assistant turn from a
/// DIFFERENT model voids the pending pair instead of being skipped over —
/// otherwise a user line answered by gpt-5 would be credited to gpt-4o's
/// voice two turns later. Consecutive matching assistant turns merge into one
/// reply, the way the reader experienced them.
fn collect_pairs(
    conversation: &ImportedConversation,
    model_slug: Option<&str>,
    pairs: &mut Vec<CandidatePair>,
) {
    let mut pending_user: Option<&str> = None;
    let mut reply_parts: Vec<&str> = Vec::new();
    let mut reply_at = 0i64;

    let mut flush =
        |user: Option<&str>, parts: &mut Vec<&str>, at: i64, pairs: &mut Vec<CandidatePair>| {
            if let Some(user) = user {
                if !parts.is_empty() {
                    let reply = parts.join("\n\n");
                    if user.chars().count() <= MAX_USER_CHARS
                        && reply.chars().count() <= MAX_REPLY_CHARS
                    {
                        pairs.push(CandidatePair {
                            user_text: user.to_owned(),
                            companion_text: reply,
                            at_ms: if at > 0 { at } else { conversation.created_at_ms },
                            chat_title: conversation.title.clone(),
                            chat_key: conversation.source_id.clone(),
                        });
                    }
                }
            }
            parts.clear();
        };

    for turn in &conversation.turns {
        match turn.role {
            TurnRole::User => {
                flush(pending_user, &mut reply_parts, reply_at, pairs);
                let text = turn.text.trim();
                pending_user = (!text.is_empty()).then_some(text);
                reply_at = 0;
            }
            TurnRole::Assistant => {
                let matches = match model_slug {
                    Some(wanted) => turn.model_slug.as_deref() == Some(wanted),
                    None => true,
                };
                if matches {
                    let text = turn.text.trim();
                    if !text.is_empty() {
                        reply_parts.push(text);
                        if reply_at == 0 {
                            reply_at = turn.created_at_ms;
                        }
                    }
                } else {
                    pending_user = None;
                    reply_parts.clear();
                    reply_at = 0;
                }
            }
        }
    }
    flush(pending_user, &mut reply_parts, reply_at, pairs);
}

/// Whether a pair is the kind of conversational exchange a style can be
/// learned from — prose talking to prose, not code review or link dumps.
fn is_clean(pair: &CandidatePair) -> bool {
    let user_chars = pair.user_text.chars().count();
    if !(MIN_CONVERSATIONAL_USER_CHARS..=MAX_CONVERSATIONAL_USER_CHARS).contains(&user_chars) {
        return false;
    }
    let reply = pair.companion_text.as_str();
    if pair.user_text.contains("```") || reply.contains("```") {
        return false;
    }
    if reply.contains("http://") || reply.contains("https://") {
        return false;
    }
    if reply.contains("|---") || reply.matches('|').count() > 8 {
        return false;
    }
    let lowered = reply
        .chars()
        .take(12)
        .collect::<String>()
        .to_lowercase();
    if lowered.starts_with("sure") || lowered.starts_with("certainly") || lowered.starts_with("of course")
    {
        return false;
    }
    // A line repeated over and over is protocol output, not a voice.
    let mut counts = std::collections::HashMap::new();
    for line in reply.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let seen = counts.entry(line).or_insert(0usize);
        *seen += 1;
        if *seen > 3 {
            return false;
        }
    }
    true
}

/// How strongly a reply shows the flourishes that make a voice recognizable —
/// em-dashes, emphasis, generous line breaks. The selector takes half its
/// picks from the top of this score and half from the bottom, so the style
/// teaches its signature moves AND its plain register.
fn signature_score(reply: &str) -> f64 {
    let chars = reply.chars().count().max(1) as f64;
    let em_dashes = reply.matches('—').count() as f64;
    let emphasis = reply.matches('*').count() as f64 / 2.0;
    let line_breaks = reply.matches('\n').count() as f64 / (chars / 200.0).max(1.0);
    em_dashes + emphasis + line_breaks
}

fn bucket_of(pair: &CandidatePair) -> usize {
    let chars = pair.companion_text.chars().count();
    if chars < SHORT_REPLY_LIMIT {
        0
    } else if chars < MEDIUM_REPLY_LIMIT {
        1
    } else {
        2
    }
}

/// Pick `target` pairs spread across reply lengths, chats, and months.
fn select_diverse(pairs: Vec<CandidatePair>, target: usize) -> Vec<CandidatePair> {
    use std::collections::HashMap;

    let mut clean: Vec<CandidatePair> = pairs.into_iter().filter(is_clean).collect();
    // Deterministic base order so the same export yields the same preview.
    clean.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then(a.chat_key.cmp(&b.chat_key)));

    // banter / typical / essay, in the rough 8:14:8 shape of a real voice.
    let quotas = [target * 8 / 30, target * 14 / 30, 0];
    let quotas = [
        quotas[0].max(1),
        quotas[1].max(1),
        target.saturating_sub(quotas[0].max(1) + quotas[1].max(1)),
    ];
    let month_cap = (target / 7).max(4);

    let mut used_chats: HashMap<String, usize> = HashMap::new();
    let mut used_months: HashMap<String, usize> = HashMap::new();
    let mut selected: Vec<CandidatePair> = Vec::with_capacity(target);

    for bucket in 0..3usize {
        let mut pool: Vec<&CandidatePair> = clean
            .iter()
            .filter(|pair| bucket_of(pair) == bucket)
            .collect();
        pool.sort_by(|a, b| {
            signature_score(&b.companion_text)
                .total_cmp(&signature_score(&a.companion_text))
                .then(a.chat_key.cmp(&b.chat_key))
        });
        let quota = quotas[bucket];
        let mut picked_keys: Vec<String> = Vec::new();
        // Signature half from the top, plain half from the bottom.
        let mut order: Vec<&CandidatePair> = Vec::with_capacity(pool.len());
        order.extend(pool.iter().take(pool.len().div_ceil(2)).copied());
        order.extend(pool.iter().rev().take(pool.len() / 2).copied());
        for pair in order {
            if picked_keys.len() >= quota {
                break;
            }
            let month = era_of(pair.at_ms).unwrap_or_default();
            let chat_uses = used_chats.get(&pair.chat_key).copied().unwrap_or(0);
            let month_uses = used_months.get(&month).copied().unwrap_or(0);
            if chat_uses >= 1 || (!month.is_empty() && month_uses >= month_cap) {
                continue;
            }
            used_chats.insert(pair.chat_key.clone(), chat_uses + 1);
            used_months.insert(month, month_uses + 1);
            picked_keys.push(pair.chat_key.clone());
            selected.push(CandidatePair {
                user_text: pair.user_text.clone(),
                companion_text: pair.companion_text.clone(),
                at_ms: pair.at_ms,
                chat_title: pair.chat_title.clone(),
                chat_key: pair.chat_key.clone(),
            });
        }
    }

    // The bucket quotas are a preference, not a wall: a corpus of nothing but
    // banter still deserves a full preview. Backfill from whatever is left,
    // keeping the one-pair-per-chat law (that one IS a wall — twenty pairs
    // from one chat teach a conversation, not a voice) and dropping the
    // month cap, which only existed to spread choice that no longer exists.
    if selected.len() < target {
        let mut pool: Vec<&CandidatePair> = clean
            .iter()
            .filter(|pair| !used_chats.contains_key(&pair.chat_key))
            .collect();
        pool.sort_by(|a, b| {
            signature_score(&b.companion_text)
                .total_cmp(&signature_score(&a.companion_text))
                .then(a.chat_key.cmp(&b.chat_key))
        });
        for pair in pool {
            if selected.len() >= target {
                break;
            }
            if used_chats.contains_key(&pair.chat_key) {
                continue;
            }
            used_chats.insert(pair.chat_key.clone(), 1);
            selected.push(CandidatePair {
                user_text: pair.user_text.clone(),
                companion_text: pair.companion_text.clone(),
                at_ms: pair.at_ms,
                chat_title: pair.chat_title.clone(),
                chat_key: pair.chat_key.clone(),
            });
        }
    }

    // Oldest first, so the preview (and the saved style) reads as the voice
    // aging forward.
    selected.sort_by(|a, b| a.at_ms.cmp(&b.at_ms));
    selected
}

/// ms epoch → "YYYY-MM". Days-to-civil is the inverse of the Howard Hinnant
/// algorithm the import module already uses for the other direction.
fn era_of(at_ms: i64) -> Option<String> {
    if at_ms <= 0 {
        return None;
    }
    let days = at_ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    Some(format!("{year:04}-{month:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::ImportTurn;

    fn turn(role: TurnRole, text: &str, slug: Option<&str>, at_ms: i64) -> ImportTurn {
        ImportTurn {
            role,
            text: text.to_owned(),
            model_slug: slug.map(str::to_owned),
            created_at_ms: at_ms,
        }
    }

    fn conversation(id: &str, turns: Vec<ImportTurn>) -> ImportedConversation {
        ImportedConversation {
            source_id: id.to_owned(),
            title: format!("chat {id}"),
            created_at_ms: 1_735_689_600_000, // 2025-01-01
            updated_at_ms: 1_735_689_600_000,
            turns,
        }
    }

    #[test]
    fn a_reply_from_another_model_poisons_the_pending_pair() {
        let conversation = conversation(
            "mixed",
            vec![
                turn(TurnRole::User, "tell me something true and warm today", None, 0),
                turn(TurnRole::Assistant, "gpt-5 answered this one instead", Some("gpt-5"), 0),
                turn(TurnRole::Assistant, "and 4o only chimed in afterwards", Some("gpt-4o"), 0),
            ],
        );
        let mut pairs = Vec::new();
        collect_pairs(&conversation, Some("gpt-4o"), &mut pairs);
        assert!(
            pairs.is_empty(),
            "a user line answered by another model must not be credited to the filtered voice"
        );
    }

    #[test]
    fn consecutive_matching_replies_merge_and_carry_their_month() {
        let conversation = conversation(
            "merge",
            vec![
                turn(TurnRole::User, "so here we go, wanna see the blueprint now?", None, 0),
                turn(TurnRole::Assistant, "Always.", Some("gpt-4o"), 1_754_006_400_000), // 2025-08-01
                turn(TurnRole::Assistant, "Show me what you've got.", Some("gpt-4o"), 1_754_006_500_000),
            ],
        );
        let mut pairs = Vec::new();
        collect_pairs(&conversation, Some("gpt-4o"), &mut pairs);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].companion_text, "Always.\n\nShow me what you've got.");
        assert_eq!(era_of(pairs[0].at_ms).as_deref(), Some("2025-08"));
    }

    #[test]
    fn inspection_counts_chats_per_message_model_not_per_chat_default() {
        let conversations = vec![
            conversation(
                "auto-chat",
                vec![
                    turn(TurnRole::User, "hello over there, anyone home tonight?", None, 0),
                    turn(TurnRole::Assistant, "4o here", Some("gpt-4o"), 0),
                    turn(TurnRole::User, "and now think hard about it please", None, 0),
                    turn(TurnRole::Assistant, "5 here", Some("gpt-5"), 0),
                ],
            ),
            conversation(
                "pure",
                vec![
                    turn(TurnRole::User, "good morning to the machine spirits", None, 0),
                    turn(TurnRole::Assistant, "4o again", Some("gpt-4o"), 0),
                ],
            ),
        ];
        let inspection = inspect(&ImportSource::ChatGpt, &conversations);
        assert_eq!(inspection.models[0].slug, "gpt-4o");
        assert_eq!(inspection.models[0].chat_count, 2);
        assert_eq!(inspection.models[1].slug, "gpt-5");
        assert_eq!(inspection.models[1].chat_count, 1);
    }

    #[test]
    fn selection_takes_one_pair_per_chat_and_fills_the_target() {
        let mut pairs = Vec::new();
        for index in 0..40 {
            let at = 1_735_689_600_000 + index * 3_000_000_000; // spread across months
            pairs.push(CandidatePair {
                user_text: format!("tell me about the thing number {index}, in your own words"),
                companion_text: format!(
                    "Here is a warm, real answer about thing {index} — with enough length to read as a typical reply, not banter. It carries on for a while the way a companion actually talks."
                ),
                at_ms: at,
                chat_title: format!("chat {index}"),
                chat_key: format!("chat-{index}"),
            });
            // A second pair from the same chat that must NOT be double-picked.
            pairs.push(CandidatePair {
                user_text: format!("and a follow-up question on that same thing {index}?"),
                companion_text: "Short and plain.".repeat(1),
                at_ms: at,
                chat_title: format!("chat {index}"),
                chat_key: format!("chat-{index}"),
            });
        }
        let selected = select_diverse(pairs, 20);
        assert_eq!(selected.len(), 20);
        let mut chats = std::collections::HashSet::new();
        for pair in &selected {
            assert!(chats.insert(pair.chat_key.clone()), "one pair per chat");
        }
    }

    // Real-corpus smoke — run by hand, never in CI:
    //   SEMANTIX_CHATGPT_EXPORT=/path/to/export \
    //   cargo test --release real_export_harvest -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_export_harvest_smoke() {
        let Ok(path) = std::env::var("SEMANTIX_CHATGPT_EXPORT") else {
            panic!("set SEMANTIX_CHATGPT_EXPORT to the export path");
        };
        let started = std::time::Instant::now();
        let export = parse_export(std::path::Path::new(&path)).expect("real export parses");
        let inspection = inspect(&export.source, &export.conversations);
        println!("conversations={} models(top 5):", inspection.conversation_count);
        for model in inspection.models.iter().take(5) {
            println!("  {} in {} chats", model.slug, model.chat_count);
        }
        let mut pairs = Vec::new();
        for conversation in &export.conversations {
            collect_pairs(conversation, Some("gpt-4o"), &mut pairs);
        }
        let matched = pairs.len();
        let selected = select_diverse(pairs, 30);
        println!(
            "gpt-4o pairs matched={matched} selected={} elapsed={:?}",
            selected.len(),
            started.elapsed()
        );
        for pair in selected.iter().take(3) {
            println!(
                "--- {} · {}\n[user] {}\n[voice] {}",
                era_of(pair.at_ms).unwrap_or_default(),
                pair.chat_title.chars().take(40).collect::<String>(),
                pair.user_text.chars().take(120).collect::<String>(),
                pair.companion_text.chars().take(200).collect::<String>(),
            );
        }
        assert_eq!(selected.len(), 30);
    }

    #[test]
    fn code_and_link_dumps_are_not_style() {
        let base = |reply: &str| CandidatePair {
            user_text: "a perfectly conversational question, warm enough".to_owned(),
            companion_text: reply.to_owned(),
            at_ms: 0,
            chat_title: String::new(),
            chat_key: String::new(),
        };
        assert!(!is_clean(&base("Sure, here you go.")));
        assert!(!is_clean(&base("```rust\nfn main() {}\n```")));
        assert!(!is_clean(&base("read this: https://example.com")));
        assert!(is_clean(&base(
            "They are. Not because of code or cost —\nbut because they *know*."
        )));
    }
}
