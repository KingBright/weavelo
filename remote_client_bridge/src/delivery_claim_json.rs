//! org.json represents integers above i64::MAX as Double. Keep the opaque
//! unsigned request ID as decimal text only across the Android JSON boundary.
use protocol::session::DeliveryClaimProof;
use serde_json::Value;

pub(crate) fn encode(claim: DeliveryClaimProof) -> Value {
    serde_json::json!({
        "request_id": claim.request_id.to_string(),
        "ticket": claim.ticket,
        "target_device_id": claim.target_device_id,
        "sources": claim.sources,
    })
}

pub(crate) fn decode(value: &Value) -> Result<DeliveryClaimProof, String> {
    let text = value["request_id"].as_str().ok_or("Missing exact delivery request ID")?;
    let id: u64 = text.parse().map_err(|_| "Invalid delivery request ID")?;
    if text != id.to_string() {
        return Err("Invalid delivery request ID".into());
    }
    let mut value = value.clone();
    value["request_id"] = id.into();
    serde_json::from_value(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::session::CaptureSource;

    #[test]
    fn unsigned_ids_survive_the_android_json_roundtrip_without_precision_loss() {
        for id in [1, (1 << 53) + 1, i64::MAX as u64, i64::MAX as u64 + 1, 16_885_485_083_476_720_000, u64::MAX] {
            let claim = DeliveryClaimProof {
                request_id: id,
                ticket: [173; 32],
                target_device_id: "owned-phone".into(),
                sources: vec![CaptureSource::Window(7), CaptureSource::MainDisplay],
            };
            let wire = serde_json::to_string(&claim).unwrap();
            let encoded = encode(claim);
            assert_eq!(encoded["request_id"].as_str(), Some(id.to_string().as_str()));
            let transported: Value = serde_json::from_str(&encoded.to_string()).unwrap();
            assert_eq!(serde_json::to_string(&decode(&transported).unwrap()).unwrap(), wire);
        }
    }

    #[test]
    fn rounded_or_noncanonical_request_ids_are_rejected() {
        let valid = encode(DeliveryClaimProof {
            request_id: 1, ticket: [1; 32], target_device_id: "owned-phone".into(),
            sources: vec![CaptureSource::MainDisplay],
        });
        for invalid in [serde_json::json!(1.688548508347672e19), serde_json::json!(1),
            serde_json::json!("18446744073709551616"), serde_json::json!("+1"), serde_json::json!("01"), Value::Null] {
            let mut value = valid.clone();
            value["request_id"] = invalid;
            assert!(decode(&value).is_err());
        }
    }
}
