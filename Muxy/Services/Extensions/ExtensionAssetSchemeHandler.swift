import Foundation
import UniformTypeIdentifiers
@preconcurrency import WebKit

final class ExtensionAssetSchemeHandler: NSObject, WKURLSchemeHandler {
    private struct SchemeTask: @unchecked Sendable {
        let value: any WKURLSchemeTask
    }

    nonisolated static let scheme = "muxy-ext"
    nonisolated static let maxAssetBytes: Int = 64 * 1024 * 1024

    nonisolated private let extensionID: String
    nonisolated private let directory: URL
    nonisolated private let ioQueue = DispatchQueue(label: "app.muxy.extension-assets", qos: .userInitiated)
    nonisolated private let activeTasksLock = NSLock()
    nonisolated(unsafe) private var activeTasks: Set<ObjectIdentifier> = []

    nonisolated init(extensionID: String, directory: URL) {
        self.extensionID = extensionID
        self.directory = directory.standardizedFileURL
    }

    func webView(_: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
        let taskID = ObjectIdentifier(urlSchemeTask)
        registerActive(taskID)

        guard let url = urlSchemeTask.request.url,
              url.scheme == Self.scheme,
              url.host == extensionID
        else {
            failIfActive(urlSchemeTask, taskID: taskID, error: URLError(.badURL))
            return
        }

        let relativePath = url.path.isEmpty ? "" : String(url.path.dropFirst())
        let resolved = directory
            .appendingPathComponent(relativePath)
            .standardizedFileURL
            .resolvingSymlinksInPath()
        let base = directory.resolvingSymlinksInPath()

        guard resolved.path == base.path || resolved.path.hasPrefix(base.path + "/") else {
            failIfActive(urlSchemeTask, taskID: taskID, error: URLError(.noPermissionsToReadFile))
            return
        }

        let task = SchemeTask(value: urlSchemeTask)
        ioQueue.async { [weak self, task] in
            guard let self else { return }
            let attributes = try? FileManager.default.attributesOfItem(atPath: resolved.path)
            let size = (attributes?[.size] as? Int) ?? 0
            if size > Self.maxAssetBytes {
                self.failIfActive(task.value, taskID: taskID, error: URLError(.dataLengthExceedsMaximum))
                return
            }
            guard let data = try? Data(contentsOf: resolved) else {
                self.failIfActive(task.value, taskID: taskID, error: URLError(.fileDoesNotExist))
                return
            }
            let response = HTTPURLResponse(
                url: url,
                statusCode: 200,
                httpVersion: "HTTP/1.1",
                headerFields: [
                    "Content-Type": Self.mimeType(for: resolved),
                    "Content-Length": String(data.count),
                    "Cache-Control": "no-store",
                ]
            )
            guard let response else { return }
            self.finishIfActive(task.value, taskID: taskID, response: response, data: data)
        }
    }

    func webView(_: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {
        let taskID = ObjectIdentifier(urlSchemeTask)
        activeTasksLock.lock()
        activeTasks.remove(taskID)
        activeTasksLock.unlock()
    }

    nonisolated private func registerActive(_ taskID: ObjectIdentifier) {
        activeTasksLock.lock()
        activeTasks.insert(taskID)
        activeTasksLock.unlock()
    }

    nonisolated private func consumeActive(_ taskID: ObjectIdentifier) -> Bool {
        activeTasksLock.lock()
        defer { activeTasksLock.unlock() }
        return activeTasks.remove(taskID) != nil
    }

    nonisolated private func failIfActive(_ task: WKURLSchemeTask, taskID: ObjectIdentifier, error: Error) {
        guard consumeActive(taskID) else { return }
        task.didFailWithError(error)
    }

    nonisolated private func finishIfActive(
        _ task: WKURLSchemeTask,
        taskID: ObjectIdentifier,
        response: URLResponse,
        data: Data
    ) {
        guard consumeActive(taskID) else { return }
        task.didReceive(response)
        task.didReceive(data)
        task.didFinish()
    }

    nonisolated private static func mimeType(for url: URL) -> String {
        let ext = url.pathExtension.lowercased()
        switch ext {
        case "html",
             "htm": return "text/html; charset=utf-8"
        case "js",
             "mjs": return "application/javascript; charset=utf-8"
        case "css": return "text/css; charset=utf-8"
        case "json": return "application/json; charset=utf-8"
        case "svg": return "image/svg+xml"
        case "png": return "image/png"
        case "jpg",
             "jpeg": return "image/jpeg"
        case "gif": return "image/gif"
        case "webp": return "image/webp"
        case "wasm": return "application/wasm"
        case "ico": return "image/x-icon"
        default:
            if let type = UTType(filenameExtension: ext),
               let mime = type.preferredMIMEType
            {
                return mime
            }
            return "application/octet-stream"
        }
    }
}
