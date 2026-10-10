//! Separate process: transport is process-wide, so tests must not change the
//! proxy under unrelated parallel HTTP fixtures in the library test binary.
//! The tests here share that transport too, so each holds `TRANSPORT`.
use spectra_core::{
    registry::Chain,
    service::{ChainEndpoints, WalletService},
    store::state::{AppSettingUpdate, StateCommand},
    tor::{TorStatus, tor_status},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

static TRANSPORT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn committed_settings_switch_proxy_and_reset_without_a_shell_callback() {
    let _transport = TRANSPORT.lock().await;
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

/// A SOCKS5 proxy that answers every JSON-RPC call it carries with `0x1`,
/// recording each destination it was asked to connect to: `None` where the
/// client resolved the name itself and sent an address.
async fn socks5_proxy() -> (
    u16,
    std::sync::Arc<std::sync::Mutex<Vec<Option<(String, u16)>>>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let destinations = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = destinations.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut stream = BufReader::new(stream);
                let mut greeting = [0u8; 2];
                stream.read_exact(&mut greeting).await.ok()?;
                let mut methods = vec![0u8; usize::from(greeting[1])];
                stream.read_exact(&mut methods).await.ok()?;
                stream.get_mut().write_all(&[5, 0]).await.ok()?;
                let mut header = [0u8; 4];
                stream.read_exact(&mut header).await.ok()?;
                let destination = if header[3] == 3 {
                    let mut length = [0u8; 1];
                    stream.read_exact(&mut length).await.ok()?;
                    let mut host = vec![0u8; usize::from(length[0])];
                    stream.read_exact(&mut host).await.ok()?;
                    let mut port = [0u8; 2];
                    stream.read_exact(&mut port).await.ok()?;
                    Some((String::from_utf8(host).ok()?, u16::from_be_bytes(port)))
                } else {
                    None
                };
                recorded.lock().unwrap().push(destination.clone());
                destination.as_ref()?;
                stream
                    .get_mut()
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                    .await
                    .ok()?;
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    stream.read_line(&mut line).await.ok()?;
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().ok()?;
                    }
                }
                let mut body = vec![0u8; length];
                stream.read_exact(&mut body).await.ok()?;
                let request: serde_json::Value = serde_json::from_slice(&body).ok()?;
                let answer = |call: &serde_json::Value| serde_json::json!({"jsonrpc": "2.0", "id": call["id"], "result": "0x1"});
                let response = match &request {
                    serde_json::Value::Array(calls) => {
                        serde_json::Value::Array(calls.iter().map(answer).collect())
                    }
                    call => answer(call),
                }
                .to_string();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{response}",
                    response.len()
                );
                stream.get_mut().write_all(reply.as_bytes()).await.ok()?;
                stream.get_mut().shutdown().await.ok()
            });
        }
    });
    (port, destinations)
}

/// A fresh service reads the saved custom proxy and routes through it, by
/// hostname: the proxy resolves the name, so a host no resolver here knows
/// is still reached.
#[tokio::test]
async fn a_reopened_store_routes_through_its_saved_proxy_by_hostname() {
    let _transport = TRANSPORT.lock().await;
    let directory =
        std::env::temp_dir().join(format!("spectra-transport-proxy-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory
        .join("state.sqlite")
        .to_string_lossy()
        .into_owned();
    let (port, destinations) = socks5_proxy().await;
    let saving = WalletService::new(vec![]).unwrap();
    saving.open_state(database.clone()).await.unwrap();
    for update in [
        AppSettingUpdate::TorUseCustomProxy { value: true },
        AppSettingUpdate::TorCustomProxyAddress {
            value: format!("socks5://127.0.0.1:{port}"),
        },
        AppSettingUpdate::TorEnabled { value: true },
    ] {
        saving
            .apply_state_command(StateCommand::SetAppSetting { update })
            .await
            .unwrap();
    }
    drop(saving);

    let reopened = WalletService::new(vec![ChainEndpoints {
        capabilities: spectra_core::EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Ethereum,
        endpoints: vec!["http://spectra.invalid:8545".into()],
    }])
    .unwrap();
    reopened.open_state(database).await.unwrap();
    assert!(matches!(
        reopened
            .configure_network_runtime(directory.to_string_lossy().into())
            .await
            .unwrap(),
        TorStatus::Ready
    ));
    let balance = reopened
        .fetch_native_balance_summary(Chain::Ethereum, format!("0x{}", "11".repeat(20)))
        .await
        .unwrap();
    assert_eq!(balance.smallest_unit, "1");
    let destinations = destinations.lock().unwrap().clone();
    assert!(!destinations.is_empty());
    assert!(
        destinations
            .iter()
            .all(|destination| destination == &Some(("spectra.invalid".to_string(), 8545))),
        "{destinations:?}"
    );

    reopened
        .apply_state_command(StateCommand::SetAppSetting {
            update: AppSettingUpdate::TorEnabled { value: false },
        })
        .await
        .unwrap();
    assert!(matches!(tor_status(), TorStatus::Stopped));
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}
