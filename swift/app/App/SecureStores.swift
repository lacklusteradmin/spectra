import Foundation
import KeychainAccess
import Security
enum KeychainStoreError: Error, Equatable {
    case missingValue
    case invalidEncoding
    /// The Secure Enclave key could not be read, created or used. Core refuses
    /// rather than mint a device key over one it could not unwrap.
    case keyWrapping(String)
}

extension KeychainStoreError: LocalizedError {
    var errorDescription: String? {
        switch self {
        case .missingValue: return "No value is stored for this item."
        case .invalidEncoding: return "The stored value is not valid UTF-8."
        case .keyWrapping(let detail): return "The device key could not be wrapped: \(detail)"
        }
    }
}

// Sound: KeychainAccess's `Keychain` stores only a `let` options value, and every call is a Security framework call, which is thread-safe.
private struct KeychainBackedSecureStore: @unchecked Sendable {
    private let keychain: Keychain
    init(service: String) {
        keychain = Keychain(service: service).accessibility(.whenPasscodeSetThisDeviceOnly)
    }
    func save(_ value: String, for account: String) throws { try keychain.set(Data(value.utf8), key: account) }
    /// `missingValue` only when the Keychain holds nothing under `account`. A
    /// failed read — the device locked, an entitlement missing — throws what
    /// the Keychain threw.
    func loadValue(for account: String) throws -> String {
        guard let data = try keychain.getData(account) else { throw KeychainStoreError.missingValue }
        guard let value = String(data: data, encoding: .utf8) else { throw KeychainStoreError.invalidEncoding }
        return value
    }
    func deleteValue(for account: String) throws { try keychain.remove(account) }
}

/// Wraps core's device key with a P-256 key held by the Secure Enclave.
///
/// The Keychain holds only the device key encrypted to an enclave key that
/// cannot leave this device's Secure Enclave, so a copied Keychain opens
/// nothing, and using the key needs this device, unlocked with a passcode set.
/// It asks for no user presence, so background work that derives from a seed
/// is unaffected.
///
/// The simulator has no enclave. There the wrapping key is a software key: the
/// logic is exercised, the hardware protection is not.
enum DeviceKeyWrapping {
    static let tag = Data("com.spectra.seed.masterkey.wrapping".utf8)
    private static let algorithm = SecKeyAlgorithm.eciesEncryptionCofactorVariableIVX963SHA256AESGCM

    /// Encrypt a device key to the enclave key, creating that key on first use.
    static func wrap(_ deviceKey: Data) throws -> Data {
        let privateKey = try wrappingKey(createIfMissing: true)
        guard let publicKey = SecKeyCopyPublicKey(privateKey) else {
            throw KeychainStoreError.keyWrapping("the wrapping key has no public key")
        }
        var error: Unmanaged<CFError>?
        guard let wrapped = SecKeyCreateEncryptedData(publicKey, algorithm, deviceKey as CFData, &error) as Data? else {
            throw KeychainStoreError.keyWrapping(describe(error))
        }
        return wrapped
    }

    /// Decrypt a stored device key. Never creates a wrapping key: a wrapped key
    /// whose wrapping key is gone is unreadable, not absent.
    static func unwrap(_ wrapped: Data) throws -> Data {
        let privateKey = try wrappingKey(createIfMissing: false)
        var error: Unmanaged<CFError>?
        guard let deviceKey = SecKeyCreateDecryptedData(privateKey, algorithm, wrapped as CFData, &error) as Data? else {
            throw KeychainStoreError.keyWrapping(describe(error))
        }
        return deviceKey
    }

    private static func wrappingKey(createIfMissing: Bool) throws -> SecKey {
        let query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: tag,
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecReturnRef as String: true,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecSuccess, let item { return item as! SecKey }
        guard status == errSecItemNotFound else {
            throw KeychainStoreError.keyWrapping("the wrapping key could not be read (\(status))")
        }
        guard createIfMissing else {
            throw KeychainStoreError.keyWrapping("no wrapping key is stored for a wrapped device key")
        }
        var attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeySizeInBits as String: 256,
        ]
        var privateAttributes: [String: Any] = [
            kSecAttrIsPermanent as String: true,
            kSecAttrApplicationTag as String: tag,
        ]
        #if targetEnvironment(simulator)
            privateAttributes[kSecAttrAccessible as String] = kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly
        #else
            attributes[kSecAttrTokenID as String] = kSecAttrTokenIDSecureEnclave
            var accessError: Unmanaged<CFError>?
            guard let access = SecAccessControlCreateWithFlags(
                nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, .privateKeyUsage, &accessError)
            else { throw KeychainStoreError.keyWrapping(describe(accessError)) }
            privateAttributes[kSecAttrAccessControl as String] = access
        #endif
        attributes[kSecPrivateKeyAttrs as String] = privateAttributes
        var error: Unmanaged<CFError>?
        guard let key = SecKeyCreateRandomKey(attributes as CFDictionary, &error) else {
            throw KeychainStoreError.keyWrapping(describe(error))
        }
        return key
    }

    private static func describe(_ error: Unmanaged<CFError>?) -> String {
        error.map { String(describing: $0.takeRetainedValue()) } ?? "unknown Security framework error"
    }
}

/// Core's secrets in the Keychain, one service per class.
///
/// Core seals seeds and private keys under its device key before they arrive,
/// and decides when that key is minted, reused or refused. What is left here
/// is what only this platform has: the Keychain, and the Secure Enclave key
/// that wraps the device key.
final class SpectraSecretStoreAdapter: SecretStore, Sendable {
    private static let seeds = KeychainBackedSecureStore(service: "com.spectra.seed")
    private static let privateKeys = KeychainBackedSecureStore(service: "com.spectra.privatekey")
    private static let generic = KeychainBackedSecureStore(service: "com.spectra.wallet")
    private static let deviceKey = KeychainBackedSecureStore(service: "com.spectra.seed.masterkey")

    private static func storage(_ kind: SecretClass) -> KeychainBackedSecureStore {
        switch kind {
        case .seed: return seeds
        case .privateKey: return privateKeys
        case .generic: return generic
        case .deviceKey: return deviceKey
        }
    }

    /// `NotFound` only for a value that is not there. Everything else — a
    /// locked device, a missing entitlement — is the store failing, and core
    /// refuses rather than reading it as absence.
    func loadSecret(kind: SecretClass, key: String) throws -> String {
        do { return try Self.storage(kind).loadValue(for: key) }
        catch KeychainStoreError.missingValue { throw SecretStoreError.NotFound }
        catch { throw SecretStoreError.Backend(message: String(describing: error)) }
    }
    func saveSecret(kind: SecretClass, key: String, value: String) throws {
        do { try Self.storage(kind).save(value, for: key) }
        catch { throw SecretStoreError.Backend(message: String(describing: error)) }
    }
    func deleteSecret(kind: SecretClass, key: String) throws {
        do { try Self.storage(kind).deleteValue(for: key) }
        catch { throw SecretStoreError.Backend(message: String(describing: error)) }
    }
    func wrapDeviceKey(key: Data) throws -> Data {
        do { return try DeviceKeyWrapping.wrap(key) }
        catch { throw SecretStoreError.Backend(message: String(describing: error)) }
    }
    func unwrapDeviceKey(wrapped: Data) throws -> Data {
        do { return try DeviceKeyWrapping.unwrap(wrapped) }
        catch { throw SecretStoreError.Backend(message: String(describing: error)) }
    }
}
