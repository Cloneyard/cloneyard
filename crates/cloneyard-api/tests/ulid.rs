use cloneyard_api::system::PingResult;
use regex::Regex;
use ulid::Ulid;

fn node_id_pattern() -> Regex {
    let schema = schemars::schema_for!(PingResult);
    let pattern = schema.as_value()["properties"]["node_id"]["pattern"]
        .as_str()
        .unwrap();
    Regex::new(pattern).unwrap()
}

#[test]
fn the_ulid_pattern_matches_every_serialized_ulid() {
    let pattern = node_id_pattern();
    for node_id in [
        Ulid::generate(),
        Ulid::generate(),
        Ulid::nil(),
        Ulid::from(u128::MAX),
    ] {
        let serialized = serde_json::to_value(PingResult { node_id }).unwrap();
        let text = serialized["node_id"].as_str().unwrap();
        assert!(pattern.is_match(text), "{text}");
    }
}

#[test]
fn the_ulid_pattern_rejects_what_is_not_a_ulid() {
    let pattern = node_id_pattern();
    for text in [
        "01arz3ndektsv4rrffq69g5fav",
        "01ARZ3NDEKTSV4RRFFQ69G5FAI",
        "01ARZ3NDEKTSV4RRFFQ69G5FAL",
        "01ARZ3NDEKTSV4RRFFQ69G5FAO",
        "01ARZ3NDEKTSV4RRFFQ69G5FAU",
        "81ARZ3NDEKTSV4RRFFQ69G5FAV",
        "01ARZ3NDEKTSV4RRFFQ69G5FA",
        "01ARZ3NDEKTSV4RRFFQ69G5FAVV",
    ] {
        assert!(!pattern.is_match(text), "{text}");
    }
}
