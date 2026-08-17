//! Record keys: the schema's own field names, borrowed rather than rebuilt.

use crate::state_schema::vcs_state_schema;

/// Resolve the `<prefix>_<suffix>` schema name, borrowing the schema's static
/// name so a record key needs no allocation.
pub(super) fn field(prefix: &str, suffix: &str) -> &'static str {
    vcs_state_schema()
        .fields
        .iter()
        .map(|f| f.name)
        .find(|name| {
            name.strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('_'))
                .is_some_and(|rest| rest == suffix)
        })
        .unwrap_or_else(|| unreachable!("no schema field {prefix}_{suffix}"))
}

/// A per-object field of one motion group; the ball is the fallback object.
pub(super) fn object_field(group: &str, obj: &str) -> &'static str {
    let obj = if matches!(obj, "p0" | "p1" | "m0" | "m1") {
        obj
    } else {
        "bl"
    };
    field(group, obj)
}
