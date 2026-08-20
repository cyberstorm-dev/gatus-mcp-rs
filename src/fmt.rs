use crate::client::{
    AlertRule, CertificateAudit, CorrelatedEvent, DiagnosticBundle, EndpointStatus,
    ExpiringCertificate, FailureSummary, FlappingService, GroupStats, PerformanceComparison, Suite,
    SuiteHealth, SystemStats,
};

pub fn format_suite_health(health: &SuiteHealth) -> String {
    let mut output = format!("### Suite Health: {} ({})\n\n", health.name, health.id);

    let total = health.up + health.down + health.degraded;
    output.push_str(&format!("- **Total Endpoints:** {}\n", total));
    output.push_str(&format!("- **UP:** {}\n", health.up));
    output.push_str(&format!("- **DOWN:** {}\n", health.down));
    output.push_str(&format!("- **DEGRADED:** {}\n", health.degraded));

    if total > 0 {
        let health_percentage = (health.up as f64 / total as f64) * 100.0;
        output.push_str(&format!(
            "- **Health Percentage:** {:.2}%\n",
            health_percentage
        ));
    } else {
        output.push_str("- **Health Percentage:** 100.00%\n");
    }

    output
}

pub fn format_certificate_audit(audit: &CertificateAudit) -> String {
    let mut output = format!(
        "### Certificate Audit: {} ({})\n\n",
        audit.name, audit.group
    );

    output.push_str(&format!(
        "- **Issuer:** {}\n",
        audit.issuer.as_deref().unwrap_or("-")
    ));
    output.push_str(&format!(
        "- **Algorithm:** {}\n",
        audit.algorithm.as_deref().unwrap_or("-")
    ));

    if let Some(exp) = audit.expiration {
        let days = exp / (24 * 60 * 60 * 1_000_000_000);
        output.push_str(&format!("- **Expiration:** {} days remaining\n", days));
    } else {
        output.push_str("- **Expiration:** -\n");
    }

    if !audit.sans.is_empty() {
        output.push_str("- **Subject Alternative Names (SANs):**\n");
        for san in &audit.sans {
            output.push_str(&format!("  - {}\n", san));
        }
    } else {
        output.push_str("- **Subject Alternative Names (SANs):** -\n");
    }

    output
}

pub fn format_diagnostic_bundle(bundle: &DiagnosticBundle) -> String {
    let mut output = format!(
        "### Diagnostic Bundle: {} ({})\n\n",
        bundle.name, bundle.group
    );

    // 1. Failure Summary
    if !bundle.failure_summary.failed_conditions.is_empty() {
        output.push_str("#### ❌ Failed Conditions\n");
        for condition in &bundle.failure_summary.failed_conditions {
            output.push_str(&format!("- {}\n", condition));
        }
        output.push('\n');
    }

    if !bundle.failure_summary.passed_conditions.is_empty() {
        output.push_str("#### ✅ Passed Conditions\n");
        for condition in &bundle.failure_summary.passed_conditions {
            output.push_str(&format!("- {}\n", condition));
        }
        output.push('\n');
    }

    // 2. Recent Results (Latest 5)
    output.push_str("#### 📊 Recent Results\n\n");
    output.push_str("| Timestamp | Success | Duration | Errors |\n");
    output.push_str("| :--- | :--- | :--- | :--- |\n");
    for result in bundle.results.iter().take(5) {
        let success = if result.success { "✅" } else { "❌" };
        let errors = if result.errors.is_empty() {
            "-"
        } else {
            &result.errors[0]
        };
        output.push_str(&format!(
            "| {} | {} | {}ms | {} |\n",
            result.timestamp,
            success,
            result.duration / 1_000_000,
            errors
        ));
    }
    output.push('\n');

    // 3. Alert Events (Latest 5)
    if !bundle.alert_events.is_empty() {
        output.push_str("#### 🔔 Recent Alert Events\n\n");
        output.push_str("| Timestamp | Type |\n");
        output.push_str("| :--- | :--- |\n");
        for event in bundle.alert_events.iter().take(5) {
            output.push_str(&format!("| {} | {} |\n", event.timestamp, event.event_type));
        }
    }

    output
}

pub fn format_alert_rules(rules: &[AlertRule]) -> String {
    let mut output = String::from("### Gatus Alerting Rules\n\n");
    output.push_str("| Endpoint | Group | Type | Status | Thresholds (F/S) | Description |\n");
    output.push_str("| :--- | :--- | :--- | :--- | :--- | :--- |\n");

    for rule in rules {
        let status = if rule.enabled { "✅" } else { "❌" };
        let thresholds = format!("{} / {}", rule.failure_threshold, rule.success_threshold);
        let description = rule.description.as_deref().unwrap_or("-");
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            rule.endpoint, rule.group, rule.alert_type, status, thresholds, description
        ));
    }
    output
}

pub fn format_suites(pages: &[Suite]) -> String {
    let mut output = String::from("### Gatus Suites\n\n");
    output.push_str("| ID | Name |\n");
    output.push_str("| :--- | :--- |\n");

    for page in pages {
        output.push_str(&format!("| {} | {} |\n", page.id, page.name));
    }
    output
}

pub fn format_endpoint_status(endpoint: &EndpointStatus, badge_url: Option<&str>) -> String {
    let mut output = format!("### {}\n", endpoint.name);
    if let Some(url) = badge_url {
        output.push_str(&format!("![Health Badge]({})\n\n", url));
    }
    output.push_str(&format!("- **Group:** {}\n", endpoint.group));
    output.push_str(&format!("- **Status:** {}\n", endpoint.display_status()));

    if let Some(result) = endpoint.latest_result() {
        output.push_str("\n#### Latest Result\n");
        output.push_str(&format!("- **Timestamp:** {}\n", result.timestamp));
        output.push_str(&format!("- **Success:** {}\n", result.success));
        output.push_str(&format!(
            "- **Duration:** {}ms\n",
            result.duration / 1_000_000
        ));

        if let Some(exp) = result.certificate_expiration {
            let days = exp / (24 * 60 * 60 * 1_000_000_000);
            output.push_str(&format!("- **SSL Expiration:** {} days remaining\n", days));
        }

        if !result.errors.is_empty() {
            output.push_str("- **Errors:**\n");
            for error in &result.errors {
                output.push_str(&format!("  - {}\n", error));
            }
        }

        if !result.success {
            if result.headers.is_some() {
                output.push_str("- **Headers:** (present)\n");
            }
            if let Some(ref body) = result.body {
                let snippet = if body.len() > 100 {
                    format!("{}...", &body[..100])
                } else {
                    body.clone()
                };
                output.push_str(&format!("- **Body Snippet:** {}\n", snippet));
            }
        }

        if !result.condition_results.is_empty() {
            output.push_str("\n#### Conditions\n");
            for condition in &result.condition_results {
                let status = if condition.success { "✅" } else { "❌" };
                output.push_str(&format!("- {} {}\n", status, condition.condition));
            }
        }
    }

    output
}

pub fn format_endpoints_summary(endpoints: &[EndpointStatus]) -> String {
    let mut output = String::from("| Service | Group | Status | Latest Result |\n");
    output.push_str("| :--- | :--- | :--- | :--- |\n");

    for endpoint in endpoints {
        let latest = match endpoint.latest_result() {
            Some(r) => {
                if r.success {
                    "✅"
                } else {
                    "❌"
                }
            }
            None => "❓",
        };
        output.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            endpoint.name,
            endpoint.group,
            endpoint.display_status(),
            latest
        ));
    }

    output
}

pub fn format_system_stats(stats: &SystemStats) -> String {
    let mut output = String::from("### Gatus System Health Summary\n\n");
    output.push_str(&format!("- **Total Endpoints:** {}\n", stats.total));
    output.push_str(&format!("- **UP:** {}\n", stats.up));
    output.push_str(&format!("- **DOWN:** {}\n", stats.down));
    output.push_str(&format!("- **DEGRADED:** {}\n", stats.degraded));
    output.push_str(&format!(
        "- **Certificates Expiring Soon:** {}\n",
        stats.certificates_expiring_soon
    ));
    output
}

pub fn format_config_summary(endpoints: &[EndpointStatus]) -> String {
    let mut output = String::from("### Gatus Endpoint Configurations\n\n");
    for endpoint in endpoints {
        output.push_str(&format!(
            "#### {} (Group: {})\n",
            endpoint.name, endpoint.group
        ));
        if let Some(result) = endpoint.latest_result() {
            if !result.condition_results.is_empty() {
                output.push_str("- **Conditions:**\n");
                for condition in &result.condition_results {
                    let status = if condition.success { "✅" } else { "❌" };
                    output.push_str(&format!("  - {} {}\n", condition.condition, status));
                }
            }
        }
        output.push('\n');
    }
    output
}

pub fn format_expiring_certificates(certs: &[ExpiringCertificate]) -> String {
    let mut output = String::from("| Service | Group | Days Remaining | Expiration |\n");
    output.push_str("| :--- | :--- | :--- | :--- |\n");

    for cert in certs {
        output.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            cert.name, cert.group, cert.days_remaining, cert.expiration
        ));
    }
    output
}

pub fn format_failure_summary(summary: &FailureSummary) -> String {
    let mut output = format!(
        "### Failure Summary for {} ({})\n\n",
        summary.name, summary.group
    );

    if !summary.failed_conditions.is_empty() {
        output.push_str("#### ❌ Failed Conditions\n");
        for condition in &summary.failed_conditions {
            output.push_str(&format!("- {}\n", condition));
        }
    }

    if !summary.passed_conditions.is_empty() {
        output.push_str("\n#### ✅ Passed Conditions\n");
        for condition in &summary.passed_conditions {
            output.push_str(&format!("- {}\n", condition));
        }
    }

    output
}

pub fn format_group_stats(stats: &GroupStats) -> String {
    let mut output = format!("### Group Health: {}\n\n", stats.group);
    output.push_str(&format!("- **Total Endpoints:** {}\n", stats.total));
    output.push_str(&format!("- **UP:** {}\n", stats.up));
    output.push_str(&format!("- **DOWN:** {}\n", stats.down));
    output.push_str(&format!("- **DEGRADED:** {}\n", stats.degraded));

    let health_percentage = (stats.up as f64 / stats.total as f64) * 100.0;
    output.push_str(&format!(
        "- **Health Percentage:** {:.2}%\n",
        health_percentage
    ));

    output
}

pub fn format_alert_correlation(events: &[CorrelatedEvent]) -> String {
    let mut output = String::from("### Notification Correlation Timeline\n\n");
    output.push_str("| Timestamp | Type | Description |\n");
    output.push_str("| :--- | :--- | :--- |\n");

    for event in events {
        let type_icon = if event.event_type == "alert" {
            "🔔"
        } else {
            "📊"
        };
        output.push_str(&format!(
            "| {} | {} {} | {} |\n",
            event.timestamp, type_icon, event.event_type, event.description
        ));
    }
    output
}

pub fn format_flapping_services(services: &[FlappingService]) -> String {
    let mut output = String::from("### Services with Failures (Flapping)\n\n");
    output.push_str("| Service | Group | Failures | Successes |\n");
    output.push_str("| :--- | :--- | :--- | :--- |\n");

    for service in services {
        output.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            service.name, service.group, service.failure_count, service.success_count
        ));
    }
    output
}

pub fn format_performance_comparison(comparison: &PerformanceComparison) -> String {
    let mut output = format!("### Performance Comparison for {}\n\n", comparison.key);
    output.push_str(&format!(
        "- **Avg Latency (1h):** {:.2}ms\n",
        comparison.avg_1h
    ));
    output.push_str(&format!(
        "- **Avg Latency (7d):** {:.2}ms\n",
        comparison.avg_7d
    ));

    let sign = if comparison.delta_percentage > 0.0 {
        "+"
    } else {
        ""
    };
    output.push_str(&format!(
        "- **Delta:** {}{:.2}%\n",
        sign, comparison.delta_percentage
    ));

    if comparison.delta_percentage > 20.0 {
        output.push_str("\n> ⚠️ **Warning:** Significant latency regression detected!");
    }

    output
}
