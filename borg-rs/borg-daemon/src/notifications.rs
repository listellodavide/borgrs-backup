//! Notification system for daemon alerts

use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::config::{NotificationConfig, RetryConfig};

/// Notification events emitted by the daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationEvent {
    JobSuccess { job: String, archive: Option<String> },
    JobFailure { job: String, error: String },
    JobMissed { job: String, expected: String, last_run: Option<String> },
}

/// Notification payload sent to providers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationPayload {
    pub event: NotificationEvent,
    pub timestamp: String,
    pub host: String,
}

impl NotificationPayload {
    pub fn new(event: NotificationEvent) -> Result<Self> {
        let host = nix::unistd::gethostname()?
            .to_string_lossy()
            .to_string();
        Ok(Self {
            event,
            timestamp: chrono::Utc::now().to_rfc3339(),
            host,
        })
    }
}

/// Notification provider interface
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, payload: &NotificationPayload) -> Result<()>;
}

/// Execute command-based notifications
pub struct CommandNotifier {
    command: String,
}

impl CommandNotifier {
    pub fn new(command: String) -> Self {
        Self { command }
    }
}

#[async_trait]
impl Notifier for CommandNotifier {
    async fn notify(&self, payload: &NotificationPayload) -> Result<()> {
        let body = serde_json::to_string(payload)?;
        let status = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .env("BORG_EVENT", body)
            .status()
            .await
            .context("Failed to execute notification command")?;

        if !status.success() {
            anyhow::bail!("Notification command failed with status: {}", status);
        }

        Ok(())
    }
}

/// Webhook notifier for HTTP endpoints
pub struct WebhookNotifier {
    url: String,
}

impl WebhookNotifier {
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

#[async_trait]
impl Notifier for WebhookNotifier {
    async fn notify(&self, payload: &NotificationPayload) -> Result<()> {
        let body = serde_json::to_string(payload)?;
        let client = reqwest::Client::new();
        let response = client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .context("Failed to send webhook notification")?;

        if !response.status().is_success() {
            anyhow::bail!("Webhook returned status {}", response.status());
        }

        Ok(())
    }
}

/// Slack notifier (incoming webhook)
pub struct SlackNotifier {
    url: String,
}

impl SlackNotifier {
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

#[async_trait]
impl Notifier for SlackNotifier {
    async fn notify(&self, payload: &NotificationPayload) -> Result<()> {
        let text = format!("Borg-Rust alert: {:?}", payload.event);
        let body = serde_json::json!({ "text": text });
        let client = reqwest::Client::new();
        let response = client
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .context("Failed to send Slack notification")?;

        if !response.status().is_success() {
            anyhow::bail!("Slack webhook returned status {}", response.status());
        }

        Ok(())
    }
}

/// Stub email notifier (SMTP placeholder)
pub struct EmailNotifier {
    to: String,
}

impl EmailNotifier {
    pub fn new(to: String) -> Self {
        Self { to }
    }
}

#[async_trait]
impl Notifier for EmailNotifier {
    async fn notify(&self, payload: &NotificationPayload) -> Result<()> {
        let body = serde_json::to_string(payload)?;
        info!("Email notification to {}: {}", self.to, body);
        Ok(())
    }
}

/// Multi-channel notification dispatcher
pub struct NotificationDispatcher {
    notifiers: Vec<Box<dyn Notifier>>,
}

impl NotificationDispatcher {
    pub fn new(notifiers: Vec<Box<dyn Notifier>>) -> Self {
        Self { notifiers }
    }

    pub async fn send(&self, payload: NotificationPayload, retry: &RetryConfig) {
        for notifier in &self.notifiers {
            if let Err(err) = send_with_retry(notifier.as_ref(), &payload, retry).await {
                warn!("Notification failed after retries: {}", err);
            }
        }
    }
}

async fn send_with_retry(
    notifier: &dyn Notifier,
    payload: &NotificationPayload,
    retry: &RetryConfig,
) -> Result<()> {
    let mut attempt = 0;
    let mut delay = Duration::from_secs(retry.initial_delay);

    loop {
        attempt += 1;
        let result = notifier.notify(payload).await;
        if result.is_ok() {
            return Ok(());
        }

        if attempt > retry.max_retries {
            return result;
        }

        debug!(
            "Notification attempt {} failed, retrying in {:?}",
            attempt,
            delay
        );
        sleep(delay).await;
        delay = Duration::from_secs_f64(
            (delay.as_secs_f64() * retry.backoff_multiplier)
                .min(retry.max_delay as f64),
        );
    }
}

/// Build notification dispatchers from config
pub fn build_dispatcher(config: &NotificationConfig) -> Option<NotificationDispatcher> {
    let mut notifiers: Vec<Box<dyn Notifier>> = Vec::new();

    if let Some(command) = config.command.clone() {
        notifiers.push(Box::new(CommandNotifier::new(command)));
    }

    if let Some(url) = config.webhook_url.clone() {
        notifiers.push(Box::new(WebhookNotifier::new(url)));
    }

    if let Some(url) = config.slack_webhook_url.clone() {
        notifiers.push(Box::new(SlackNotifier::new(url)));
    }

    if let Some(email) = config.email.clone() {
        notifiers.push(Box::new(EmailNotifier::new(email)));
    }

    if notifiers.is_empty() {
        None
    } else {
        Some(NotificationDispatcher::new(notifiers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_notification_payload() {
        let payload = NotificationPayload::new(NotificationEvent::JobSuccess {
            job: "demo".to_string(),
            archive: Some("archive1".to_string()),
        })
        .unwrap();

        assert!(payload.timestamp.contains('T'));
        assert!(!payload.host.is_empty());
    }

    #[test]
    fn test_build_dispatcher() {
        let config = NotificationConfig {
            on_success: true,
            on_failure: true,
            command: Some("echo ok".to_string()),
            email: Some("ops@example.com".to_string()),
            webhook_url: None,
            slack_webhook_url: None,
            failure_threshold: None,
            missed_threshold_minutes: None,
        };

        let dispatcher = build_dispatcher(&config);
        assert!(dispatcher.is_some());
    }
}