import Foundation
import JavaScriptCore
import MuxyShared

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("[muxy-extension-host] \(message)\n".utf8))
    exit(1)
}

let parentDeathMonitor = ParentDeathMonitor()
parentDeathMonitor.start()

let environment = ProcessInfo.processInfo.environment

guard let scriptPath = CommandLine.arguments.dropFirst().first else {
    fail("missing background script path argument")
}

guard let socketPath = environment["MUXY_SOCKET_PATH"] else {
    fail("missing MUXY_SOCKET_PATH")
}

guard let extensionID = environment["MUXY_EXTENSION_ID"] else {
    fail("missing MUXY_EXTENSION_ID")
}

let token = environment["MUXY_EXTENSION_TOKEN"] ?? ""

guard let source = try? String(contentsOfFile: scriptPath, encoding: .utf8) else {
    fail("could not read background script at \(scriptPath)")
}

let client: HostSocketClient
do {
    client = try HostSocketClient(socketPath: socketPath)
} catch {
    fail("could not connect to Muxy socket: \(error)")
}

guard let context = JSContext() else {
    fail("could not create JSContext")
}

let bridge = HostBridge(client: client, extensionID: extensionID, context: context)
bridge.install()

client.onEvent { [weak bridge] line in
    bridge?.handleEventLine(line)
}

client.onExtensionEvent { [weak bridge] line in
    bridge?.handleExtensionEventLine(line)
}

client.onInvoke { [weak bridge] line in
    bridge?.handleInvokeLine(line)
}

client.onModalResult { [weak bridge] line in
    bridge?.handleModalResultLine(line)
}

client.onModalQuery { [weak bridge] line in
    bridge?.handleModalQueryLine(line)
}

client.startReading()

func identify() {
    let maxAttempts = HostSocketClient.maxIdentifyAttempts
    var lastReply = ""
    for attempt in 1 ... maxAttempts {
        do {
            let reply = try client.sendAndWaitReply("identify|\(extensionID)|\(token)")
            if reply == "ok" {
                return
            }
            lastReply = reply
            if !HostSocketClient.isTransientIdentifyRejection(reply) || attempt == maxAttempts {
                fail("identify rejected: \(reply)")
            }
            Thread.sleep(forTimeInterval: HostSocketClient.identifyRetryDelay)
        } catch {
            fail("identify failed: \(error)")
        }
    }
    fail("identify rejected: \(lastReply)")
}

identify()

context.exceptionHandler = { _, exception in
    let message = exception?.toString() ?? "unknown error"
    FileHandle.standardError.write(Data("[muxy-extension-host] \(extensionID) error: \(message)\n".utf8))
}

context.evaluateScript(source, withSourceURL: URL(fileURLWithPath: scriptPath))

let runLoop = RunLoop.current
while !client.isClosed, runLoop.run(mode: .default, before: .distantFuture) {}
