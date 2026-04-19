//! Two-column "key: value" formatter used by lscpu, hwclock -v, etc.
//! The reference util-linux binaries align values to a fixed column.

/// Print a list of (label, value) rows, aligning all values to the
/// column after the widest label + ": ". Mimics util-linux `lscpu`'s
/// default unstructured output.
pub fn print_kv(rows: &[(String, String)]) {
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (k, v) in rows {
        println!("{:<width$}   {}", format!("{}:", k), v, width = width + 1);
    }
}
