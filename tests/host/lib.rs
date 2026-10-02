#[path = "../../src/reliability.rs"]
pub mod reliability;
#[path = "../../src/hw/battery_measurement.rs"]
pub mod battery_measurement;
#[path = "../../src/scale/protocol.rs"]
pub mod protocol;
#[path = "../../src/net/telemetry_batch.rs"]
pub mod telemetry_batch;
#[path = "../../src/net/outbox.rs"]
pub mod outbox;

#[path = "../../src/config/model.rs"]
pub mod config;
#[path = "../../src/config/patch.rs"]
pub mod config_patch;

#[path = "../../src/display/orientation.rs"]
pub mod orientation;

#[path = "../../src/net/admin_policy.rs"]
pub mod admin_policy;

#[path = "../../src/net/miio_wire.rs"]
pub mod miio_wire;

#[path = "../../src/net/miio_config.rs"]
pub mod miio_config;

#[path = "../../src/ui/weight_daily.rs"]
pub mod weight_daily;

#[path = "../../src/scale/history.rs"]
pub mod scale_history;

#[test]
fn extended_history_never_replays_acknowledged_samples() {
    assert!(outbox::SEEN_LIMIT >= scale_history::CAPACITY);
    let mut state = outbox::Durable::default();
    for index in 0..scale_history::CAPACITY {
        let key = format!("{index:016x}");
        assert!(state.enqueue(outbox::Pending { key, payload: "{}".into() }));
        state.promote();
        state.acknowledge();
    }
    let restored: outbox::Durable = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    for index in 0..scale_history::CAPACITY {
        assert!(restored.contains(&format!("{index:016x}")));
    }
}
