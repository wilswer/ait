//! Local, metadata-only observability for LLM requests and provider prompt caching.
//!
//! Observations are written only through the application's local tracing logger;
//! nothing in this module transmits analytics or prompt data to a remote service.
//! Prompt contents are never logged. Stable, truncated SHA-256 fingerprints
//! make cache-affecting changes visible without exposing user data.

use std::time::Instant;

use genai::ModelSpec;
use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, StreamEnd, Tool};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTransition {
    FirstRequest,
    NormalAppend,
    ModelChanged,
    ProviderChanged,
    SystemPromptChanged,
    ToolsChanged,
    HistoryRewritten,
    LegacyReplay,
}

impl CacheTransition {
    fn as_str(self) -> &'static str {
        match self {
            Self::FirstRequest => "first_request",
            Self::NormalAppend => "normal_append",
            Self::ModelChanged => "model_changed",
            Self::ProviderChanged => "provider_changed",
            Self::SystemPromptChanged => "system_prompt_changed",
            Self::ToolsChanged => "tools_changed",
            Self::HistoryRewritten => "history_rewritten",
            Self::LegacyReplay => "legacy_replay",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromptFingerprint {
    model: String,
    provider: String,
    system_hash: Option<String>,
    tools_hash: String,
    history_hash: String,
    message_hashes: Vec<String>,
    request_hash: String,
    message_count: usize,
    tool_count: usize,
    prompt_bytes: usize,
    legacy_message_count: usize,
}

impl PromptFingerprint {
    pub fn build(
        model: &ModelSpec,
        system_prompt: &Option<String>,
        messages: &[ChatMessage],
        tools: &[Tool],
        legacy_message_count: usize,
    ) -> Self {
        let (provider, model_name) = model_identity(model);
        let system_hash = system_prompt.as_deref().map(hash_str);

        let tool_values = tools
            .iter()
            .map(|tool| canonical_serialized(tool))
            .collect::<Vec<_>>();
        let tools_hash = hash_json(&tool_values);

        let message_values = messages
            .iter()
            .map(canonical_serialized)
            .collect::<Vec<_>>();
        let message_hashes = message_values.iter().map(hash_json).collect::<Vec<_>>();
        let history_hash = hash_json(&message_hashes);
        let request_hash = hash_json(&(
            &provider,
            &model_name,
            &system_hash,
            &tools_hash,
            &message_hashes,
        ));
        let prompt_bytes = system_prompt.as_ref().map_or(0, String::len)
            + message_values.iter().map(serialized_len).sum::<usize>()
            + tool_values.iter().map(serialized_len).sum::<usize>();

        Self {
            model: model_name,
            provider,
            system_hash,
            tools_hash,
            history_hash,
            message_hashes,
            request_hash,
            message_count: messages.len(),
            tool_count: tools.len(),
            prompt_bytes,
            legacy_message_count,
        }
    }

    pub fn transition_from(&self, previous: Option<&Self>) -> CacheTransition {
        classify_transition(previous, self)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CacheUsage {
    pub prompt_tokens: Option<i32>,
    pub cached_tokens: Option<i32>,
    pub cache_creation_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
}

impl CacheUsage {
    pub fn from_stream_end(end: &StreamEnd) -> Self {
        let Some(usage) = end.captured_usage.as_ref() else {
            return Self::default();
        };
        let details = usage.prompt_tokens_details.as_ref();
        Self {
            prompt_tokens: usage.prompt_tokens,
            cached_tokens: details.and_then(|d| d.cached_tokens),
            cache_creation_tokens: details.and_then(|d| d.cache_creation_tokens),
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        }
    }

    pub fn uncached_prompt_tokens(&self) -> Option<i32> {
        self.prompt_tokens
            .map(|prompt| prompt.saturating_sub(self.cached_tokens.unwrap_or(0)))
    }

    pub fn hit_ratio(&self) -> Option<f64> {
        let prompt = self.prompt_tokens?;
        (prompt != 0).then(|| self.cached_tokens.unwrap_or(0) as f64 / prompt as f64)
    }
}

pub struct RequestObservation {
    conversation_id: i64,
    request_id: String,
    round: usize,
    fingerprint: PromptFingerprint,
    transition: CacheTransition,
    started: Instant,
    first_token: Option<Instant>,
    finished: bool,
}

impl RequestObservation {
    pub fn start(
        conversation_id: i64,
        request_id: String,
        round: usize,
        fingerprint: PromptFingerprint,
        transition: CacheTransition,
    ) -> Self {
        let observation = Self {
            conversation_id,
            request_id,
            round,
            fingerprint,
            transition,
            started: Instant::now(),
            first_token: None,
            finished: false,
        };
        observation.log_prepared();
        observation
    }

    pub fn mark_first_token(&mut self) {
        self.first_token.get_or_insert_with(Instant::now);
    }

    pub fn completed(&mut self, end: &StreamEnd) {
        let usage = CacheUsage::from_stream_end(end);
        let duration_ms = elapsed_ms(self.started);
        let ttft_ms = self
            .first_token
            .map(|instant| elapsed_ms_between(self.started, instant));
        let cache_hit_ratio = usage.hit_ratio();
        let cache_hit_percent = cache_hit_ratio.map(|ratio| ratio * 100.0);
        tracing::info!(
            event = "llm.request.completed",
            conversation_id = self.conversation_id,
            request_id = %self.request_id,
            round = self.round,
            prompt_tokens = ?usage.prompt_tokens,
            cached_tokens = ?usage.cached_tokens,
            cache_creation_tokens = ?usage.cache_creation_tokens,
            uncached_prompt_tokens = ?usage.uncached_prompt_tokens(),
            cache_hit_ratio = cache_hit_ratio.map(truncate_ratio).map(tracing::field::display),
            cache_hit_percent = cache_hit_percent.map(format_percent).map(tracing::field::display),
            completion_tokens = ?usage.completion_tokens,
            total_tokens = ?usage.total_tokens,
            duration_ms,
            time_to_first_token_ms = ?ttft_ms,
        );
        self.finished = true;
    }

    pub fn failed(&mut self, error: &dyn std::fmt::Display) {
        tracing::warn!(
            event = "llm.request.failed",
            conversation_id = self.conversation_id,
            request_id = %self.request_id,
            round = self.round,
            duration_ms = elapsed_ms(self.started),
            error = %error,
        );
        self.finished = true;
    }

    pub fn cancelled(&mut self) {
        tracing::info!(
            event = "llm.request.cancelled",
            conversation_id = self.conversation_id,
            request_id = %self.request_id,
            round = self.round,
            duration_ms = elapsed_ms(self.started),
        );
        self.finished = true;
    }

    pub fn ended_without_end_event(&mut self) {
        tracing::warn!(
            event = "llm.request.incomplete",
            conversation_id = self.conversation_id,
            request_id = %self.request_id,
            round = self.round,
            duration_ms = elapsed_ms(self.started),
            "stream ended without an explicit End event"
        );
        self.finished = true;
    }

    fn log_prepared(&self) {
        tracing::info!(
            event = "llm.request.prepared",
            conversation_id = self.conversation_id,
            request_id = %self.request_id,
            round = self.round,
            transition = self.transition.as_str(),
            provider = %self.fingerprint.provider,
            model = %self.fingerprint.model,
            message_count = self.fingerprint.message_count,
            tool_count = self.fingerprint.tool_count,
            prompt_bytes = self.fingerprint.prompt_bytes,
            legacy_message_count = self.fingerprint.legacy_message_count,
            system_hash = ?self.fingerprint.system_hash,
            tools_hash = %self.fingerprint.tools_hash,
            history_hash = %self.fingerprint.history_hash,
            request_hash = %self.fingerprint.request_hash,
        );
    }
}

impl Drop for RequestObservation {
    fn drop(&mut self) {
        if !self.finished {
            tracing::warn!(
                event = "llm.request.abandoned",
                conversation_id = self.conversation_id,
                request_id = %self.request_id,
                round = self.round,
                duration_ms = elapsed_ms(self.started),
            );
        }
    }
}

pub fn classify_transition(
    previous: Option<&PromptFingerprint>,
    current: &PromptFingerprint,
) -> CacheTransition {
    let Some(previous) = previous else {
        return if current.legacy_message_count > 0 {
            CacheTransition::LegacyReplay
        } else {
            CacheTransition::FirstRequest
        };
    };
    if previous.provider != current.provider {
        return CacheTransition::ProviderChanged;
    }
    if previous.model != current.model {
        return CacheTransition::ModelChanged;
    }
    if previous.system_hash != current.system_hash {
        return CacheTransition::SystemPromptChanged;
    }
    if previous.tools_hash != current.tools_hash {
        return CacheTransition::ToolsChanged;
    }
    if current.legacy_message_count > previous.legacy_message_count {
        return CacheTransition::LegacyReplay;
    }
    if current.message_hashes.starts_with(&previous.message_hashes) {
        return CacheTransition::NormalAppend;
    }
    CacheTransition::HistoryRewritten
}

fn model_identity(model: &ModelSpec) -> (String, String) {
    match model {
        ModelSpec::Iden(identity) => (
            identity.adapter_kind.as_str().to_owned(),
            identity.model_name.to_string(),
        ),
        ModelSpec::Name(name) => (infer_provider(name).to_owned(), name.to_string()),
        ModelSpec::Target(target) => (
            target.model.adapter_kind.as_str().to_owned(),
            target.model.model_name.to_string(),
        ),
    }
}

fn infer_provider(model: &str) -> &'static str {
    if model.starts_with("gpt-") || model.starts_with("o1") || model.starts_with("o3") {
        AdapterKind::OpenAI.as_str()
    } else if model.starts_with("claude-") {
        AdapterKind::Anthropic.as_str()
    } else if model.starts_with("gemini-") {
        AdapterKind::Gemini.as_str()
    } else {
        "auto"
    }
}

fn canonical_serialized<T: Serialize>(value: &T) -> Value {
    canonicalize_json(serde_json::to_value(value).unwrap_or(Value::Null))
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

fn hash_str(value: &str) -> String {
    hash_bytes(value.as_bytes())
}

fn hash_json<T: Serialize>(value: &T) -> String {
    hash_bytes(&serde_json::to_vec(value).unwrap_or_default())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries = map.into_iter().collect::<Vec<_>>();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize_json(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        other => other,
    }
}

fn truncate_ratio(ratio: f64) -> f64 {
    (ratio * 10_000.0).round() / 10_000.0
}

fn format_percent(percent: f64) -> String {
    format!("{percent:.2}%")
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn elapsed_ms_between(started: Instant, ended: Instant) -> u64 {
    ended.duration_since(started).as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_prompt_has_no_ratio() {
        let usage = CacheUsage {
            prompt_tokens: Some(0),
            cached_tokens: Some(0),
            ..Default::default()
        };
        assert_eq!(usage.hit_ratio(), None);
    }

    #[test]
    fn usage_calculations_are_safe() {
        let usage = CacheUsage {
            prompt_tokens: Some(100),
            cached_tokens: Some(70),
            ..Default::default()
        };
        assert_eq!(usage.uncached_prompt_tokens(), Some(30));
        assert_eq!(usage.hit_ratio(), Some(0.7));
    }

    #[test]
    fn ratio_and_percent_are_readable() {
        let ratio = 0.9839125997959061;
        assert_eq!(truncate_ratio(ratio), 0.9839);
        assert_eq!(format_percent(ratio * 100.0), "98.39%");
    }

    #[test]
    fn canonical_json_ignores_object_key_order() {
        let a = serde_json::json!({"a": 1, "b": {"x": 2, "y": 3}});
        let b = serde_json::json!({"b": {"y": 3, "x": 2}, "a": 1});
        assert_eq!(
            hash_json(&canonicalize_json(a)),
            hash_json(&canonicalize_json(b))
        );
    }

    #[test]
    fn append_is_normal_transition() {
        let old = PromptFingerprint {
            model: "m".into(),
            provider: "p".into(),
            system_hash: None,
            tools_hash: "t".into(),
            history_hash: "h".into(),
            message_hashes: vec!["a".into()],
            request_hash: "r".into(),
            message_count: 1,
            tool_count: 0,
            prompt_bytes: 1,
            legacy_message_count: 0,
        };
        let mut current = old.clone();
        current.message_hashes.push("b".into());
        assert_eq!(
            classify_transition(Some(&old), &current),
            CacheTransition::NormalAppend
        );
    }
}
