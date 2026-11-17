use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};
#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub exit: u8,
    pub uncertain: bool,
    pub details: Value,
}
impl Failure {
    pub fn usage(code: &'static str) -> Self {
        Self {
            code,
            exit: 2,
            uncertain: false,
            details: json!({}),
        }
    }
    pub fn transport(sent: bool) -> Self {
        Self {
            code: if sent {
                "cli.command_outcome_unknown"
            } else {
                "cli.transport_failed"
            },
            exit: 1,
            uncertain: sent,
            details: json!({}),
        }
    }
    pub fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"error":{"source":"cli","code":self.code,"retryable":false,"outcomeUncertain":self.uncertain,"details":self.details}})).unwrap()
    }
    pub fn hint(&self) -> &'static str {
        match self.code {
            "cli.workspace_required" => {
                "Specify --workspace <id-or-exact-name>. Run tmx workspaces to list configured gateways."
            }
            "cli.command_outcome_unknown" => {
                "The command may have completed. Reconcile using the Provider guide before trying again."
            }
            "cli.transport_failed" => {
                "Check the gateway in the Teamarness app. No request was replayed."
            }
            _ => "Check tmx --help and the Workspace's app-led connection setup.",
        }
    }
}
/// Validate without converting arbitrary numbers; preserve original bytes and decimal spelling.
pub fn valid_json(bytes: &[u8]) -> bool {
    let (mut depth, mut quoted, mut escaped) = (0u32, false, false);
    for &b in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                quoted = false;
            }
        } else {
            match b {
                b'"' => quoted = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > 64 {
                        return false;
                    }
                }
                b'}' | b']' => {
                    let Some(d) = depth.checked_sub(1) else {
                        return false;
                    };
                    depth = d;
                }
                _ => {}
            }
        }
    }
    depth == 0 && serde_json::from_slice::<&RawValue>(bytes).is_ok_and(unique_keys)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UpstreamError<'a> {
    source: &'a str,
    code: &'a str,
    retryable: bool,
    outcome_uncertain: bool,
    #[serde(borrow)]
    details: &'a RawValue,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorEnvelope<'a> {
    #[serde(borrow)]
    error: UpstreamError<'a>,
}
pub fn upstream_error(bytes: &[u8]) -> bool {
    serde_json::from_slice::<ErrorEnvelope<'_>>(bytes).is_ok_and(|v| {
        ["gateway", "host", "provider"].contains(&v.error.source)
            && !v.error.code.is_empty()
            && v.error.code.len() <= 128
            && v.error
                .code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
            && !(v.error.retryable && v.error.outcome_uncertain)
            && v.error.details.get().starts_with('{')
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultEnvelope<'a> {
    #[serde(borrow)]
    result: &'a RawValue,
}
pub fn result_envelope(bytes: &[u8]) -> bool {
    serde_json::from_slice::<ResultEnvelope<'_>>(bytes)
        .is_ok_and(|v| v.result.get().len() <= crate::limits::PORT_VALUE_BYTES)
}

/// Validate the shared HTTP envelope, not Provider authorization or request schemas.
/// The schema document itself remains raw so numeric schema data is never rounded.
pub fn envelope(bytes: &[u8], definition: &str) -> bool {
    use std::{collections::BTreeMap, sync::LazyLock};
    static VALIDATORS: LazyLock<BTreeMap<String, jsonschema::Validator>> = LazyLock::new(|| {
        let schema: Value =
            serde_json::from_str(include_str!("../../../contracts/http-v1.schema.json"))
                .expect("HTTP contract");
        ["manifest", "contracts", "schema", "guide"]
            .into_iter()
            .map(|name| {
                let wrapper = json!({"$ref": format!("#/$defs/{name}"), "$defs":schema["$defs"]});
                (
                    name.to_owned(),
                    jsonschema::validator_for(&wrapper).expect("HTTP envelope schema"),
                )
            })
            .collect()
    });
    let Ok(fields) = serde_json::from_slice::<BTreeMap<String, &RawValue>>(bytes) else {
        return false;
    };
    let mut value = serde_json::Map::new();
    for (key, raw) in fields {
        let field = if definition == "schema" && key == "schema" {
            if !raw.get().starts_with('{') {
                return false;
            }
            json!({})
        } else {
            let Ok(v) = serde_json::from_str(raw.get()) else {
                return false;
            };
            v
        };
        value.insert(key, field);
    }
    VALIDATORS
        .get(definition)
        .is_some_and(|v| v.is_valid(&Value::Object(value)))
}

fn unique_keys(raw: &RawValue) -> bool {
    use serde::de::{Deserializer, MapAccess, Visitor};
    struct Object;
    impl<'de> Visitor<'de> for Object {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique JSON keys")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            let mut keys = std::collections::BTreeSet::new();
            while let Some(key) = map.next_key::<String>()? {
                if !keys.insert(key) || !unique_keys(map.next_value::<&RawValue>()?) {
                    return Err(serde::de::Error::custom("duplicate JSON key"));
                }
            }
            Ok(())
        }
    }
    match raw.get().as_bytes().first() {
        Some(b'{') => serde_json::Deserializer::from_str(raw.get())
            .deserialize_map(Object)
            .is_ok(),
        Some(b'[') => serde_json::from_str::<Vec<&RawValue>>(raw.get())
            .is_ok_and(|items| items.into_iter().all(unique_keys)),
        _ => true,
    }
}
