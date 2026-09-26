//! Separate process: transport is process-wide, so tests must not change the
//! proxy under unrelated parallel HTTP fixtures in the library test binary.
use spectra_core::{
    service::WalletService,
    store::state::{AppSettingUpdate, StateCommand},
    tor::{TorStatus, tor_status},
};

#[tokio::test]
async fn committed_settings_switch_proxy_and_reset_without_a_shell_callback() {
    let directory = std::env::temp_dir().join(format!("spectra-transport-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let service = WalletService::new(vec![]).unwrap();
    service
        .open_state(directory.join("state.sqlite").to_string_lossy().into())
        .await
        .unwrap();
    for update in [
        AppSettingUpdate::TorUseCustomProxy { value: true },
        AppSettingUpdate::TorEnabled { value: true },
    ] {
        service
            .apply_state_command(StateCommand::SetAppSetting { update })
            .await
            .unwrap();
    }
    assert!(matches!(
        service
            .configure_network_runtime(directory.to_string_lossy().into())
            .await
            .unwrap(),
        TorStatus::Ready
    ));
    service
        .apply_state_command(StateCommand::SetAppSetting {
            update: AppSettingUpdate::TorCustomProxyAddress {
                value: "socks5h://127.0.0.1:9999".into(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(tor_status(), TorStatus::Ready));
    assert!(matches!(service.reconnect_tor().await, TorStatus::Ready));
    service
        .reset_data(vec![
            spectra_core::store::state::ResetScope::SettingsAndEndpoints,
        ])
        .await
        .unwrap();
    assert!(matches!(tor_status(), TorStatus::Stopped));
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}
