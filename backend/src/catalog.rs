//! Bounded Person metadata and ordinary queries. No known-Provider allowlist or all-schema cache.
use crate::error;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use teamarness_package_contract::port_contract::{CompiledPortContracts, PortKind};
use teamarness_package_sdk::{
    BackendError,
    ports_v3::{
        ContractSummaryV3, GuideEligibility, PortAccessV3, PortClientV3, PortContractRef,
        PortPageV3,
    },
};

pub const CONVERSATIONS: &str = "dev.teamarness.conversations";
pub struct Catalog {
    client: PortClientV3,
}
pub struct Observed {
    pub summary: ContractSummaryV3,
    pub document: CompiledPortContracts,
}
impl Observed {
    pub fn route(&self) -> PortContractRef<'_> {
        route(&self.summary)
    }
    pub fn ordinary(&self, port: &str) -> bool {
        self.document.contracts[0].ports.iter().any(|p| {
            p.name == port && p.kind == PortKind::Query && p.response.is_some() && p.event.is_none()
        })
    }
    pub fn fingerprint(&self, port: &str) -> Result<String, BackendError> {
        let projection = self
            .document
            .project_metadata(
                &self.summary.contract,
                Some(&BTreeSet::from([port.to_owned()])),
            )
            .map_err(|_| error("forbidden"))?;
        let v: Value =
            serde_json::from_slice(&projection.bytes).map_err(|_| error("unavailable"))?;
        let p = &v["contracts"][0]["ports"][0];
        let mut schemas = json!({"kind":p["kind"],"request":p["request"],"response":p["response"],"event":p["event"],"$defs":v["$defs"]});
        for key in ["request", "response", "event"] {
            strip_descriptions(&mut schemas[key]);
        }
        if let Some(definitions) = schemas["$defs"].as_object_mut() {
            for schema in definitions.values_mut() {
                strip_descriptions(schema);
            }
        }
        Ok(
            Sha256::digest(serde_json::to_vec(&schemas).map_err(|_| error("unavailable"))?)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        )
    }
}
fn strip_descriptions(value: &mut Value) {
    let Some(schema) = value.as_object_mut() else {
        return;
    };
    schema.remove("description");
    for (key, child) in schema {
        match key.as_str() {
            "properties" | "patternProperties" | "$defs" | "dependentSchemas" => {
                if let Some(map) = child.as_object_mut() {
                    for sub in map.values_mut() {
                        strip_descriptions(sub);
                    }
                }
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                if let Some(list) = child.as_array_mut() {
                    for sub in list {
                        strip_descriptions(sub);
                    }
                }
            }
            "items"
            | "contains"
            | "additionalProperties"
            | "unevaluatedProperties"
            | "unevaluatedItems"
            | "propertyNames"
            | "not"
            | "if"
            | "then"
            | "else" => strip_descriptions(child),
            _ => {} // enum/const/default are data, not schemas.
        }
    }
}
pub fn excluded(row: &ContractSummaryV3) -> bool {
    row.contract == CONVERSATIONS
        || row
            .provider
            .as_ref()
            .is_some_and(|p| p.identity == CONVERSATIONS)
}
fn route(row: &ContractSummaryV3) -> PortContractRef<'_> {
    PortContractRef {
        access: PortAccessV3::Person,
        contract: &row.contract,
        major: row.major,
        route_revision: row.route_revision.as_deref(),
    }
}
impl Catalog {
    pub fn new(client: PortClientV3) -> Self {
        Self { client }
    }
    pub async fn page(&self, cursor: Option<&str>) -> Result<PortPageV3, BackendError> {
        self.client.available(PortAccessV3::Person, cursor).await
    }
    pub async fn observe(&self, contract: &str) -> Result<Observed, BackendError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(crate::limits::CATALOG_READ_TIMEOUT as u64),
            self.observe_bounded(contract),
        )
        .await
        .map_err(|_| error("timeout"))?
    }
    async fn observe_bounded(&self, contract: &str) -> Result<Observed, BackendError> {
        // One stale-cursor restart. Pages are consumed and dropped; never collect the catalog.
        let mut cursor: Option<String> = None;
        let mut restarted = false;
        loop {
            let page = match self.page(cursor.as_deref()).await {
                Err(e) if e.code() == "port_catalog_changed" && !restarted => {
                    cursor = None;
                    restarted = true;
                    continue;
                }
                other => other?,
            };
            if let Some(summary) = page.contracts.into_iter().find(|r| r.contract == contract) {
                if excluded(&summary)
                    || summary.guide_eligibility != GuideEligibility::Eligible
                    || summary.route_revision.is_none()
                    || summary.provider.is_none()
                {
                    return Err(error("forbidden"));
                }
                let digest = summary
                    .metadata_sha256
                    .as_deref()
                    .ok_or_else(|| error("unavailable"))?;
                let document = self.client.describe(route(&summary), digest).await?;
                if !document.has_guide(contract) {
                    return Err(error("forbidden"));
                }
                return Ok(Observed { summary, document });
            }
            match page.next_cursor {
                Some(next) if cursor.as_ref() != Some(&next) => cursor = Some(next),
                Some(_) => return Err(error("catalog_changed")),
                None => return Err(error("not_found")),
            }
        }
    }
    pub async fn enable(&self, observed: &Observed) -> Result<(), BackendError> {
        self.client.enable(observed.route()).await
    }
    pub async fn disable(&self, contract: &str) -> Result<(), BackendError> {
        self.client.disable(PortAccessV3::Person, contract).await
    }
    pub async fn query(
        &self,
        observed: &Observed,
        port: &str,
        request: Value,
    ) -> Result<Value, BackendError> {
        if !observed.ordinary(port) || excluded(&observed.summary) {
            return Err(error("forbidden"));
        }
        self.client.query(observed.route(), port, request).await
    }
}
