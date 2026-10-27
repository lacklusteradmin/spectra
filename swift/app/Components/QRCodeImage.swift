import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation
import SwiftUI
import UIKit

struct QRCodeImage: View {
    let address: String
    var body: some View {
        Group {
            if let image = QRCodeRenderer.makeImage(from: address) {
                Image(uiImage: image).interpolation(.none).resizable().scaledToFit()
            } else {
                Image(systemName: "qrcode").resizable().scaledToFit().padding(SpectraLayout.Space.xl).foregroundStyle(.black)
            }
        }
        .accessibilityLabel(AppLocalization.string("QR code of the address"))
    }
}

struct QRCodeRenderer {
    static func makeImage(from string: String) -> UIImage? {
        let context = CIContext()
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(string.utf8)
        filter.correctionLevel = "M"
        guard let outputImage = filter.outputImage else { return nil }
        let scaledImage = outputImage.transformed(by: CGAffineTransform(scaleX: 12, y: 12))
        guard let cgImage = context.createCGImage(scaledImage, from: scaledImage.extent) else { return nil }
        return UIImage(cgImage: cgImage)
    }
}
