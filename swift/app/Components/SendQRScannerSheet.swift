import SwiftUI
import UIKit
import Vision
import VisionKit

struct SendQRScannerSheet: View {
    @Environment(\.dismiss) private var dismiss
    let onScan: (String) -> Void
    var body: some View {
        NavigationStack {
            QRCodeScannerView { payload in
                onScan(payload)
                dismiss()
            }.ignoresSafeArea(edges: .bottom).navigationTitle(AppLocalization.string("Scan QR Code")).navigationBarTitleDisplayMode(
                .inline
            ).toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(AppLocalization.string("Cancel")) {
                        dismiss()
                    }
                }
            }
        }
    }
}
struct QRCodeScannerView: UIViewControllerRepresentable {
    let onScan: (String) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(onScan: onScan) }
    func makeUIViewController(context: Context) -> DataScannerViewController {
        let controller = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced, recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false, isPinchToZoomEnabled: true, isGuidanceEnabled: true, isHighlightingEnabled: true
        )
        controller.delegate = context.coordinator
        try? controller.startScanning()
        return controller
    }
    func updateUIViewController(_ uiViewController: DataScannerViewController, context: Context) {}
    static func dismantleUIViewController(_ uiViewController: DataScannerViewController, coordinator: Coordinator) {
        uiViewController.stopScanning()
    }
    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        private let onScan: (String) -> Void
        private var hasResolvedPayload = false
        init(onScan: @escaping (String) -> Void) {
            self.onScan = onScan
        }
        func dataScanner(_ dataScanner: DataScannerViewController, didTapOn item: RecognizedItem) { resolve(item, from: dataScanner) }
        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            guard let firstItem = addedItems.first else { return }
            resolve(firstItem, from: dataScanner)
        }
        private func resolve(_ item: RecognizedItem, from dataScanner: DataScannerViewController) {
            guard !hasResolvedPayload else { return }
            guard case .barcode(let barcode) = item,
                let payload = barcode.payloadStringValue?.trimmingCharacters(in: .whitespacesAndNewlines), !payload.isEmpty
            else { return }
            hasResolvedPayload = true
            dataScanner.stopScanning()
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            onScan(payload)
        }
    }
}
