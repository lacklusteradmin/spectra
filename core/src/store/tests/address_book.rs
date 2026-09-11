use crate::service::WalletService;
use crate::state::{AddressBookRejection, StateCommand};

const BTC: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
const BTC2: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

fn tmp_db(tag: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-address-book-{tag}-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

fn service() -> std::sync::Arc<WalletService> {
    WalletService::new_typed(Vec::new()).expect("service")
}

fn add(id: &str, name: &str, address: &str) -> StateCommand {
    StateCommand::AddAddressBookEntry {
        id: id.to_string(),
        name: name.to_string(),
        chain_name: "Bitcoin".to_string(),
        address: address.to_string(),
        note: String::new(),
    }
}

fn rejection(events: &[crate::state::StateEvent]) -> Option<String> {
    events
        .iter()
        .find(|e| e.kind == "addressBookRejected")
        .and_then(|e| e.subject_id.clone())
}

#[tokio::test]
async fn adds_newest_first_and_trims() {
    let service = service();
    service
        .apply_state_command(add("1", "  Cold  ", BTC))
        .await
        .expect("add");
    let transition = service
        .apply_state_command(add("2", "Hot", BTC2))
        .await
        .expect("add");

    let ids: Vec<&str> = transition
        .state
        .address_book
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, vec!["2", "1"], "newest entry comes first");
    assert_eq!(transition.state.address_book[1].name, "Cold");
}

#[tokio::test]
async fn refuses_an_empty_name() {
    let service = service();
    let transition = service
        .apply_state_command(add("1", "   ", BTC))
        .await
        .expect("add");
    assert!(transition.state.address_book.is_empty());
    assert_eq!(rejection(&transition.events).as_deref(), Some("emptyName"));
}

#[tokio::test]
async fn refuses_an_address_that_is_not_valid_for_the_chain() {
    let service = service();
    let transition = service
        .apply_state_command(add("1", "Typo", "bc1qnot-a-real-address"))
        .await
        .expect("add");
    assert!(transition.state.address_book.is_empty());
    assert_eq!(
        rejection(&transition.events).as_deref(),
        Some("invalidAddress")
    );
}

/// The same address twice is refused, and case does not get around it —
/// addresses are stored normalized.
#[tokio::test]
async fn refuses_a_duplicate_regardless_of_case() {
    let service = service();
    service
        .apply_state_command(add("1", "Cold", BTC))
        .await
        .expect("add");
    let transition = service
        .apply_state_command(add("2", "Cold again", &BTC.to_uppercase()))
        .await
        .expect("add");
    assert_eq!(transition.state.address_book.len(), 1);
    assert_eq!(
        rejection(&transition.events).as_deref(),
        Some("duplicateAddress")
    );
}

/// The same address on a different chain is a different recipient.
#[tokio::test]
async fn the_same_address_on_another_chain_is_not_a_duplicate() {
    let service = service();
    service
        .apply_state_command(add("1", "BTC", BTC))
        .await
        .expect("add");
    let transition = service
        .apply_state_command(StateCommand::AddAddressBookEntry {
            id: "2".to_string(),
            name: "LTC".to_string(),
            chain_name: "Litecoin".to_string(),
            address: "ltc1qw508d6qejxtdg4y5r3zarvary0c5xw7kgmn4n9".to_string(),
            note: String::new(),
        })
        .await
        .expect("add");
    assert_eq!(transition.state.address_book.len(), 2);
    assert!(rejection(&transition.events).is_none());
}

#[tokio::test]
async fn renames_and_removes() {
    let service = service();
    service
        .apply_state_command(add("1", "Cold", BTC))
        .await
        .expect("add");

    let renamed = service
        .apply_state_command(StateCommand::RenameAddressBookEntry {
            id: "1".to_string(),
            name: "  Vault  ".to_string(),
        })
        .await
        .expect("rename");
    assert_eq!(renamed.state.address_book[0].name, "Vault");

    let empty = service
        .apply_state_command(StateCommand::RenameAddressBookEntry {
            id: "1".to_string(),
            name: "  ".to_string(),
        })
        .await
        .expect("rename");
    assert_eq!(empty.state.address_book[0].name, "Vault", "unchanged");
    assert_eq!(rejection(&empty.events).as_deref(), Some("emptyName"));

    let removed = service
        .apply_state_command(StateCommand::RemoveAddressBookEntry {
            id: "1".to_string(),
        })
        .await
        .expect("remove");
    assert!(removed.state.address_book.is_empty());

    // Removing what is already gone is not a change.
    let again = service
        .apply_state_command(StateCommand::RemoveAddressBookEntry {
            id: "1".to_string(),
        })
        .await
        .expect("remove");
    assert!(again.events.is_empty());
}

#[tokio::test]
async fn survives_a_restart_in_order() {
    let db = tmp_db("persist");

    let first = service();
    first.open_state(db.clone()).await.expect("open");
    first
        .apply_state_command(add("1", "Cold", BTC))
        .await
        .expect("add");
    first
        .apply_state_command(add("2", "Hot", BTC2))
        .await
        .expect("add");

    let second = service();
    let reopened = second.open_state(db.clone()).await.expect("reopen");
    let ids: Vec<&str> = reopened
        .address_book
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, vec!["2", "1"]);
    assert_eq!(reopened.address_book[0].name, "Hot");

    let _ = std::fs::remove_file(&db);
}

#[test]
fn rejection_reasons_serialize_as_the_strings_front_ends_match_on() {
    for (reason, expected) in [
        (AddressBookRejection::EmptyName, "emptyName"),
        (AddressBookRejection::InvalidAddress, "invalidAddress"),
        (AddressBookRejection::DuplicateAddress, "duplicateAddress"),
    ] {
        assert_eq!(
            serde_json::to_value(reason).unwrap().as_str(),
            Some(expected)
        );
    }
}
