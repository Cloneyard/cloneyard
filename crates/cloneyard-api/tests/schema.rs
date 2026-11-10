use std::{env, fs};

const SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema/cloneyard.openrpc.json"
);

#[test]
fn the_committed_schema_is_the_generated_document() {
    let generated = serde_json::to_string_pretty(&cloneyard_api::document()).unwrap() + "\n";

    if env::var("UPDATE_SCHEMA").is_ok_and(|update| update == "1") {
        fs::write(SCHEMA, generated).unwrap();
        return;
    }

    let committed = fs::read_to_string(SCHEMA).unwrap_or_default();
    assert!(
        committed == generated,
        "schema/cloneyard.openrpc.json is stale: run UPDATE_SCHEMA=1 cargo test -p cloneyard-api --test schema"
    );
}
