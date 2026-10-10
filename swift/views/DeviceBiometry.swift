import LocalAuthentication

/// What this device proves its owner with, named as the device names it:
/// every prompt and switch says Touch ID on a Touch ID phone. A device with
/// no biometry falls back to its passcode, which the authentication policy
/// accepts too.
enum DeviceBiometry {
    case faceID, touchID, opticID, passcode

    /// The sensor does not change while the app runs, so it is read once.
    static let current: DeviceBiometry = {
        let context = LAContext()
        // `biometryType` is set only once a biometric policy has been asked
        // about; the answer itself does not matter here.
        _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
        switch context.biometryType {
        case .faceID: return .faceID
        case .touchID: return .touchID
        case .opticID: return .opticID
        default: return .passcode
        }
    }()

    @MainActor
    var name: String {
        switch self {
        case .faceID: AppLocalization.string("Face ID")
        case .touchID: AppLocalization.string("Touch ID")
        case .opticID: AppLocalization.string("Optic ID")
        case .passcode: AppLocalization.string("Passcode")
        }
    }

    var symbol: String {
        switch self {
        case .faceID: "faceid"
        case .touchID: "touchid"
        case .opticID: "opticid"
        case .passcode: "lock"
        }
    }
}
