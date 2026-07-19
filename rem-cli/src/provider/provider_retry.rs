use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use anyhow::{anyhow, Result};
use rand::Rng;
use tokio::time::{sleep, Duration};

use crate::constants::{LLM_RETRY_BASE_DELAY_MS, LLM_RETRY_MAX_ATTEMPTS};
use crate::provider::provider_error::ProviderError;
use crate::provider::provider_stream::STREAM_CANCELLED;
use crate::provider::ProviderKind;

const CIRCUIT_BREAKER_THRESHOLD: u32 = 3;
const CIRCUIT_BREAKER_COOLDOWN_SECS: u64 = 30;

#[derive(Default)]
struct CircuitState {
    consecutive_failures: u32,
    cooldown_until: Option<Instant>,
}

static CIRCUIT_BREAKER: LazyLock<Mutex<HashMap<ProviderKind, CircuitState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn circuit_breaker_check(kind: ProviderKind) -> Result<()> {
    let mut cb = CIRCUIT_BREAKER.lock().unwrap_or_else(|e| e.into_inner());
    let state = cb.entry(kind).or_default();
    if state.consecutive_failures >= CIRCUIT_BREAKER_THRESHOLD {
        if let Some(cooldown) = state.cooldown_until {
            if Instant::now() < cooldown {
                return Err(anyhow!(
                    "circuit breaker open for {} — too many transient errors ({} consecutive). Retry in ~{}s",
                    kind.as_str(),
                    state.consecutive_failures,
                    cooldown.saturating_duration_since(Instant::now()).as_secs(),
                ));
            }
            state.consecutive_failures = 0;
            state.cooldown_until = None;
        }
    }
    Ok(())
}

fn circuit_breaker_record_success(kind: ProviderKind) {
    let mut cb = CIRCUIT_BREAKER.lock().unwrap_or_else(|e| e.into_inner());
    let state = cb.entry(kind).or_default();
    state.consecutive_failures = 0;
    state.cooldown_until = None;
}

fn circuit_breaker_record_failure(kind: ProviderKind) {
    let mut cb = CIRCUIT_BREAKER.lock().unwrap_or_else(|e| e.into_inner());
    let state = cb.entry(kind).or_default();
    state.consecutive_failures += 1;
    if state.consecutive_failures >= CIRCUIT_BREAKER_THRESHOLD {
        state.cooldown_until = Some(Instant::now() + Duration::from_secs(CIRCUIT_BREAKER_COOLDOWN_SECS));
        tracing::warn!(
            "circuit breaker tripped for {} — {} consecutive failures, cooling down for {}s",
            kind.as_str(),
            state.consecutive_failures,
            CIRCUIT_BREAKER_COOLDOWN_SECS,
        );
    }
}

pub(crate) fn is_transient_error(e: &anyhow::Error) -> bool {
    let err_str = e.to_string();
    if let Some(pe) = e.downcast_ref::<ProviderError>() {
        return matches!(
            pe,
            ProviderError::Timeout(_) | ProviderError::RateLimit(_) | ProviderError::ServerError(_)
        );
    }
    if e.downcast_ref::<tokio::time::error::Elapsed>().is_some() {
        return true;
    }
    if let Some(req_err) = e.downcast_ref::<reqwest::Error>() {
        if req_err.is_timeout() || req_err.is_connect() {
            return true;
        }
        if let Some(status) = req_err.status() {
            let code = status.as_u16();
            if code == 429 || (500..=504).contains(&code) {
                return true;
            }
        }
    }
    err_str.contains("connection refused")
        || err_str.contains("connection reset")
        || err_str.contains("timed out")
        || err_str.contains("429 ")
        || err_str.contains("429,")
        || err_str.contains("500 ")
        || err_str.contains("501 ")
        || err_str.contains("502 ")
        || err_str.contains("503 ")
        || err_str.contains("504 ")
}

async fn with_retry<F, Fut, T>(f: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    if STREAM_CANCELLED.load(Ordering::Relaxed) {
        return Err(anyhow!(ProviderError::Cancelled));
    }
    let mut last_err = None;
    for attempt in 0..LLM_RETRY_MAX_ATTEMPTS as usize {
        if STREAM_CANCELLED.load(Ordering::Relaxed) {
            return Err(anyhow!(ProviderError::Cancelled));
        }
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if !is_transient_error(&e) || attempt == LLM_RETRY_MAX_ATTEMPTS as usize - 1 {
                    return Err(e);
                }
                last_err = Some(e);
                let base_delay_ms = LLM_RETRY_BASE_DELAY_MS * 2u64.pow(attempt as u32);
                let jitter = rand::thread_rng().gen_range(0.5_f64..1.5);
                let delay_ms = (base_delay_ms as f64 * jitter) as u64;
                let delay = Duration::from_millis(delay_ms.max(100));
                let poll_interval = Duration::from_millis(100);
                let mut remaining = delay;
                while remaining > Duration::ZERO {
                    if STREAM_CANCELLED.load(Ordering::Relaxed) {
                        return Err(anyhow!(ProviderError::Cancelled));
                    }
                    let step = remaining.min(poll_interval);
                    sleep(step).await;
                    remaining = remaining.saturating_sub(step);
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!(ProviderError::Other("retry exhausted".into()))))
}

pub(crate) async fn with_retry_and_circuit_breaker<F, Fut, T>(kind: ProviderKind, f: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    circuit_breaker_check(kind)?;
    match with_retry(f).await {
        Ok(v) => {
            circuit_breaker_record_success(kind);
            Ok(v)
        }
        Err(e) => {
            if is_transient_error(&e) {
                circuit_breaker_record_failure(kind);
            }
            Err(e)
        }
    }
}
