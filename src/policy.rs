use crate::{config::PolicyConfig, model::*};
use std::collections::BTreeSet;

/// Unavailable prerequisites are separate from evaluated violations.
pub fn evaluate(report: &mut ScanReport, config: &PolicyConfig, github_requested: bool) -> bool {
    let mut complete = true;
    if let Some(max) = config.max_total
        && report.findings.len() > max
    {
        report.violations.push(PolicyViolation {
            policy: "max_total".into(),
            message: format!("{} findings exceeds maximum {max}", report.findings.len()),
        });
    }
    for (marker, max) in &config.max_by_marker {
        let actual = report.counts.get(marker).copied().unwrap_or(0);
        if actual > *max {
            report.violations.push(PolicyViolation {
                policy: "max_by_marker".into(),
                message: format!("{marker}: {actual} exceeds maximum {max}"),
            });
        }
    }
    let mut skipped = BTreeSet::new();
    for finding in &report.findings {
        let location = format!("{}:{}", finding.path, finding.line);
        if config.require_references.contains(&finding.marker) && finding.references.is_empty() {
            report.violations.push(PolicyViolation {
                policy: "require_references".into(),
                message: format!("{location}: {} requires an issue reference", finding.marker),
            });
        }
        if config.needs_github() {
            for reference in &finding.references {
                match reference.status {
                    ReferenceStatus::Closed if config.fail_on_closed => {
                        report.violations.push(PolicyViolation {
                            policy: "fail_on_closed".into(),
                            message: format!(
                                "{location}: {} references a closed item",
                                reference.raw
                            ),
                        })
                    }
                    ReferenceStatus::Unavailable if config.fail_on_unavailable => {
                        report.violations.push(PolicyViolation {
                            policy: "fail_on_unavailable".into(),
                            message: format!("{location}: {} is unavailable", reference.raw),
                        })
                    }
                    ReferenceStatus::Error
                    | ReferenceStatus::Unresolved
                    | ReferenceStatus::NotRequested => {
                        complete = false;
                        skipped.insert(reference.key().unwrap_or_else(|| reference.raw.clone()));
                    }
                    _ => {}
                }
            }
        }
    }
    for reference in skipped {
        report.warn(
            "policy",
            format!(
                "issue-state policies skipped for {reference}: {}",
                if github_requested {
                    "enrichment did not complete"
                } else {
                    "GitHub enrichment is disabled"
                }
            ),
        );
    }
    complete
}
