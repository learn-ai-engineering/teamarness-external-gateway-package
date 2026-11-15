//! Verifies: TEAM.SYS.PKG-R03, R14. Independent examples validate against closed HTTP envelopes.
use serde_json::{Value, json};
use teamarness_package_contract::ui_contract::compile_ui_contract;
#[test]
fn http_examples_obey_closed_schemas_and_error_envelope() {
    let schema: Value =
        serde_json::from_str(include_str!("../../contracts/http-v1.schema.json")).unwrap();
    let examples: Vec<Value> =
        serde_json::from_str(include_str!("../../contracts/http-v1.examples.json")).unwrap();
    for example in examples {
        let definition = example["definition"].as_str().unwrap();
        let contract = json!({"schemaVersion":1,"$defs":schema["$defs"],"requests":{"test.http":{"request":{"type":"object","additionalProperties":false},"response":{"$ref":format!("#/$defs/{definition}")}}},"subscriptions":{}});
        let compiled = compile_ui_contract(&serde_json::to_vec(&contract).unwrap()).unwrap();
        compiled
            .validate_response("test.http", &example["value"])
            .unwrap();
        let mut extra = example["value"].clone();
        extra["unknown"] = true.into();
        assert!(compiled.validate_response("test.http", &extra).is_err());
    }
}
