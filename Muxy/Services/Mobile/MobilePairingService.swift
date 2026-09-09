import Darwin
import Foundation
import SystemConfiguration

enum MobilePairingNetwork: String, CaseIterable, Identifiable {
    case local
    case tailscale

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .local: "Local"
        case .tailscale: "Tailscale"
        }
    }
}

struct MobilePairingHost: Equatable {
    let network: MobilePairingNetwork
    let host: String
    let serviceName: String?
    let label: String?
}

enum MobilePairingService {
    static func localHostName() -> String? {
        guard let cfName = SCDynamicStoreCopyLocalHostName(nil) else { return nil }
        let value = cfName as String
        return value.isEmpty ? nil : value
    }

    static func availableHosts() -> [MobilePairingHost] {
        let name = localHostName()
        var hosts: [MobilePairingHost] = [
            MobilePairingHost(
                network: .local,
                host: "\(name ?? "localhost").local",
                serviceName: name,
                label: name
            ),
        ]
        if let tailscaleIP = tailscaleIPv4() {
            hosts.append(MobilePairingHost(
                network: .tailscale,
                host: tailscaleIP,
                serviceName: nil,
                label: name
            ))
        }
        return hosts
    }

    static func pairingURIString(for host: MobilePairingHost, port: UInt16) -> String? {
        MobilePairingURI.makeString(
            host: host.host,
            port: port,
            service: host.serviceName,
            label: host.label
        )
    }

    static func isTailscaleAddress(_ address: String) -> Bool {
        let parts = address.split(separator: ".")
        guard parts.count == 4 else { return false }
        guard let first = UInt8(parts[0]),
              let second = UInt8(parts[1]),
              UInt8(parts[2]) != nil,
              UInt8(parts[3]) != nil
        else { return false }
        return first == 100 && (64 ... 127).contains(second)
    }

    static func tailscaleIPv4() -> String? {
        var head: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&head) == 0, let first = head else { return nil }
        defer { freeifaddrs(head) }

        var pointer: UnsafeMutablePointer<ifaddrs>? = first
        while let current = pointer {
            defer { pointer = current.pointee.ifa_next }
            guard let addrPtr = current.pointee.ifa_addr,
                  addrPtr.pointee.sa_family == sa_family_t(AF_INET)
            else { continue }
            let name = String(cString: current.pointee.ifa_name)
            guard name.hasPrefix("utun") else { continue }
            var storage = sockaddr_in()
            memcpy(&storage, addrPtr, MemoryLayout<sockaddr_in>.size)
            var raw = storage.sin_addr
            var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
            guard inet_ntop(AF_INET, &raw, &buffer, socklen_t(INET_ADDRSTRLEN)) != nil else { continue }
            let bytes = buffer.prefix { $0 != 0 }.map(UInt8.init(bitPattern:))
            guard let ip = String(bytes: bytes, encoding: .utf8) else { continue }
            if isTailscaleAddress(ip) {
                return ip
            }
        }
        return nil
    }
}
