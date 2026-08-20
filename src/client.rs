use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use governor::{Quota, RateLimiter};
use moka::future::Cache;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EndpointStatus {
    pub name: String,
    pub group: String,
    pub status: Option<String>,
    #[serde(default)]
    pub results: Vec<HealthResult>,
    #[serde(default)]
    pub events: Vec<Event>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Event {
    #[serde(rename = "type")]
    pub event_type: String,
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AlertEvent {
    pub service: String,
    pub group: String,
    pub event_type: String,
    pub timestamp: String,
}

impl EndpointStatus {
    pub fn get_key(&self) -> String {
        format!(
            "{}_{}",
            self.sanitize_field(&self.group),
            self.sanitize_field(&self.name)
        )
    }

    fn sanitize_field(&self, input: &str) -> String {
        let mut output = String::with_capacity(input.len());
        for c in input.chars() {
            if c.is_alphanumeric() || c == '_' {
                output.push(c);
            } else {
                output.push('-');
            }
        }
        output
    }

    pub fn display_status(&self) -> String {
        self.status
            .clone()
            .unwrap_or_else(|| match self.latest_result() {
                Some(r) => {
                    if r.success {
                        "UP".to_string()
                    } else {
                        "DOWN".to_string()
                    }
                }
                None => "UNKNOWN".to_string(),
            })
    }

    /// Gatus returns historical results in ascending timestamp order.
    pub fn latest_result(&self) -> Option<&HealthResult> {
        self.results.last()
    }

    pub fn calculate_uptime(&self, timeframe: &str) -> f64 {
        if self.results.is_empty() {
            return 100.0;
        }

        let now = Utc::now();
        let duration = match timeframe {
            "7d" => ChronoDuration::days(7),
            "30d" => ChronoDuration::days(30),
            _ => ChronoDuration::hours(24),
        };
        let cutoff = now - duration;

        let filtered_results: Vec<&HealthResult> = self
            .results
            .iter()
            .filter(|r| {
                if let Ok(ts) = DateTime::parse_from_rfc3339(&r.timestamp) {
                    ts.with_timezone(&Utc) >= cutoff
                } else {
                    false
                }
            })
            .collect();

        if filtered_results.is_empty() {
            return 100.0;
        }

        let success_count = filtered_results.iter().filter(|r| r.success).count();
        (success_count as f64 / filtered_results.len() as f64) * 100.0
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct HealthResult {
    pub timestamp: String,
    pub success: bool,
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub duration: u64,
    #[serde(default)]
    pub errors: Vec<String>,
    pub status: Option<u16>,
    #[serde(rename = "conditionResults", default)]
    pub condition_results: Vec<ConditionResult>,
    pub body: Option<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
    #[serde(rename = "certificateExpiration", alias = "certificate_expiration")]
    pub certificate_expiration: Option<u64>,
    #[serde(rename = "certificateIssuer")]
    pub certificate_issuer: Option<String>,
    #[serde(rename = "certificateAlgorithm")]
    pub certificate_algorithm: Option<String>,
    #[serde(rename = "certificateSans")]
    pub certificate_sans: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConditionResult {
    pub condition: String,
    pub success: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ResponseTimePoint {
    pub timestamp: String,
    pub value: u64,
}

pub type UptimeResponse = std::collections::HashMap<String, f64>;
pub type ResponseTimeResponse = Vec<ResponseTimePoint>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExpiringCertificate {
    pub name: String,
    pub group: String,
    pub expiration: u64,
    pub days_remaining: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CertificateAudit {
    pub name: String,
    pub group: String,
    pub issuer: Option<String>,
    pub algorithm: Option<String>,
    pub sans: Vec<String>,
    pub expiration: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemStats {
    pub total: usize,
    pub up: usize,
    pub down: usize,
    pub degraded: usize,
    pub certificates_expiring_soon: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FailureSummary {
    pub name: String,
    pub group: String,
    pub failed_conditions: Vec<String>,
    pub passed_conditions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PerformanceComparison {
    pub key: String,
    pub avg_1h: f64,
    pub avg_7d: f64,
    pub delta_percentage: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GroupStats {
    pub group: String,
    pub total: usize,
    pub up: usize,
    pub down: usize,
    pub degraded: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FlappingService {
    pub name: String,
    pub group: String,
    pub failure_count: u64,
    pub success_count: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CorrelatedEvent {
    pub timestamp: String,
    pub event_type: String, // "result" or "alert"
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Suite {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SuiteHealth {
    pub id: String,
    pub name: String,
    pub up: usize,
    pub down: usize,
    pub degraded: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlertConfig {
    #[serde(rename = "type")]
    pub alert_type: String,
    pub enabled: bool,
    #[serde(rename = "failure-threshold")]
    pub failure_threshold: u32,
    #[serde(rename = "success-threshold")]
    pub success_threshold: u32,
    pub description: Option<String>,
    #[serde(rename = "send-on-resolved", default)]
    pub send_on_resolved: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlertRule {
    pub endpoint: String,
    pub group: String,
    #[serde(rename = "type")]
    pub alert_type: String,
    pub enabled: bool,
    pub failure_threshold: u32,
    pub success_threshold: u32,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EndpointConfig {
    pub name: String,
    pub group: Option<String>,
    pub url: String,
    pub interval: Option<String>,
    pub conditions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub alerts: Vec<AlertConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiagnosticBundle {
    pub name: String,
    pub group: String,
    pub results: Vec<HealthResult>,
    pub failure_summary: FailureSummary,
    pub alert_events: Vec<AlertEvent>,
}

#[derive(Clone)]
pub struct GatusClient {
    api_url: String,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
    client: Client,
    cache: Cache<String, Vec<EndpointStatus>>,
    uptime_cache: Cache<String, UptimeResponse>,
    response_time_cache: Cache<String, ResponseTimeResponse>,
    rate_limiter: Arc<
        RateLimiter<
            governor::state::NotKeyed,
            governor::state::InMemoryState,
            governor::clock::DefaultClock,
        >,
    >,
}

impl GatusClient {
    pub fn new(
        api_url: String,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(2).unwrap()); // 2 requests per second

        Self {
            api_url,
            api_key,
            username,
            password,
            client: Client::new(),
            cache: Cache::builder()
                .max_capacity(100)
                .time_to_live(Duration::from_secs(60))
                .build(),
            uptime_cache: Cache::builder()
                .max_capacity(100)
                .time_to_live(Duration::from_secs(60))
                .build(),
            response_time_cache: Cache::builder()
                .max_capacity(100)
                .time_to_live(Duration::from_secs(60))
                .build(),
            rate_limiter: Arc::new(RateLimiter::direct(quota)),
        }
    }

    fn add_auth_header(&self, mut request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        } else if let (Some(ref u), Some(ref p)) = (&self.username, &self.password) {
            request = request.basic_auth(u, Some(p));
        }
        request
    }

    pub fn sanitize_key(&self, input: &str) -> String {
        // Create a dummy endpoint to use its private sanitize_field
        let dummy = EndpointStatus {
            name: String::new(),
            group: String::new(),
            status: None,
            results: vec![],
            events: vec![],
        };
        dummy.sanitize_field(input)
    }

    #[tracing::instrument(skip(self))]
    pub async fn list_suites(&self) -> Result<Vec<Suite>> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/suites/statuses", self.api_url);
        let mut request = self.client.get(url);
        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        #[derive(Deserialize)]
        struct SuiteStatusResponse {
            #[serde(rename = "key")]
            id: String,
            name: String,
        }

        let suites_resp: Vec<SuiteStatusResponse> = serde_json::from_str(&text)?;
        let suites = suites_resp
            .into_iter()
            .map(|s| Suite {
                id: s.id,
                name: s.name,
            })
            .collect();
        Ok(suites)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_suite_health(&self, id: &str) -> Result<SuiteHealth> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/suites/{}/statuses", self.api_url, id);
        let mut request = self.client.get(url);
        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        #[derive(Deserialize)]
        struct SuiteResponse {
            #[serde(rename = "key")]
            id: String,
            name: String,
            results: Vec<SuiteResult>,
        }

        #[derive(Deserialize)]
        struct SuiteResult {
            #[serde(rename = "endpointResults")]
            endpoint_results: Vec<EndpointResult>,
        }

        #[derive(Deserialize)]
        struct EndpointResult {
            success: bool,
        }

        let suite_resp: SuiteResponse = serde_json::from_str(&text)?;

        let mut up = 0;
        let mut down = 0;
        let degraded = 0; // Gatus suites don't explicitly have degraded in results, usually just success/failure

        // Gatus returns historical suite results oldest-first.
        if let Some(latest) = suite_resp.results.last() {
            for res in &latest.endpoint_results {
                if res.success {
                    up += 1;
                } else {
                    down += 1;
                }
            }
        }

        Ok(SuiteHealth {
            id: suite_resp.id,
            name: suite_resp.name,
            up,
            down,
            degraded,
        })
    }

    #[tracing::instrument(skip(self, config))]
    pub async fn create_endpoint(&self, suite_id: &str, config: EndpointConfig) -> Result<()> {
        self.rate_limiter.until_ready().await;

        let url = format!(
            "{}/api/v1/external/status-pages/{}/endpoints",
            self.api_url, suite_id
        );
        let mut request = self.client.post(url).json(&config);
        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self, config))]
    pub async fn update_endpoint(
        &self,
        suite_id: &str,
        endpoint_id: &str,
        config: EndpointConfig,
    ) -> Result<()> {
        self.rate_limiter.until_ready().await;

        let url = format!(
            "{}/api/v1/external/status-pages/{}/endpoints/{}",
            self.api_url, suite_id, endpoint_id
        );
        let mut request = self.client.put(url).json(&config);
        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn delete_endpoint(&self, suite_id: &str, endpoint_id: &str) -> Result<()> {
        self.rate_limiter.until_ready().await;

        let url = format!(
            "{}/api/v1/external/status-pages/{}/endpoints/{}",
            self.api_url, suite_id, endpoint_id
        );
        let mut request = self.client.delete(url);
        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_endpoint_statuses(&self, key: &str) -> Result<Vec<EndpointStatus>> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/endpoints/{}/statuses", self.api_url, key);
        let mut request = self.client.get(url);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        let services: Vec<EndpointStatus> = serde_json::from_str(&text)?;
        Ok(services)
    }

    #[tracing::instrument(skip(self))]
    pub async fn list_services(
        &self,
        refresh: bool,
        status_filter: Option<&str>,
    ) -> Result<Vec<EndpointStatus>> {
        let cache_key = "endpoints_statuses".to_string();

        let all_services = if !refresh {
            if let Some(cached) = self.cache.get(&cache_key).await {
                cached
            } else {
                self.fetch_all_services(&cache_key).await?
            }
        } else {
            self.fetch_all_services(&cache_key).await?
        };

        if let Some(filter) = status_filter {
            let filter_upper = filter.to_uppercase();
            Ok(all_services
                .into_iter()
                .filter(|s| s.display_status().to_uppercase() == filter_upper)
                .collect())
        } else {
            Ok(all_services)
        }
    }

    async fn fetch_all_services(&self, cache_key: &str) -> Result<Vec<EndpointStatus>> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/endpoints/statuses", self.api_url);
        let mut request = self.client.get(url);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response
            .text()
            .await
            .context("Failed to get response text")?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        let services: Vec<EndpointStatus> = serde_json::from_str(&text)
            .with_context(|| format!("Failed to decode Gatus API response: {}", text))?;

        self.cache
            .insert(cache_key.to_string(), services.clone())
            .await;

        Ok(services)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_system_stats(&self) -> Result<SystemStats> {
        let services = self.list_services(false, None).await?;
        let mut up = 0;
        let mut down = 0;
        let mut degraded = 0;
        let mut certificates_expiring_soon = 0;

        // 30 days in nanoseconds
        let threshold = 30 * 24 * 60 * 60 * 1_000_000_000u64;

        for service in &services {
            let status = service.display_status().to_uppercase();
            match status.as_str() {
                "UP" => up += 1,
                "DOWN" => down += 1,
                "DEGRADED" => degraded += 1,
                _ => {
                    if let Some(result) = service.latest_result() {
                        if result.success {
                            up += 1;
                        } else {
                            down += 1;
                        }
                    }
                }
            }

            if let Some(result) = service.latest_result() {
                if let Some(exp) = result.certificate_expiration {
                    if exp < threshold {
                        certificates_expiring_soon += 1;
                    }
                }
            }
        }

        Ok(SystemStats {
            total: services.len(),
            up,
            down,
            degraded,
            certificates_expiring_soon,
        })
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_group_stats(&self, group_name: &str) -> Result<GroupStats> {
        let services = self.list_services(false, None).await?;
        let filtered: Vec<_> = services
            .into_iter()
            .filter(|s| s.group.to_lowercase() == group_name.to_lowercase())
            .collect();

        if filtered.is_empty() {
            anyhow::bail!("Group not found or empty: {}", group_name);
        }

        let mut up = 0;
        let mut down = 0;
        let mut degraded = 0;

        for service in &filtered {
            let status = service.display_status().to_uppercase();
            match status.as_str() {
                "UP" => up += 1,
                "DOWN" => down += 1,
                "DEGRADED" => degraded += 1,
                _ => {
                    if let Some(result) = service.latest_result() {
                        if result.success {
                            up += 1;
                        } else {
                            down += 1;
                        }
                    }
                }
            }
        }

        Ok(GroupStats {
            group: group_name.to_string(),
            total: filtered.len(),
            up,
            down,
            degraded,
        })
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_expiring_certificates(
        &self,
        threshold_days: u64,
    ) -> Result<Vec<ExpiringCertificate>> {
        let services = self.list_services(false, None).await?;
        let mut expiring = Vec::new();
        let threshold_ns = threshold_days * 24 * 60 * 60 * 1_000_000_000u64;

        for service in services {
            if let Some(result) = service.latest_result() {
                if let Some(exp) = result.certificate_expiration {
                    if exp < threshold_ns {
                        expiring.push(ExpiringCertificate {
                            name: service.name.clone(),
                            group: service.group.clone(),
                            expiration: exp,
                            days_remaining: (exp / (24 * 60 * 60 * 1_000_000_000)) as i64,
                        });
                    }
                }
            }
        }

        Ok(expiring)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_uptime(&self, service_name: &str, timeframe: &str) -> Result<f64> {
        let services = self.list_services(false, None).await?;
        let service = services
            .iter()
            .find(|s| s.name == service_name)
            .ok_or_else(|| anyhow::anyhow!("Service not found: {}", service_name))?;

        Ok(service.calculate_uptime(timeframe))
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_alert_history(&self, limit: usize) -> Result<Vec<AlertEvent>> {
        let services = self.list_services(false, None).await?;
        let mut all_events = Vec::new();

        for service in services {
            for event in service.events {
                all_events.push(AlertEvent {
                    service: service.name.clone(),
                    group: service.group.clone(),
                    event_type: event.event_type,
                    timestamp: event.timestamp,
                });
            }
        }

        // Sort events by timestamp descending (newest first)
        all_events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(all_events.into_iter().take(limit).collect())
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_failure_summary(&self, key: &str) -> Result<FailureSummary> {
        let statuses = self.get_endpoint_statuses(key).await?;
        let service = statuses
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Service not found: {}", key))?;

        let result = service
            .latest_result()
            .ok_or_else(|| anyhow::anyhow!("No results found for service: {}", key))?;

        let mut failed_conditions = Vec::new();
        let mut passed_conditions = Vec::new();

        for condition in &result.condition_results {
            if condition.success {
                passed_conditions.push(condition.condition.clone());
            } else {
                failed_conditions.push(condition.condition.clone());
            }
        }

        Ok(FailureSummary {
            name: service.name,
            group: service.group,
            failed_conditions,
            passed_conditions,
        })
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_notification_events(&self, key: &str) -> Result<Vec<CorrelatedEvent>> {
        let statuses = self.get_endpoint_statuses(key).await?;
        let service = statuses
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Service not found: {}", key))?;

        let mut correlated = Vec::new();

        for result in service.results {
            correlated.push(CorrelatedEvent {
                timestamp: result.timestamp,
                event_type: "result".to_string(),
                description: if result.success {
                    "Success".to_string()
                } else {
                    format!(
                        "Failure: {}",
                        result
                            .errors
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "Unknown error".to_string())
                    )
                },
            });
        }

        for event in service.events {
            correlated.push(CorrelatedEvent {
                timestamp: event.timestamp,
                event_type: "alert".to_string(),
                description: event.event_type,
            });
        }

        // Sort by timestamp descending
        correlated.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(correlated)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_flapping_services(&self) -> Result<Vec<FlappingService>> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/metrics", self.api_url);
        let mut request = self.client.get(url);
        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        let re = Regex::new(
            r#"gatus_results_total\{group="(?P<group>[^"]+)",name="(?P<name>[^"]+)",success="(?P<success>[^"]+)",type="[^"]+"\} (?P<value>\d+)"#,
        )?;

        let mut services: std::collections::HashMap<(String, String), (u64, u64)> =
            std::collections::HashMap::new();

        for cap in re.captures_iter(&text) {
            let group = cap["group"].to_string();
            let name = cap["name"].to_string();
            let success = &cap["success"];
            let value: u64 = cap["value"].parse().unwrap_or(0);

            let entry = services.entry((group, name)).or_insert((0, 0));
            if success == "true" {
                entry.1 += value;
            } else {
                entry.0 += value;
            }
        }

        let mut flapping: Vec<FlappingService> = services
            .into_iter()
            .map(
                |((group, name), (failure_count, success_count))| FlappingService {
                    name,
                    group,
                    failure_count,
                    success_count,
                },
            )
            .filter(|s| s.failure_count > 0)
            .collect();

        // Sort by failure count descending
        flapping.sort_by_key(|s| std::cmp::Reverse(s.failure_count));

        Ok(flapping)
    }

    #[tracing::instrument(skip(self))]
    pub async fn compare_performance(&self, key: &str) -> Result<PerformanceComparison> {
        let times_1h = self.get_endpoint_response_times(key, "1h").await?;
        let times_7d = self.get_endpoint_response_times(key, "7d").await?;

        let avg_1h = if times_1h.is_empty() {
            0.0
        } else {
            times_1h.iter().map(|p| p.value as f64).sum::<f64>() / times_1h.len() as f64
        };

        let avg_7d = if times_7d.is_empty() {
            0.0
        } else {
            times_7d.iter().map(|p| p.value as f64).sum::<f64>() / times_7d.len() as f64
        };

        let delta_percentage = if avg_7d == 0.0 {
            0.0
        } else {
            ((avg_1h - avg_7d) / avg_7d) * 100.0
        };

        Ok(PerformanceComparison {
            key: key.to_string(),
            avg_1h,
            avg_7d,
            delta_percentage,
        })
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_endpoint_uptimes(&self, key: &str, duration: &str) -> Result<UptimeResponse> {
        let cache_key = format!("uptimes_{}_{}", key, duration);
        if let Some(cached) = self.uptime_cache.get(&cache_key).await {
            return Ok(cached);
        }

        self.rate_limiter.until_ready().await;

        let url = format!(
            "{}/api/v1/endpoints/{}/uptimes/{}",
            self.api_url, key, duration
        );
        let mut request = self.client.get(url);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        let uptimes: UptimeResponse = serde_json::from_str(&text)?;
        self.uptime_cache.insert(cache_key, uptimes.clone()).await;
        Ok(uptimes)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_endpoint_response_times(
        &self,
        key: &str,
        duration: &str,
    ) -> Result<ResponseTimeResponse> {
        let cache_key = format!("response_times_{}_{}", key, duration);
        if let Some(cached) = self.response_time_cache.get(&cache_key).await {
            return Ok(cached);
        }

        self.rate_limiter.until_ready().await;

        let url = format!(
            "{}/api/v1/endpoints/{}/response-times/{}",
            self.api_url, key, duration
        );
        let mut request = self.client.get(url);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        let response_times: ResponseTimeResponse = serde_json::from_str(&text)?;
        self.response_time_cache
            .insert(cache_key, response_times.clone())
            .await;
        Ok(response_times)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_config(&self) -> Result<Value> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/config", self.api_url);
        let mut request = self.client.get(url);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        let config: Value = serde_json::from_str(&text)?;
        Ok(config)
    }

    #[tracing::instrument(skip(self))]
    pub async fn trigger_check(&self, key: &str) -> Result<()> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/endpoints/{}/trigger", self.api_url, key);
        let mut request = self.client.post(url);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn reload_config(&self) -> Result<()> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/config/reload", self.api_url);
        let mut request = self.client.post(url);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        Ok(())
    }

    pub fn get_badge_url(&self, key: &str) -> String {
        format!("{}/api/v1/endpoints/{}/health/badge.svg", self.api_url, key)
    }

    pub fn get_uptime_badge_url(&self, key: &str, duration: &str) -> String {
        format!(
            "{}/api/v1/endpoints/{}/uptimes/{}/badge.svg",
            self.api_url, key, duration
        )
    }

    pub fn get_latency_badge_url(&self, key: &str, duration: &str) -> String {
        format!(
            "{}/api/v1/endpoints/{}/response-times/{}/badge.svg",
            self.api_url, key, duration
        )
    }

    pub fn get_latency_chart_url(&self, key: &str, duration: &str) -> String {
        format!(
            "{}/api/v1/endpoints/{}/response-times/{}/chart.svg",
            self.api_url, key, duration
        )
    }

    #[tracing::instrument(skip(self, result))]
    pub async fn push_endpoint_result(&self, key: &str, result: HealthResult) -> Result<()> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/endpoints/{}/external", self.api_url, key);
        let mut query_params = vec![
            ("success", result.success.to_string()),
            ("duration", result.duration.to_string()),
        ];
        if let Some(err) = result.errors.first() {
            query_params.push(("error", err.clone()));
        }

        let mut request = self.client.post(url).query(&query_params);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_raw_results(&self, key: &str, limit: usize) -> Result<Vec<HealthResult>> {
        let services = self.get_endpoint_statuses(key).await?;
        match services.into_iter().next() {
            Some(s) => Ok(s.results.into_iter().take(limit).collect()),
            None => anyhow::bail!("Endpoint '{}' not found", key),
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_alert_rules(&self) -> Result<Vec<AlertRule>> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/config", self.api_url);
        let mut request = self.client.get(url);
        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        #[derive(Deserialize)]
        struct Config {
            endpoints: Vec<EndpointConfig>,
        }

        let config: Config = serde_json::from_str(&text)?;
        let mut alert_rules = Vec::new();

        for endpoint in config.endpoints {
            for alert in endpoint.alerts {
                alert_rules.push(AlertRule {
                    endpoint: endpoint.name.clone(),
                    group: endpoint.group.clone().unwrap_or_else(|| "".to_string()),
                    alert_type: alert.alert_type,
                    enabled: alert.enabled,
                    failure_threshold: alert.failure_threshold,
                    success_threshold: alert.success_threshold,
                    description: alert.description,
                });
            }
        }

        Ok(alert_rules)
    }

    #[tracing::instrument(skip(self))]
    pub async fn test_alert_notification(&self, key: &str) -> Result<()> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/api/v1/endpoints/{}/test-alert", self.api_url, key);
        let mut request = self.client.post(url);
        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_diagnostic_bundle(&self, key: &str) -> Result<DiagnosticBundle> {
        let statuses = self.get_endpoint_statuses(key).await?;
        let service = statuses
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Service not found: {}", key))?;

        let failure_summary = if let Some(result) = service.latest_result() {
            let mut failed_conditions = Vec::new();
            let mut passed_conditions = Vec::new();

            for condition in &result.condition_results {
                if condition.success {
                    passed_conditions.push(condition.condition.clone());
                } else {
                    failed_conditions.push(condition.condition.clone());
                }
            }

            FailureSummary {
                name: service.name.clone(),
                group: service.group.clone(),
                failed_conditions,
                passed_conditions,
            }
        } else {
            FailureSummary {
                name: service.name.clone(),
                group: service.group.clone(),
                failed_conditions: Vec::new(),
                passed_conditions: Vec::new(),
            }
        };

        let alert_events = service
            .events
            .iter()
            .map(|e| AlertEvent {
                service: service.name.clone(),
                group: service.group.clone(),
                event_type: e.event_type.clone(),
                timestamp: e.timestamp.clone(),
            })
            .collect();

        Ok(DiagnosticBundle {
            name: service.name,
            group: service.group,
            results: service.results,
            failure_summary,
            alert_events,
        })
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_certificate_audit(&self, key: &str) -> Result<CertificateAudit> {
        let statuses = self.get_endpoint_statuses(key).await?;
        let service = statuses
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Service not found: {}", key))?;

        let result = service
            .latest_result()
            .ok_or_else(|| anyhow::anyhow!("No results found for service: {}", key))?;

        Ok(CertificateAudit {
            name: service.name.clone(),
            group: service.group.clone(),
            issuer: result.certificate_issuer.clone(),
            algorithm: result.certificate_algorithm.clone(),
            sans: result.certificate_sans.clone().unwrap_or_default(),
            expiration: result.certificate_expiration,
        })
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_instance_health(&self) -> Result<String> {
        self.rate_limiter.until_ready().await;

        let url = format!("{}/health", self.api_url);
        let mut request = self.client.get(url);

        request = self.add_auth_header(request);

        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gatus API error: status {}, body: {}", status, text);
        }

        Ok(text)
    }
}
