//! Canonical current roster at the existing SDK capability seam.
use serde_json::json;
use teamarness_package_sdk::testing::ScriptedCapability;
pub fn present() -> ScriptedCapability {
    ScriptedCapability::new().calls(|method, _| {
        assert_eq!(method, "current");
        Ok(json!({"workspaceId":"workspace1","epoch":"1","membershipVersion":"a".repeat(64),"members":[{"identity":"b".repeat(64),"role":"member"}]}))
    })
}
