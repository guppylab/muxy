import AppKit
import SwiftUI

struct SymbolIcon: View {
    let name: String
    let size: CGFloat
    var weight: NSFont.Weight = .regular

    var body: some View {
        Image(nsImage: SymbolIconCache.shared.image(name: name, size: size, weight: weight))
            .renderingMode(.template)
    }
}

private final class SymbolIconCache: @unchecked Sendable {
    static let shared = SymbolIconCache()

    private let lock = NSLock()
    private var cache: [Key: NSImage] = [:]

    private struct Key: Hashable {
        let name: String
        let size: CGFloat
        let weight: CGFloat
    }

    func image(name: String, size: CGFloat, weight: NSFont.Weight) -> NSImage {
        let key = Key(name: name, size: size, weight: weight.rawValue)
        lock.lock()
        if let cached = cache[key] {
            lock.unlock()
            return cached
        }
        lock.unlock()

        let pointSize = max(size, 1)
        let configuration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: weight)
        let rendered = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(configuration)
            ?? NSImage(size: CGSize(width: pointSize, height: pointSize))
        rendered.isTemplate = true

        lock.lock()
        cache[key] = rendered
        lock.unlock()
        return rendered
    }
}
