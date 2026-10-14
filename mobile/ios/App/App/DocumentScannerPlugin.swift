import Capacitor
import VisionKit
import Vision
import AVFoundation
import UIKit

final class TandemBridgeViewController: CAPBridgeViewController {
    override func capacitorDidLoad() {
        bridge?.registerPluginInstance(DocumentScannerPlugin())
        bridge?.registerPluginInstance(ChatGPTAccountPlugin())
    }
}

@objc(DocumentScannerPlugin)
public class DocumentScannerPlugin: CAPPlugin, CAPBridgedPlugin, VNDocumentCameraViewControllerDelegate {
    public let identifier = "DocumentScannerPlugin"
    public let jsName = "DocumentScanner"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "scan", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "recognize", returnType: CAPPluginReturnPromise)
    ]
    private var pendingScan: CAPPluginCall?
    private var pageLimit = 12

    @objc func scan(_ call: CAPPluginCall) {
        DispatchQueue.main.async {
            guard self.pendingScan == nil else { call.reject("A scan is already open."); return }
            guard VNDocumentCameraViewController.isSupported else { call.reject("The camera scanner is unavailable on this device. Use Add photos instead."); return }
            let limit = call.getInt("pageLimit") ?? 12
            guard limit > 0 && limit <= 12 else { call.reject("A scan can contain up to 12 pages."); return }
            self.pendingScan = call
            self.pageLimit = limit
            AVCaptureDevice.requestAccess(for: .video) { allowed in
                DispatchQueue.main.async {
                    guard allowed else { self.pendingScan = nil; call.reject("Allow camera access in Settings, or use Add photos."); return }
                    guard let host = self.bridge?.viewController else { self.pendingScan = nil; call.reject("The scanner could not open."); return }
                    let scanner = VNDocumentCameraViewController()
                    scanner.delegate = self
                    host.present(scanner, animated: true)
                }
            }
        }
    }

    public func documentCameraViewControllerDidCancel(_ controller: VNDocumentCameraViewController) {
        controller.dismiss(animated: true)
        pendingScan?.resolve(["images": []]); pendingScan = nil
    }
    public func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFailWithError error: Error) {
        controller.dismiss(animated: true)
        pendingScan?.reject("The camera could not scan this document. Try Add photos."); pendingScan = nil
    }
    public func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFinishWith scan: VNDocumentCameraScan) {
        controller.dismiss(animated: true)
        guard let call = pendingScan else { return }
        pendingScan = nil
        guard scan.pageCount <= pageLimit else { call.reject("Too many pages. Scan up to \(pageLimit) pages at a time."); return }
        DispatchQueue.global(qos: .userInitiated).async {
            var images: [String] = [], total = 0
            for index in 0..<scan.pageCount {
                let data: Data? = autoreleasepool {
                    let image = scan.imageOfPage(at: index)
                    let scale = min(1, 2400 / max(image.size.width, image.size.height))
                    let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
                    let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
                    return UIGraphicsImageRenderer(size: size, format: format).image { _ in
                        UIColor.white.setFill(); UIRectFill(CGRect(origin: .zero, size: size))
                        image.draw(in: CGRect(origin: .zero, size: size))
                    }.jpegData(compressionQuality: 0.88)
                }
                guard let data = data else { call.reject("One of the scanned pages could not be read."); return }
                total += data.count
                guard total <= 30 * 1024 * 1024 else { call.reject("The scans exceed 30 MB. Scan fewer pages."); return }
                images.append(data.base64EncodedString())
            }
            call.resolve(["images": images])
        }
    }

    @objc func recognize(_ call: CAPPluginCall) {
        guard let encoded = call.getString("data"), encoded.count <= 16_000_000,
              let data = Data(base64Encoded: encoded) else { call.reject("Invalid scan image."); return }
        DispatchQueue.global(qos: .userInitiated).async {
            autoreleasepool {
                do {
                    let lines = try Self.recognizeImage(data)
                    call.resolve(["lines": lines])
                } catch { call.reject("Text recognition failed. Try a sharper photo.") }
            }
        }
    }
    static func recognizeImage(_ data: Data) throws -> [[String: Any]] {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        if #available(iOS 16.0, *) { request.automaticallyDetectsLanguage = true }
        try VNImageRequestHandler(data: data, orientation: .up).perform([request])
        return (request.results ?? []).compactMap { observation in
            guard let text = observation.topCandidates(1).first?.string else { return nil }
            let bounds = observation.boundingBox
            return ["text": text, "x": max(0, bounds.minX), "y": max(0, 1 - bounds.maxY), "width": min(1 - bounds.minX, bounds.width), "height": min(bounds.maxY, bounds.height)]
        }
    }
}
