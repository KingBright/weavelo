//! org.json represents integers above i64::MAX as Double. Keep the opaque
//! unsigned request ID as decimal text only across the Android JSON boundary.
use protocol::session::{DeliveryClaimProof, SessionCommand};
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
    let id = exact_request_id(&value["request_id"])?;
    let mut value = value.clone();
    value["request_id"] = id.into();
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn exact_request_id(value: &Value) -> Result<u64, String> {
    let text = value.as_str().ok_or("Missing exact delivery request ID")?;
    let id: u64 = text.parse().map_err(|_| "Invalid delivery request ID")?;
    if text != id.to_string() {
        return Err("Invalid delivery request ID".into());
    }
    Ok(id)
}

pub(crate) fn encode_control(command: &SessionCommand) -> Value {
    let mut value = serde_json::to_value(command).expect("Session control JSON");
    if let SessionCommand::CameraSwitchResult { request, .. } = command
        && let Some(id) = request.delivery_request_id
    {
        value["CameraSwitchResult"]["request"]["delivery_request_id"] = id.to_string().into();
    }
    value
}

pub(crate) fn decode_control(json: &str) -> Result<SessionCommand, String> {
    let mut value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if let Some(id) = value.get_mut("RequestCameraSwitch").and_then(|r| r.get_mut("delivery_request_id"))
        && !id.is_null()
    {
        *id = exact_request_id(id)?.into();
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
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

    #[test]
    fn camera_switch_keeps_unsigned_delivery_ids_exact_across_android_json() {
        for id in [(1 << 53) + 1, i64::MAX as u64 + 1, u64::MAX] {
            let request = protocol::session::CameraSwitchRequest {
                id: 1, request_id: 2, source_revision: 3,
                from: CaptureSource::Camera(4), target: CaptureSource::Camera(5),
                delivery_request_id: Some(id),
            };
            let reply = encode_control(&SessionCommand::CameraSwitchResult { request: request.clone(), ready: Ok(()) });
            assert_eq!(reply["CameraSwitchResult"]["request"]["delivery_request_id"], id.to_string());
            let mut command = serde_json::to_value(SessionCommand::RequestCameraSwitch(request.clone())).unwrap();
            command["RequestCameraSwitch"]["delivery_request_id"] = id.to_string().into();
            assert!(matches!(decode_control(&command.to_string()).unwrap(),
                SessionCommand::RequestCameraSwitch(decoded) if decoded == request));
            command["RequestCameraSwitch"]["delivery_request_id"] = serde_json::json!(id);
            assert!(decode_control(&command.to_string()).is_err());
        }
    }
}
