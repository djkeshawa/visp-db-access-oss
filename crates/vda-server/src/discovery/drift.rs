use serde_json::Value;

/// Connection drift between an inventory payload and its imported cluster.
/// Deletion drift is applied in SQL when a resource disappears from a scan.
pub(super) fn compare(resource: &Value, cluster: Option<&Value>) -> Vec<&'static str> {
    let Some(cluster) = cluster else {
        return Vec::new();
    };
    let mut drift = Vec::new();
    if resource.get("host") != cluster.get("host") || resource.get("port") != cluster.get("port") {
        drift.push("endpoint_changed");
    }
    if resource.get("replica_host") != cluster.get("replica_host")
        || resource.get("replica_port") != cluster.get("replica_port")
    {
        drift.push("replica_changed");
    }
    if resource.get("engine") != cluster.get("engine") {
        drift.push("engine_changed");
    }
    drift
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reports_each_connection_change() {
        let c = json!({"host":"a","port":5432,"replica_host":null,"replica_port":null,"engine":"postgres"});
        assert!(compare(&c, Some(&c)).is_empty());
        let r = json!({"host":"b","port":5432,"replica_host":"reader","replica_port":5432,"engine":"mysql"});
        assert_eq!(
            compare(&r, Some(&c)),
            vec!["endpoint_changed", "replica_changed", "engine_changed"]
        );
        assert!(compare(&r, None).is_empty());
    }
}
