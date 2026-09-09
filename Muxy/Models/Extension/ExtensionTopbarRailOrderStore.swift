import Foundation

@MainActor
@Observable
final class ExtensionTopbarRailOrderStore {
    static let shared = ExtensionTopbarRailOrderStore()

    private let defaults: UserDefaults
    private let key: String
    @ObservationIgnored nonisolated(unsafe) private var observer: (any NSObjectProtocol)?

    var ids: [String] {
        didSet {
            persistIfNeeded()
        }
    }

    func reconcile(visibleRailIDs: [String], visibleNonRailIDs: [String]) {
        let reconciled = ExtensionTopbarRailOrder.persisting(
            visibleRailIDs: visibleRailIDs,
            visibleNonRailIDs: visibleNonRailIDs,
            savedIDs: ids
        )
        guard reconciled != ids else { return }
        ids = reconciled
    }

    init(
        defaults: UserDefaults = .standard,
        key: String = TopbarPreferences.railOrderKey
    ) {
        self.defaults = defaults
        self.key = key
        ids = Self.load(defaults: defaults, key: key)
        observer = NotificationCenter.default.addObserver(
            forName: UserDefaults.didChangeNotification,
            object: defaults,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.reloadFromDefaults()
            }
        }
    }

    deinit {
        if let observer {
            NotificationCenter.default.removeObserver(observer)
        }
    }

    private func reloadFromDefaults() {
        let loaded = Self.load(defaults: defaults, key: key)
        guard loaded != ids else { return }
        ids = loaded
    }

    private func persistIfNeeded() {
        let current = Self.load(defaults: defaults, key: key)
        guard current != ids else { return }
        defaults.set(ids, forKey: key)
    }

    private static func load(defaults: UserDefaults, key: String) -> [String] {
        defaults.stringArray(forKey: key) ?? TopbarPreferences.defaultRailOrder
    }
}
