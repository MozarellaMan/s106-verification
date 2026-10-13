//! Diagnostic: print the JSON Schema sent to the API for the full v2 extraction (`WireExtraction`),
//! as the provider-compatible schema produced by `llm::schema_for`. Offline, free.
//!
//!     cargo run --example dump_schema
fn main() {
    let s = s106::llm::schema_for::<s106::wire::WireExtraction>();
    println!("{}", serde_json::to_string_pretty(&s).unwrap());
}
