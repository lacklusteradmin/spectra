use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityCounter {
    pub success_count: u32,
    pub failure_count: u32,
    pub last_updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EndpointOrderingRequest {
    pub candidates: Vec<String>,
    pub counters: HashMap<String, ReliabilityCounter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EndpointAttemptRequest {
    pub counters: HashMap<String, ReliabilityCounter>,
    pub endpoint: String,
    pub success: bool,
    pub observed_at: i64,
}

pub fn order_endpoints(request: EndpointOrderingRequest) -> Vec<String> {
    let mut ordered = request.candidates;
    let counters = request.counters;
    ordered.sort_by(|lhs, rhs| {
        let left_score = score(counters.get(lhs));
        let right_score = score(counters.get(rhs));
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| lhs.cmp(rhs))
    });
    ordered
}

pub fn record_attempt(request: EndpointAttemptRequest) -> HashMap<String, ReliabilityCounter> {
    let mut counters = request.counters;
    let mut counter = counters
        .remove(&request.endpoint)
        .unwrap_or(ReliabilityCounter {
            success_count: 0,
            failure_count: 0,
            last_updated_at: 0,
        });

    if request.success {
        counter.success_count = counter.success_count.saturating_add(1);
    } else {
        counter.failure_count = counter.failure_count.saturating_add(1);
    }
    counter.last_updated_at = request.observed_at;
    counters.insert(request.endpoint, counter);
    counters
}

fn score(counter: Option<&ReliabilityCounter>) -> f64 {
    match counter {
        Some(counter) => {
            let attempts = std::cmp::max(1, counter.success_count + counter.failure_count) as f64;
            counter.success_count as f64 / attempts
        }
        None => 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_by_success_ratio_then_name() {
        let ordered = order_endpoints(EndpointOrderingRequest {
            candidates: vec![
                "https://b.example".to_string(),
                "https://a.example".to_string(),
                "https://c.example".to_string(),
            ],
            counters: HashMap::from([
                (
                    "https://a.example".to_string(),
                    ReliabilityCounter {
                        success_count: 4,
                        failure_count: 1,
                        last_updated_at: 10,
                    },
                ),
                (
                    "https://b.example".to_string(),
                    ReliabilityCounter {
                        success_count: 1,
                        failure_count: 4,
                        last_updated_at: 10,
                    },
                ),
            ]),
        });

        assert_eq!(
            ordered,
            vec![
                "https://a.example".to_string(),
                "https://c.example".to_string(),
                "https://b.example".to_string()
            ]
        );
    }

    #[test]
    fn records_success_attempts() {
        let counters = record_attempt(EndpointAttemptRequest {
            counters: HashMap::new(),
            endpoint: "https://rpc.example".to_string(),
            success: true,
            observed_at: 42,
        });

        assert_eq!(
            counters.get("https://rpc.example"),
            Some(&ReliabilityCounter {
                success_count: 1,
                failure_count: 0,
                last_updated_at: 42,
            })
        );
    }
}
