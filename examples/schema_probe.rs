//! Diagnostic: which output schemas does the API compile? Sends one tiny text request per part schema.
//!
//! MAKES PAID API CALLS (three small requests, needs credentials, see README). Not part of any test.
//!
//!     cargo run --example schema_probe
use rust_claude_sdk::{ContentBlockParam, Effort, MessageParam, MessagesRequest, OutputConfig};
use s106::{llm, wire};
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let client = llm::client()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let probes = vec![
        (
            "instrument part",
            llm::schema_for::<wire::WireInstrumentPart>(),
        ),
        ("housing part", llm::schema_for::<wire::WireHousingPart>()),
        (
            "contributions part",
            llm::schema_for::<wire::WireContributionsPart>(),
        ),
    ];
    for (name, schema) in probes {
        let mut req =
            MessagesRequest::new(llm::DEFAULT_MODEL, 64).message(MessageParam::user(vec![
                ContentBlockParam::text("Reply with the smallest valid JSON."),
            ]));
        req.output_config = Some(OutputConfig {
            effort: Some(Effort::Low),
            format: Some(json!({"type": "json_schema", "schema": schema})),
        });
        let r = rt.block_on(client.create(&req));
        println!(
            "{name:<22} {} bytes: {}",
            schema.to_string().len(),
            match r {
                Ok(_) => "compiles".to_string(),
                Err(e) => e.to_string().chars().take(90).collect(),
            }
        );
    }
    Ok(())
}
