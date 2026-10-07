import Foundation
import Testing
@testable import Spectra

@MainActor
struct DeviceAuthenticationTests {
    @Test func disablingSendAuthenticationDoesNotDisableOtherProtectedActions() {
        for action in [DeviceAuthenticationAction.unlock, .deleteWallet, .resetData] {
            #expect(action.requiresAuthentication(useFaceId: true, authenticateSends: false))
        }
        #expect(!DeviceAuthenticationAction.send.requiresAuthentication(useFaceId: true, authenticateSends: false))
        #expect(DeviceAuthenticationAction.send.requiresAuthentication(useFaceId: true, authenticateSends: true))
    }

    @Test func usingSecretMaterialAlwaysRequiresAuthentication() {
        for useFaceId in [true, false] {
            for authenticateSends in [true, false] {
                #expect(DeviceAuthenticationAction.secretMaterial.requiresAuthentication(
                    useFaceId: useFaceId, authenticateSends: authenticateSends))
            }
        }
    }
}
