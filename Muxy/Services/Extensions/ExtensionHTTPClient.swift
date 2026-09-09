import Foundation

struct HTTPRequest {
    let url: String
    let method: String
    let headers: [String: String]?
    let body: String?
    let timeoutMs: Int?
}

struct HTTPResult {
    let status: Int
    let headers: [String: String]
    let body: String
    let truncated: Bool
}

enum HTTPError: Error, LocalizedError {
    case invalidArguments(String)
    case blockedHost(String)
    case requestFailed(String)

    var errorDescription: String? {
        switch self {
        case let .invalidArguments(detail): "http: \(detail)"
        case let .blockedHost(host): "http: blocked request to private or loopback host '\(host)'"
        case let .requestFailed(detail): "http request failed: \(detail)"
        }
    }
}

enum ExtensionHTTPClient {
    static let defaultTimeoutMs = 30000
    static let maxResourceTimeoutMs = 120_000
    static let maxResponseBytes = 10 * 1024 * 1024
    static let allowedMethods: Set<String> = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]
    private static let forbiddenHeaders: Set<String> = ["host", "content-length", "connection"]

    @MainActor
    static func fetch(
        request: HTTPRequest,
        extensionID: String,
        session: URLSession = .shared,
        consentService: ExtensionConsentService = .shared
    ) async throws -> HTTPResult {
        let urlRequest = try buildRequest(request)
        guard let host = urlRequest.url?.host else {
            throw HTTPError.invalidArguments("URL has no host")
        }
        guard !HostSecurityPolicy.isBlocked(host) else {
            throw HTTPError.blockedHost(host)
        }

        let consentRequest = ExtensionConsentRequestBuilder.make(
            extensionID: extensionID,
            verb: .httpFetch,
            payload: .http(hostname: host, method: urlRequest.httpMethod ?? "GET", url: request.url),
            source: "http"
        )
        let decision = await consentService.gate(consentRequest)
        guard decision == .allow else {
            throw HTTPError.requestFailed("user denied consent for \(host)")
        }

        return try await perform(urlRequest, session: session)
    }

    private static func buildRequest(_ request: HTTPRequest) throws -> URLRequest {
        let trimmed = request.url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: trimmed), let scheme = url.scheme?.lowercased() else {
            throw HTTPError.invalidArguments("invalid URL")
        }
        guard scheme == "http" || scheme == "https" else {
            throw HTTPError.invalidArguments("only http and https URLs are allowed")
        }

        let method = request.method.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        let resolvedMethod = method.isEmpty ? "GET" : method
        guard allowedMethods.contains(resolvedMethod) else {
            throw HTTPError.invalidArguments("unsupported method '\(resolvedMethod)'")
        }

        let timeoutMs = request.timeoutMs ?? defaultTimeoutMs
        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = resolvedMethod
        urlRequest.timeoutInterval = TimeInterval(max(timeoutMs, 1)) / 1000

        if let headers = request.headers {
            for (key, value) in headers where !forbiddenHeaders.contains(key.lowercased()) {
                urlRequest.setValue(value, forHTTPHeaderField: key)
            }
        }
        if let body = request.body, !body.isEmpty {
            urlRequest.httpBody = Data(body.utf8)
        }
        return urlRequest
    }

    private static func perform(
        _ request: URLRequest,
        session: URLSession
    ) async throws -> HTTPResult {
        let delegate = HTTPRedirectGuard()
        let guardedSession = guardedSession(from: session, delegate: delegate)
        defer { guardedSession.invalidateAndCancel() }

        do {
            let (stream, response) = try await guardedSession.bytes(for: request)
            guard let httpResponse = response as? HTTPURLResponse else {
                throw HTTPError.requestFailed("unexpected response")
            }
            let (data, truncated) = try await collect(stream)
            return HTTPResult(
                status: httpResponse.statusCode,
                headers: stringHeaders(httpResponse.allHeaderFields),
                body: String(bytes: data, encoding: .utf8) ?? "",
                truncated: truncated
            )
        } catch let error as HTTPError {
            throw error
        } catch {
            throw HTTPError.requestFailed(error.localizedDescription)
        }
    }

    private static func guardedSession(from session: URLSession, delegate: HTTPRedirectGuard) -> URLSession {
        let configuration = session.configuration
        configuration.timeoutIntervalForResource = TimeInterval(maxResourceTimeoutMs) / 1000
        return URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
    }

    private static func collect(_ stream: URLSession.AsyncBytes) async throws -> (Data, truncated: Bool) {
        var data = Data()
        data.reserveCapacity(min(maxResponseBytes, 1 << 16))
        for try await byte in stream {
            if data.count >= maxResponseBytes {
                return (data, true)
            }
            data.append(byte)
        }
        return (data, false)
    }

    private static func stringHeaders(_ raw: [AnyHashable: Any]) -> [String: String] {
        var headers: [String: String] = [:]
        for (key, value) in raw {
            guard let name = key as? String else { continue }
            headers[name] = String(describing: value)
        }
        return headers
    }
}

private final class HTTPRedirectGuard: NSObject, URLSessionTaskDelegate {
    func urlSession(
        _: URLSession,
        task _: URLSessionTask,
        willPerformHTTPRedirection _: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        guard let host = request.url?.host, !HostSecurityPolicy.isBlocked(host) else {
            completionHandler(nil)
            return
        }
        completionHandler(request)
    }
}

enum HostSecurityPolicy {
    static func isBlocked(_ host: String) -> Bool {
        let normalized = host.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
        if normalized.isEmpty {
            return true
        }
        if normalized == "localhost" || normalized.hasSuffix(".localhost") {
            return true
        }
        if normalized.hasSuffix(".local") {
            return true
        }

        guard let addresses = resolvedAddresses(for: normalized) else { return true }
        return addresses.contains(where: isPrivateAddress)
    }

    private static func resolvedAddresses(for host: String) -> [String]? {
        var hints = addrinfo(
            ai_flags: 0,
            ai_family: AF_UNSPEC,
            ai_socktype: SOCK_STREAM,
            ai_protocol: 0,
            ai_addrlen: 0,
            ai_canonname: nil,
            ai_addr: nil,
            ai_next: nil
        )
        var result: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(host, nil, &hints, &result) == 0, let first = result else { return nil }
        defer { freeaddrinfo(result) }

        var addresses: [String] = []
        var node: UnsafeMutablePointer<addrinfo>? = first
        while let current = node {
            var buffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(
                current.pointee.ai_addr,
                current.pointee.ai_addrlen,
                &buffer,
                socklen_t(buffer.count),
                nil,
                0,
                NI_NUMERICHOST
            ) == 0 {
                let bytes = buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }
                if let address = String(bytes: bytes, encoding: .utf8) {
                    addresses.append(address)
                }
            }
            node = current.pointee.ai_next
        }
        return addresses.isEmpty ? nil : addresses
    }

    private static func isPrivateAddress(_ address: String) -> Bool {
        let host = address.split(separator: "%").first.map(String.init) ?? address
        if let v4 = ipv4Octets(host) {
            return isPrivateIPv4(v4)
        }
        return isPrivateIPv6(host.lowercased())
    }

    private static func ipv4Octets(_ host: String) -> [Int]? {
        let parts = host.split(separator: ".", omittingEmptySubsequences: false).map(String.init)
        guard parts.count == 4 else { return nil }
        let octets = parts.compactMap { Int($0) }
        guard octets.count == 4, octets.allSatisfy({ (0 ... 255).contains($0) }) else { return nil }
        return octets
    }

    private static func isPrivateIPv4(_ octets: [Int]) -> Bool {
        switch (octets[0], octets[1]) {
        case (10, _),
             (127, _),
             (0, _),
             (169, 254):
            true
        case (192, 168):
            true
        case (172, 16 ... 31):
            true
        default:
            false
        }
    }

    private static func isPrivateIPv6(_ host: String) -> Bool {
        if host == "::1" || host == "::" {
            return true
        }
        if host.hasPrefix("fe80") || host.hasPrefix("fc") || host.hasPrefix("fd") {
            return true
        }
        if host.hasPrefix("::ffff:") {
            let mapped = String(host.dropFirst("::ffff:".count))
            if let v4 = ipv4Octets(mapped) {
                return isPrivateIPv4(v4)
            }
        }
        return false
    }
}
