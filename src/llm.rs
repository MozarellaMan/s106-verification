//! The one place that talks to the Anthropic API (through `rust-claude-sdk`, pinned).
//!
//! Callers send a PDF, a prompt and a JSON schema, and get back the model's JSON text with usage.
//! SDK calls stay here; orchestration also names the concrete client type.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use base64::Engine;
use rust_claude_sdk::{
    Client, ClientConfig, ContentBlockParam, Effort, Fallbacks, MessageParam, MessagesRequest,
    OutputConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEFAULT_MODEL: &str = "claude-haiku-5-5";
pub const ESCALATION_MODEL: &str = "claude-opus-5-5";
/// Structured output occasionally arrives truncated (invalid JSON); retry this many times in all.
const PARSE_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub attempts: u32,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.attempts = self.attempts.max(o.attempts);
        self.cache_write_tokens += o.cache_write_tokens;
        self.cache_read_tokens += o.cache_read_tokens;
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    /// The model's structured output (validated JSON).
    pub output: Value,
    /// The model that actually served the request (differs when a fallback ran).
    pub served_by: String,
    pub usage: Usage,
}

/// The harness's key file (`~/.config/s106/anthropic_key`, or `S106_KEY_FILE`), else the
/// environment (`ANTHROPIC_API_KEY`). The key goes straight to the client, never into the environment.
pub fn client() -> Result<Client> {
    let key_file = std::env::var_os("S106_KEY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".config/s106/anthropic_key")
        });
    let config = match std::fs::read_to_string(&key_file) {
        Ok(k) if !k.trim().is_empty() => ClientConfig::api_key(k.trim()),
        _ => ClientConfig::from_env().context("no Anthropic credentials: write a key to ~/.config/s106/anthropic_key or set ANTHROPIC_API_KEY")?,
    };
    Ok(Client::new(config)?)
}

/// Make a Schemars schema acceptable to strict structured outputs: every object closed
/// (`additionalProperties: false`) with every property required (optional fields are nullable
/// instead), and keywords the API doesn't accept removed.
pub fn strict_schema(mut schema: Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Object(m) => {
                // Schemars writes enums with documented variants as `oneOf` of constants, which the
                // API doesn't accept; a plain string `enum` says the same thing.
                let consts: Option<Vec<Value>> =
                    m.get("oneOf").and_then(Value::as_array).and_then(|alts| {
                        alts.iter()
                            .map(|a| {
                                a.get("const").cloned().or_else(|| {
                                    a.get("enum")
                                        .and_then(|e| e.as_array())
                                        .filter(|e| e.len() == 1)
                                        .map(|e| e[0].clone())
                                })
                            })
                            .collect()
                    });
                if let Some(values) = consts {
                    m.remove("oneOf");
                    m.insert("type".into(), json!("string"));
                    m.insert("enum".into(), Value::Array(values));
                }
                for k in [
                    "$schema", "format", "minimum", "maximum", "title", "default",
                ] {
                    m.remove(k);
                }
                if m.get("type") == Some(&json!("object")) || m.contains_key("properties") {
                    let keys: Vec<Value> = m
                        .get("properties")
                        .and_then(Value::as_object)
                        .map_or(vec![], |p| p.keys().map(|k| json!(k)).collect());
                    m.insert("required".into(), Value::Array(keys));
                    m.insert("additionalProperties".into(), json!(false));
                }
                for (_, child) in m.iter_mut() {
                    walk(child);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut schema);
    schema
}

pub fn schema_for<T: schemars::JsonSchema>() -> Value {
    strict_schema(serde_json::to_value(schemars::schema_for!(T)).expect("schema serialises"))
}

/// Models that accept the server-side `fallbacks: "default"` parameter.
fn takes_fallbacks(model: &str) -> bool {
    matches!(
        model,
        "claude-opus-5-5" | "claude-opus-5" | "claude-fable-5-1" | "claude-sonnet-5-5"
    )
}

/// The request `extract` sends: the PDF as a base64 document block, then the prompt; the output
/// schema and effort in `output_config`; server-side fallbacks where the model takes them.
pub fn request(
    model: &str,
    pdf: &[u8],
    prompt: &str,
    schema: &Value,
    effort: Effort,
) -> MessagesRequest {
    let mut request = MessagesRequest::new(model, 32_000).message(MessageParam::user(vec![
        // Not cached: each extraction part uses a different output schema, which is part of the
        // cached prefix, so the three calls never share an entry (measured: 3 writes, 0 reads).
        ContentBlockParam::base64_pdf(base64::engine::general_purpose::STANDARD.encode(pdf)),
        ContentBlockParam::text(prompt),
    ]));
    request.output_config = Some(OutputConfig {
        effort: Some(effort),
        format: Some(json!({"type": "json_schema", "schema": schema})),
    });
    if takes_fallbacks(model) {
        request = request.fallbacks(Fallbacks::Default);
    }
    request
}

/// One structured-output call over a PDF. Retries truncated output; the SDK retries transport
/// errors, 408/409/429 and 5xx itself. Dropping the future cancels the request.
pub async fn extract(
    client: &Client,
    model: &str,
    pdf: &[u8],
    prompt: &str,
    schema: &Value,
    effort: Effort,
) -> Result<Response> {
    let request = request(model, pdf, prompt, schema, effort);
    let mut last_err = String::new();
    for attempt in 1..=PARSE_ATTEMPTS {
        // Streamed and accumulated: a long structured output on a non-streaming call risks HTTP timeouts.
        let message = client.stream(&request).await?.final_message().await?;
        if message.is_refusal() {
            bail!(
                "{model} declined the extraction (stop_details: {})",
                message
                    .stop_details
                    .map_or("none".into(), |d| d.to_string())
            );
        }
        let text = message.text();
        match serde_json::from_str::<Value>(&text) {
            Ok(output) if output.is_object() => {
                return Ok(Response {
                    output,
                    served_by: message.model.clone(),
                    usage: Usage {
                        input_tokens: message.usage.input_tokens,
                        output_tokens: message.usage.output_tokens,
                        attempts: attempt,
                        cache_write_tokens: message.usage.cache_creation_input_tokens.unwrap_or(0),
                        cache_read_tokens: message.usage.cache_read_input_tokens.unwrap_or(0),
                    },
                });
            }
            Ok(_) => last_err = "output is not a JSON object".into(),
            Err(e) => last_err = format!("{e} (stop_reason {:?})", message.stop_reason),
        }
        eprintln!(
            "  {model}: invalid structured output ({last_err}), retrying ({attempt}/{PARSE_ATTEMPTS})"
        );
    }
    bail!("{model} returned invalid structured output {PARSE_ATTEMPTS} times: {last_err}")
}

/// Rough cost in USD from the published per-MTok rates (input, output), with Haiku's
/// higher tier above 100k prompt tokens; cache writes at 1.25x and reads at 0.1x the input rate.
pub fn cost_usd(model: &str, u: &Usage) -> f64 {
    let prompt = u.input_tokens + u.cache_write_tokens + u.cache_read_tokens;
    let (i, o) = match model {
        "claude-haiku-5-5" if prompt > 100_000 => (0.50, 2.50),
        "claude-haiku-5-5" => (0.10, 0.50),
        "claude-sonnet-5-5" => (2.0, 10.0),
        "claude-opus-5-5" => (4.0, 20.0),
        _ => (0.0, 0.0),
    };
    (u.input_tokens as f64 * i
        + u.cache_write_tokens as f64 * i * 1.25
        + u.cache_read_tokens as f64 * i * 0.1
        + u.output_tokens as f64 * o)
        / 1e6
}

pub use rust_claude_sdk::Effort as ExtractEffort;
