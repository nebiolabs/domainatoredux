//! Applying app-level defaults to a tool's parameters before it runs.
//!
//! The one rule that makes this non-trivial: `cpu` is a config property on only
//! some tools, and every config struct in `domainator-config` is
//! `deny_unknown_fields`. Setting `cpu` on a tool whose schema lacks it does not
//! get ignored — it makes the whole run be **rejected**. So the schema decides.

use serde_json::Value;

/// Apply the app's default thread count to `params`.
///
/// No-op when: the default is `0` (meaning "all cores", already the tool's own
/// default), the tool's schema has no `cpu` property, the caller has set `cpu`
/// explicitly, or `params` is not an object.
pub fn apply_default_cpu(params: &mut Value, schema: &Value, default_cpu: usize) {
    if default_cpu == 0 {
        return;
    }
    if !schema
        .get("properties")
        .and_then(|p| p.get("cpu"))
        .is_some()
    {
        return;
    }
    let Some(obj) = params.as_object_mut() else {
        return;
    };
    // An explicit value in the dialog always wins over the preference.
    if obj.contains_key("cpu") {
        return;
    }
    obj.insert("cpu".into(), Value::from(default_cpu));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn with_cpu() -> Value {
        json!({ "properties": { "cpu": { "type": "integer" }, "input": { "type": "string" } } })
    }
    fn without_cpu() -> Value {
        json!({ "properties": { "input": { "type": "string" } } })
    }

    #[test]
    fn injects_when_the_schema_has_cpu() {
        let mut p = json!({ "input": "a.gb" });
        apply_default_cpu(&mut p, &with_cpu(), 4);
        assert_eq!(p["cpu"], json!(4));
    }

    /// The failure this whole module exists to prevent: every config struct is
    /// `deny_unknown_fields`, so an injected `cpu` on a tool that has no such
    /// property would make the run be rejected outright.
    #[test]
    fn never_injects_when_the_schema_lacks_cpu() {
        let mut p = json!({ "input": "a.gb" });
        apply_default_cpu(&mut p, &without_cpu(), 4);
        assert_eq!(p, json!({ "input": "a.gb" }));
    }

    #[test]
    fn an_explicit_value_wins() {
        let mut p = json!({ "cpu": 1 });
        apply_default_cpu(&mut p, &with_cpu(), 16);
        assert_eq!(p["cpu"], json!(1));
    }

    #[test]
    fn zero_means_leave_it_alone() {
        let mut p = json!({});
        apply_default_cpu(&mut p, &with_cpu(), 0);
        assert!(p.get("cpu").is_none(), "0 already means all cores");
    }

    #[test]
    fn a_non_object_is_left_alone() {
        let mut p = json!("not params");
        apply_default_cpu(&mut p, &with_cpu(), 4);
        assert_eq!(p, json!("not params"));
    }

    #[test]
    fn a_schema_without_properties_is_left_alone() {
        let mut p = json!({});
        apply_default_cpu(&mut p, &json!({}), 4);
        assert!(p.get("cpu").is_none());
    }
}
