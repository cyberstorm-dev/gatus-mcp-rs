use gatus_mcp_rs::client::{EndpointStatus, HealthResult};

#[test]
fn test_display_status_manual() {
    let ep = EndpointStatus {
        name: "svc".to_string(),
        group: "grp".to_string(),
        status: Some("DEGRADED".to_string()),
        results: vec![],
        events: vec![],
    };
    assert_eq!(ep.display_status(), "DEGRADED");
}

#[test]
fn test_display_status_from_results_up() {
    let ep = EndpointStatus {
        name: "svc".to_string(),
        group: "grp".to_string(),
        status: None,
        results: vec![HealthResult {
            timestamp: "ts".to_string(),
            success: true,
            hostname: None,
            ip: None,
            duration: 0,
            errors: vec![],
            status: None,
            condition_results: vec![],
            body: None,
            headers: None,
            certificate_expiration: None,
            certificate_issuer: None,
            certificate_algorithm: None,
            certificate_sans: None,
        }],
        events: vec![],
    };
    assert_eq!(ep.display_status(), "UP");
}

#[test]
fn test_display_status_from_results_down() {
    let ep = EndpointStatus {
        name: "svc".to_string(),
        group: "grp".to_string(),
        status: None,
        results: vec![HealthResult {
            timestamp: "ts".to_string(),
            success: false,
            hostname: None,
            ip: None,
            duration: 0,
            errors: vec![],
            status: None,
            condition_results: vec![],
            body: None,
            headers: None,
            certificate_expiration: None,
            certificate_issuer: None,
            certificate_algorithm: None,
            certificate_sans: None,
        }],
        events: vec![],
    };
    assert_eq!(ep.display_status(), "DOWN");
}

#[test]
fn test_display_status_uses_newest_gatus_result() {
    let result = |timestamp: &str, success| HealthResult {
        timestamp: timestamp.to_string(),
        success,
        hostname: None,
        ip: None,
        duration: 0,
        errors: vec![],
        status: None,
        condition_results: vec![],
        body: None,
        headers: None,
        certificate_expiration: None,
        certificate_issuer: None,
        certificate_algorithm: None,
        certificate_sans: None,
    };
    let ep = EndpointStatus {
        name: "svc".to_string(),
        group: "grp".to_string(),
        status: None,
        // Gatus returns status history oldest-first.
        results: vec![
            result("2026-08-20T13:46:19Z", false),
            result("2026-08-20T14:30:45Z", true),
        ],
        events: vec![],
    };

    assert_eq!(ep.display_status(), "UP");
    assert!(ep.latest_result().is_some_and(|item| item.success));
}

#[test]
fn test_display_status_uses_api_order_when_timestamp_is_malformed() {
    let result = |timestamp: &str, success| HealthResult {
        timestamp: timestamp.to_string(),
        success,
        hostname: None,
        ip: None,
        duration: 0,
        errors: vec![],
        status: None,
        condition_results: vec![],
        body: None,
        headers: None,
        certificate_expiration: None,
        certificate_issuer: None,
        certificate_algorithm: None,
        certificate_sans: None,
    };
    let ep = EndpointStatus {
        name: "svc".to_string(),
        group: "grp".to_string(),
        status: None,
        results: vec![
            result("2026-08-20T14:30:45Z", false),
            result("not-a-timestamp", true),
        ],
        events: vec![],
    };

    assert_eq!(ep.display_status(), "UP");
}

#[test]
fn test_calculate_uptime_different_timeframes() {
    let now = chrono::Utc::now();
    let ts_1h = (now - chrono::Duration::minutes(30)).to_rfc3339();
    let ts_12h = (now - chrono::Duration::hours(12)).to_rfc3339();
    let ts_2d = (now - chrono::Duration::days(2)).to_rfc3339();
    let ts_10d = (now - chrono::Duration::days(10)).to_rfc3339();

    let ep = EndpointStatus {
        name: "svc".to_string(),
        group: "grp".to_string(),
        status: None,
        results: vec![
            HealthResult {
                timestamp: ts_1h,
                success: true,
                hostname: None,
                ip: None,
                duration: 0,
                errors: vec![],
                status: None,
                condition_results: vec![],
                body: None,
                headers: None,
                certificate_expiration: None,
                certificate_issuer: None,
                certificate_algorithm: None,
                certificate_sans: None,
            },
            HealthResult {
                timestamp: ts_12h,
                success: false,
                hostname: None,
                ip: None,
                duration: 0,
                errors: vec![],
                status: None,
                condition_results: vec![],
                body: None,
                headers: None,
                certificate_expiration: None,
                certificate_issuer: None,
                certificate_algorithm: None,
                certificate_sans: None,
            },
            HealthResult {
                timestamp: ts_2d,
                success: true,
                hostname: None,
                ip: None,
                duration: 0,
                errors: vec![],
                status: None,
                condition_results: vec![],
                body: None,
                headers: None,
                certificate_expiration: None,
                certificate_issuer: None,
                certificate_algorithm: None,
                certificate_sans: None,
            },
            HealthResult {
                timestamp: ts_10d,
                success: false,
                hostname: None,
                ip: None,
                duration: 0,
                errors: vec![],
                status: None,
                condition_results: vec![],
                body: None,
                headers: None,
                certificate_expiration: None,
                certificate_issuer: None,
                certificate_algorithm: None,
                certificate_sans: None,
            },
        ],
        events: vec![],
    };

    // 24h: ts_1h (true), ts_12h (false) -> 50%
    assert_eq!(ep.calculate_uptime("24h"), 50.0);

    // 7d: ts_1h (true), ts_12h (false), ts_2d (true) -> 2/3 = 66.66...
    assert!((ep.calculate_uptime("7d") - 66.666).abs() < 0.1);

    // 30d: all 4 -> 2/4 = 50%
    assert_eq!(ep.calculate_uptime("30d"), 50.0);
}

#[test]
fn test_calculate_uptime_invalid_timestamp() {
    let ep = EndpointStatus {
        name: "svc".to_string(),
        group: "grp".to_string(),
        status: None,
        results: vec![HealthResult {
            timestamp: "invalid".to_string(),
            success: true,
            hostname: None,
            ip: None,
            duration: 0,
            errors: vec![],
            status: None,
            condition_results: vec![],
            body: None,
            headers: None,
            certificate_expiration: None,
            certificate_issuer: None,
            certificate_algorithm: None,
            certificate_sans: None,
        }],
        events: vec![],
    };
    // Should skip invalid timestamp and return 100.0 (no results filtered)
    assert_eq!(ep.calculate_uptime("24h"), 100.0);
}
