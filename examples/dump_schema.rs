fn main() {
    let s = s106::llm::schema_for::<s106::wire::WireExtraction>();
    println!("{}", serde_json::to_string_pretty(&s).unwrap());
}
