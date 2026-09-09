import AppKit
import CoreGraphics
import Foundation
import OSLog
import WebKit

private let browserAutomationLogger = Logger(subsystem: "app.muxy", category: "BrowserAutomation")

struct BrowserCookieInfo: Equatable {
    let name: String
    let value: String
    let domain: String
    let path: String
    let secure: Bool
    let httpOnly: Bool
    let expires: Double?
}

extension MuxyAPI.Browser {
    enum StorageKind: String {
        case local
        case session
    }

    static func eval(tabIDString: String, script: String, appState: AppState) async -> Result<String, APIError> {
        let isStatementBody = script.contains(";") || script.contains("\n")
        let body = isStatementBody ? script : "return (\(script));"
        return await runScript(tabIDString: tabIDString, appState: appState, body: body)
    }

    static func click(tabIDString: String, selector: String, appState: AppState) async -> Result<Bool, APIError> {
        await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        if (!el) { return false; }
        el.click();
        return true;
        """)
    }

    static func type(
        tabIDString: String,
        selector: String,
        text: String,
        submit: Bool,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        if (!el) { return false; }
        el.focus();
        const setter = Object.getOwnPropertyDescriptor(el.__proto__, 'value');
        if (setter && setter.set) { setter.set.call(el, \(jsString(text))); }
        else { el.value = \(jsString(text)); }
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
        if (\(submit ? "true" : "false")) {
            const form = el.form;
            if (form) { form.requestSubmit ? form.requestSubmit() : form.submit(); }
            else { el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); }
        }
        return true;
        """)
    }

    static func waitFor(
        tabIDString: String,
        selector: String,
        timeoutMs: Int,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        await pollCondition(
            tabIDString: tabIDString,
            timeoutMs: timeoutMs,
            appState: appState,
            expression: "document.querySelector(\(jsString(selector)))"
        )
    }

    static func getText(tabIDString: String, selector: String, appState: AppState) async -> Result<String?, APIError> {
        await optionalStringScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        return el ? el.innerText : null;
        """)
    }

    static func getHTML(tabIDString: String, selector: String?, appState: AppState) async -> Result<String?, APIError> {
        let target = selector.map { "document.querySelector(\(jsString($0)))" } ?? "document.documentElement"
        return await optionalStringScript(tabIDString: tabIDString, appState: appState, body: """
        const el = \(target);
        if (!el) { return null; }
        return (el.outerHTML || '').slice(0, \(maxStringLength));
        """)
    }

    static func getAttribute(
        tabIDString: String,
        selector: String,
        attribute: String,
        appState: AppState
    ) async -> Result<String?, APIError> {
        await optionalStringScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        return el ? el.getAttribute(\(jsString(attribute))) : null;
        """)
    }

    struct WaitCondition {
        let selector: String?
        let text: String?
        let urlContains: String?
        let function: String?
    }

    static func wait(
        tabIDString: String,
        condition: WaitCondition,
        timeoutMs: Int,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        await pollCondition(
            tabIDString: tabIDString,
            timeoutMs: timeoutMs,
            appState: appState,
            expression: waitConditionExpression(condition)
        )
    }

    static func fill(
        tabIDString: String,
        selector: String,
        text: String,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        if (!el) { return false; }
        el.focus();
        const setter = Object.getOwnPropertyDescriptor(el.__proto__, 'value');
        if (setter && setter.set) { setter.set.call(el, \(jsString(text))); }
        else { el.value = \(jsString(text)); }
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
        return true;
        """)
    }

    static func press(
        tabIDString: String,
        selector: String?,
        key: String,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        let target = selector.map { "document.querySelector(\(jsString($0)))" } ?? "(document.activeElement || document.body)"
        return await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = \(target);
        if (!el) { return false; }
        const key = \(jsString(key));
        for (const type of ['keydown', 'keypress', 'keyup']) {
            el.dispatchEvent(new KeyboardEvent(type, { key, bubbles: true }));
        }
        return true;
        """)
    }

    static func selectOption(
        tabIDString: String,
        selector: String,
        value: String,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        if (!el) { return false; }
        el.value = \(jsString(value));
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
        return true;
        """)
    }

    static func hover(tabIDString: String, selector: String, appState: AppState) async -> Result<Bool, APIError> {
        await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        if (!el) { return false; }
        for (const type of ['mouseover', 'mouseenter', 'mousemove']) {
            el.dispatchEvent(new MouseEvent(type, { bubbles: true }));
        }
        return true;
        """)
    }

    static func scrollIntoView(
        tabIDString: String,
        selector: String,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        if (!el) { return false; }
        el.scrollIntoView({ block: 'center', inline: 'center' });
        return true;
        """)
    }

    static func setChecked(
        tabIDString: String,
        selector: String,
        checked: Bool,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        if (!el) { return false; }
        if (el.checked !== \(checked ? "true" : "false")) {
            el.click();
        }
        return true;
        """)
    }

    static func isState(
        tabIDString: String,
        selector: String,
        property: String,
        appState: AppState
    ) async -> Result<Bool, APIError> {
        let expression = isExpression(property: property)
        return await boolScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        if (!el) { return false; }
        return Boolean(\(expression));
        """)
    }

    static func getValue(tabIDString: String, selector: String, appState: AppState) async -> Result<String?, APIError> {
        await optionalStringScript(tabIDString: tabIDString, appState: appState, body: """
        const el = document.querySelector(\(jsString(selector)));
        return el && el.value != null ? String(el.value) : null;
        """)
    }

    static func getCount(tabIDString: String, selector: String, appState: AppState) async -> Result<Int, APIError> {
        await runAsyncResult(tabIDString: tabIDString, appState: appState, body: """
        return document.querySelectorAll(\(jsString(selector))).length;
        """) { ($0 as? Int) ?? (($0 as? NSNumber)?.intValue ?? 0) }
    }

    static func find(
        tabIDString: String,
        kind: String,
        value: String,
        appState: AppState
    ) async -> Result<String, APIError> {
        let matcher = findMatcher(kind: kind, value: value)
        return await runScript(tabIDString: tabIDString, appState: appState, body: """
        const all = Array.from(document.querySelectorAll('*'));
        const match = (el) => { try { return \(matcher); } catch (e) { return false; } };
        const matched = all.filter(match);
        const leaves = matched.filter((el) => !matched.some((other) => other !== el && el.contains(other)));
        const found = leaves.slice(0, 20).map((el) => ({
            tag: el.tagName.toLowerCase(),
            text: (el.innerText || el.value || '').trim().slice(0, 120),
            role: el.getAttribute('role'),
            id: el.id || null,
            testid: el.getAttribute('data-testid'),
        }));
        return found;
        """)
    }

    static func snapshot(
        tabIDString: String,
        selector: String?,
        appState: AppState
    ) async -> Result<String, APIError> {
        let root = selector.map { "document.querySelector(\(jsString($0)))" } ?? "document.body"
        return await runScript(tabIDString: tabIDString, appState: appState, body: """
        const root = \(root);
        if (!root) { return []; }
        const interactive = 'a, button, input, textarea, select, [role=button], [role=link], [role=tab], [onclick], [contenteditable=true]';
        const nodes = Array.from(root.querySelectorAll(interactive));
        return nodes.slice(0, 200).map((el) => {
            const rect = el.getBoundingClientRect();
            const visible = rect.width > 0 && rect.height > 0;
            return {
                tag: el.tagName.toLowerCase(),
                role: el.getAttribute('role') || el.type || null,
                name: (el.innerText || el.value || el.getAttribute('aria-label')
                    || el.getAttribute('placeholder') || '').trim().slice(0, 120),
                id: el.id || null,
                testid: el.getAttribute('data-testid'),
                visible,
            };
        }).filter((n) => n.visible);
        """)
    }

    static func navigation(
        tabIDString: String,
        command: BrowserTabState.NavigationCommand,
        appState: AppState
    ) async -> Result<Void, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        let start = ContinuousClock.now
        guard let resolved = await resolve(tabIDString: tabIDString, appState: appState, start: start) else {
            return surfaceFailure(tabIDString: tabIDString, start: start, appState: appState)
        }
        switch command {
        case .back: resolved.webView.goBack()
        case .forward: resolved.webView.goForward()
        case .reload: resolved.webView.reload()
        default: resolved.state.pendingCommand = command
        }
        return .success(())
    }

    static func waitForNavigation(
        tabIDString: String,
        timeoutMs: Int,
        appState: AppState
    ) async -> Result<String?, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        guard let state = browserState(tabIDString: tabIDString, appState: appState) else {
            return .failure(.browserTabNotFound(tabIDString))
        }
        let bounded = max(0, min(timeoutMs, maxWaitMilliseconds))
        let deadline = ContinuousClock.now + .milliseconds(bounded)
        while ContinuousClock.now < deadline {
            if !state.isLoading {
                return .success(state.url?.absoluteString)
            }
            do { try await Task.sleep(for: .milliseconds(100)) } catch { break }
        }
        return .success(state.url?.absoluteString)
    }

    static func screenshot(tabIDString: String, appState: AppState) async -> Result<String, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        let start = ContinuousClock.now
        guard let resolved = await resolve(tabIDString: tabIDString, appState: appState, start: start) else {
            return surfaceFailure(tabIDString: tabIDString, start: start, appState: appState)
        }
        guard let png = await capturePNG(webView: resolved.webView) else {
            return .failure(.browserTabSurfaceNotReady(
                tabID: tabIDString,
                waitedSeconds: (ContinuousClock.now - start).secondsValue
            ))
        }
        return .success(png.base64EncodedString())
    }

    private static let defaultSnapshotSize = CGSize(width: 1200, height: 800)
    private static let captureTimeout: Duration = .seconds(8)

    private static func capturePNG(webView: WKWebView) async -> Data? {
        let rect = snapshotRect(for: webView)
        guard let pdf = await createPDFWithTimeout(webView: webView, rect: rect) else {
            return nil
        }
        return renderPDFToPNG(pdf, rect: rect)
    }

    private static func createPDFWithTimeout(webView: WKWebView, rect: CGRect) async -> Data? {
        let config = WKPDFConfiguration()
        config.rect = rect

        let box = PDFResultBox()
        let timeoutTask = Task { @MainActor in
            try? await Task.sleep(for: captureTimeout)
            box.resume(with: nil)
        }
        let data = await withCheckedContinuation { (continuation: CheckedContinuation<Data?, Never>) in
            box.attach(continuation)
            webView.createPDF(configuration: config) { result in
                switch result {
                case let .success(data): box.resume(with: data)
                case .failure: box.resume(with: nil)
                }
            }
        }
        timeoutTask.cancel()
        return data
    }

    private static func renderPDFToPNG(_ pdfData: Data, rect: CGRect) -> Data? {
        guard let provider = CGDataProvider(data: pdfData as CFData),
              let document = CGPDFDocument(provider),
              let page = document.page(at: 1)
        else { return nil }

        let scale: CGFloat = 2
        let pageRect = page.getBoxRect(.mediaBox)
        let pixelWidth = Int(pageRect.width * scale)
        let pixelHeight = Int(pageRect.height * scale)
        guard pixelWidth > 0, pixelHeight > 0,
              let context = CGContext(
                  data: nil,
                  width: pixelWidth,
                  height: pixelHeight,
                  bitsPerComponent: 8,
                  bytesPerRow: 0,
                  space: CGColorSpaceCreateDeviceRGB(),
                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
              )
        else { return nil }

        context.setFillColor(NSColor.white.cgColor)
        context.fill(CGRect(x: 0, y: 0, width: pixelWidth, height: pixelHeight))
        context.scaleBy(x: scale, y: scale)
        context.drawPDFPage(page)

        guard let cgImage = context.makeImage() else { return nil }
        let bitmap = NSBitmapImageRep(cgImage: cgImage)
        return bitmap.representation(using: .png, properties: [:])
    }

    private static func snapshotRect(for webView: WKWebView) -> CGRect {
        let bounds = webView.bounds
        if bounds.width > 0, bounds.height > 0 {
            return bounds
        }
        let fallback = webView.window?.contentView?.bounds.size
            ?? AppDelegate.mainAppWindow()?.contentView?.bounds.size
            ?? defaultSnapshotSize
        return CGRect(origin: .zero, size: fallback)
    }

    static func storageGet(
        tabIDString: String,
        kind: StorageKind,
        key: String,
        appState: AppState
    ) async -> Result<String?, APIError> {
        await optionalStringScript(tabIDString: tabIDString, appState: appState, body: """
        return window.\(kind.rawValue)Storage.getItem(\(jsString(key)));
        """)
    }

    static func storageSet(
        tabIDString: String,
        kind: StorageKind,
        key: String,
        value: String,
        appState: AppState
    ) async -> Result<Void, APIError> {
        let result = await runScript(tabIDString: tabIDString, appState: appState, body: """
        window.\(kind.rawValue)Storage.setItem(\(jsString(key)), \(jsString(value)));
        return null;
        """)
        return result.map { _ in () }
    }

    static func storageClear(
        tabIDString: String,
        kind: StorageKind,
        appState: AppState
    ) async -> Result<Void, APIError> {
        let result = await runScript(tabIDString: tabIDString, appState: appState, body: """
        window.\(kind.rawValue)Storage.clear();
        return null;
        """)
        return result.map { _ in () }
    }

    static func cookiesGet(
        tabIDString: String,
        urlString: String?,
        appState: AppState
    ) async -> Result<[BrowserCookieInfo], APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        guard let store = cookieStore(tabIDString: tabIDString, appState: appState) else {
            return .failure(.browserTabNotFound(tabIDString))
        }
        let host = urlString.flatMap(BrowserURL.resolve(from:))?.host
        let cookies = await allCookies(in: store)
        let filtered = cookies.filter { cookie in
            guard let host else { return true }
            return cookie.domainMatches(host: host)
        }
        return .success(filtered.map(BrowserCookieInfo.init(cookie:)))
    }

    static func cookiesSet(
        tabIDString: String,
        cookie: BrowserCookieInfo,
        appState: AppState
    ) async -> Result<Void, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        guard let store = cookieStore(tabIDString: tabIDString, appState: appState) else {
            return .failure(.browserTabNotFound(tabIDString))
        }
        guard let httpCookie = cookie.makeHTTPCookie() else {
            return .failure(.invalidArguments("invalid cookie"))
        }
        await store.setCookie(httpCookie)
        return .success(())
    }

    static func cookiesDelete(
        tabIDString: String,
        name: String,
        domain: String?,
        appState: AppState
    ) async -> Result<Void, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        guard let store = cookieStore(tabIDString: tabIDString, appState: appState) else {
            return .failure(.browserTabNotFound(tabIDString))
        }
        let cookies = await allCookies(in: store)
        for cookie in cookies where cookie.name == name && (domain == nil || cookie.domain == domain) {
            await store.deleteCookie(cookie)
        }
        return .success(())
    }

    static func cookiesClear(tabIDString: String, appState: AppState) async -> Result<Void, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        guard let store = cookieStore(tabIDString: tabIDString, appState: appState) else {
            return .failure(.browserTabNotFound(tabIDString))
        }
        let cookies = await allCookies(in: store)
        for cookie in cookies {
            await store.deleteCookie(cookie)
        }
        return .success(())
    }

    private static func allCookies(in store: WKHTTPCookieStore) async -> [HTTPCookie] {
        await allCookiesAttempt(in: store) ?? []
    }

    private static func allCookiesAttempt(in store: WKHTTPCookieStore) async -> [HTTPCookie]? {
        await cookieFetchCoordinator.value(for: store, timeout: .milliseconds(cookieAttemptTimeoutMs)) {
            await store.allCookies()
        }
    }

    private static let maxStringLength = 5_000_000
    private static let maxWaitMilliseconds = 60000
    private static let cookieAttemptTimeoutMs = 800
    private static let cookieFetchCoordinator = CookieFetchCoordinator<[HTTPCookie]>()

    private struct ResolvedWebView {
        let state: BrowserTabState
        let webView: WKWebView
    }

    private struct LocatedState {
        let isLoading: Bool
        let url: URL?
    }

    private static func locateState(tabIDString: String, appState: AppState) -> LocatedState? {
        guard let id = UUID(uuidString: tabIDString) else { return nil }
        for (_, root) in appState.workspaceRoots {
            for area in root.allAreas() {
                for tab in area.tabs where tab.id == id {
                    guard let state = tab.content.browserState else { return nil }
                    return LocatedState(isLoading: state.isLoading, url: state.url)
                }
            }
        }
        return nil
    }

    private static func browserState(tabIDString: String, appState: AppState) -> BrowserTabState? {
        guard let id = UUID(uuidString: tabIDString) else { return nil }
        for (_, root) in appState.workspaceRoots {
            for area in root.allAreas() {
                for tab in area.tabs where tab.id == id {
                    return tab.content.browserState
                }
            }
        }
        return nil
    }

    private static func cookieStore(tabIDString: String, appState: AppState) -> WKHTTPCookieStore? {
        guard let state = browserState(tabIDString: tabIDString, appState: appState) else { return nil }
        if let webView = BrowserWebViewRegistry.shared.webView(for: state.id) {
            return webView.configuration.websiteDataStore.httpCookieStore
        }
        return BrowserDataStoreCache.shared.store(for: state.profileID).httpCookieStore
    }

    private static func resolve(
        tabIDString: String,
        appState: AppState,
        start: ContinuousClock.Instant
    ) async -> ResolvedWebView? {
        guard let state = browserState(tabIDString: tabIDString, appState: appState) else { return nil }
        guard let webView = await waitForRegisteredWebView(tabID: state.id, start: start) else { return nil }
        return ResolvedWebView(state: state, webView: webView)
    }

    private static func surfaceFailure<T>(
        tabIDString: String,
        start: ContinuousClock.Instant,
        appState: AppState
    ) -> Result<T, APIError> {
        guard browserState(tabIDString: tabIDString, appState: appState) != nil else {
            return .failure(.browserTabNotFound(tabIDString))
        }
        let waited = (ContinuousClock.now - start).secondsValue
        return .failure(.browserTabSurfaceNotReady(tabID: tabIDString, waitedSeconds: waited))
    }

    private static func runScript(
        tabIDString: String,
        appState: AppState,
        body: String
    ) async -> Result<String, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        let start = ContinuousClock.now
        guard let resolved = await resolve(tabIDString: tabIDString, appState: appState, start: start) else {
            return surfaceFailure(tabIDString: tabIDString, start: start, appState: appState)
        }
        let wrapped = "const __muxyResult = await (async () => { \(body) })(); return JSON.stringify(__muxyResult ?? null);"
        do {
            let result = try await resolved.webView.callAsyncJavaScript(
                wrapped,
                arguments: [:],
                contentWorld: .page
            )
            return .success(stringify(result))
        } catch {
            return .failure(.underlying(error.localizedDescription))
        }
    }

    private static func pollCondition(
        tabIDString: String,
        timeoutMs: Int,
        appState: AppState,
        expression: String
    ) async -> Result<Bool, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        let start = ContinuousClock.now
        guard browserState(tabIDString: tabIDString, appState: appState) != nil else {
            return .failure(.browserTabNotFound(tabIDString))
        }
        let bounded = max(0, min(timeoutMs, maxWaitMilliseconds))
        let deadline = start + .milliseconds(bounded)
        let body = "try { return Boolean(\(expression)); } catch (e) { return false; }"
        repeat {
            if let webView = BrowserWebViewRegistry.shared.webView(for: stateID(tabIDString, appState)),
               let value = await evaluateOnce(webView: webView, body: body),
               (value as? Bool) == true
            {
                return .success(true)
            }
            do { try await Task.sleep(for: .milliseconds(100)) } catch { break }
        } while ContinuousClock.now < deadline
        return .success(false)
    }

    private static func stateID(_ tabIDString: String, _ appState: AppState) -> UUID {
        browserState(tabIDString: tabIDString, appState: appState)?.id ?? UUID()
    }

    private static func evaluateOnce(webView: WKWebView, body: String) async -> Any? {
        do {
            return try await webView.callAsyncJavaScript(body, arguments: [:], contentWorld: .page)
        } catch {
            return nil
        }
    }

    private static func runAsyncResult<T>(
        tabIDString: String,
        appState: AppState,
        body: String,
        transform: @escaping (Any?) -> T
    ) async -> Result<T, APIError> {
        guard BrowserPreferences.isEnabled else { return .failure(.browserDisabled) }
        let start = ContinuousClock.now
        guard let resolved = await resolve(tabIDString: tabIDString, appState: appState, start: start) else {
            return surfaceFailure(tabIDString: tabIDString, start: start, appState: appState)
        }
        do {
            let result = try await resolved.webView.callAsyncJavaScript(
                body,
                arguments: [:],
                contentWorld: .page
            )
            return .success(transform(result))
        } catch {
            return .failure(.underlying(error.localizedDescription))
        }
    }

    private static func boolScript(
        tabIDString: String,
        appState: AppState,
        body: String
    ) async -> Result<Bool, APIError> {
        await runAsyncResult(tabIDString: tabIDString, appState: appState, body: body) { ($0 as? Bool) ?? false }
    }

    private static func optionalStringScript(
        tabIDString: String,
        appState: AppState,
        body: String
    ) async -> Result<String?, APIError> {
        await runAsyncResult(tabIDString: tabIDString, appState: appState, body: body) { $0 as? String }
    }

    private static func stringify(_ value: Any?) -> String {
        guard let value, !(value is NSNull) else { return "null" }
        if let string = value as? String {
            return string
        }
        if let data = try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]),
           let json = String(data: data, encoding: .utf8)
        {
            return json
        }
        return String(describing: value)
    }

    private static func waitConditionExpression(_ condition: WaitCondition) -> String {
        if let function = condition.function, !function.isEmpty {
            return "(\(function))"
        }
        if let selector = condition.selector, !selector.isEmpty {
            return "document.querySelector(\(jsString(selector)))"
        }
        if let text = condition.text, !text.isEmpty {
            return "document.body && document.body.innerText.includes(\(jsString(text)))"
        }
        if let urlContains = condition.urlContains, !urlContains.isEmpty {
            return "location.href.includes(\(jsString(urlContains)))"
        }
        return "true"
    }

    private static func isExpression(property: String) -> String {
        switch property {
        case "checked": "el.checked"
        case "enabled": "!el.disabled"
        case "disabled": "el.disabled"
        case "hidden": "el.offsetParent === null"
        default: "el.offsetParent !== null"
        }
    }

    private static func findMatcher(kind: String, value: String) -> String {
        let needle = jsString(value)
        switch kind {
        case "role": return "el.getAttribute('role') === \(needle)"
        case "testid": return "el.getAttribute('data-testid') === \(needle)"
        case "label": return "(el.getAttribute('aria-label') || '') === \(needle)"
        case "placeholder": return "el.getAttribute('placeholder') === \(needle)"
        default: return "(el.innerText || '').trim().includes(\(needle))"
        }
    }
}

func jsString(_ value: String) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: [value], options: [.fragmentsAllowed]),
          let json = String(data: data, encoding: .utf8)
    else {
        return "\"\""
    }
    return String(json.dropFirst().dropLast())
        .replacingOccurrences(of: "\u{2028}", with: "\\u2028")
        .replacingOccurrences(of: "\u{2029}", with: "\\u2029")
}

@MainActor
final class CookieFetchCoordinator<Value: Sendable> {
    private var states: [ObjectIdentifier: State] = [:]
    private let maximumActiveRequests = 2

    func value(
        for owner: AnyObject,
        timeout: Duration,
        operation: @escaping @MainActor () async -> Value
    ) async -> Value? {
        removeReleasedOwners()
        let identifier = ObjectIdentifier(owner)
        let state = states[identifier] ?? State(owner: owner)
        states[identifier] = state
        if let request = state.current {
            return await request.value()
        }
        guard state.active.count < maximumActiveRequests else {
            browserAutomationLogger.warning("Cookie request retry limit reached before WebKit returned data")
            return nil
        }
        let request = Request()
        state.current = request
        state.active[request.id] = request
        start(operation, for: identifier, request: request)
        startExpiry(for: identifier, request: request, after: timeout)
        return await request.value()
    }

    var requestCount: Int {
        states.values.reduce(0) { $0 + $1.active.count }
    }

    var waiterCount: Int {
        states.values.reduce(0) { total, state in
            total + state.active.values.reduce(0) { $0 + $1.waiterCount }
        }
    }

    private func start(
        _ operation: @escaping @MainActor () async -> Value,
        for identifier: ObjectIdentifier,
        request: Request
    ) {
        Task { [weak self, weak request] in
            let value = await operation()
            self?.complete(value, for: identifier, request: request)
        }
    }

    private func startExpiry(for identifier: ObjectIdentifier, request: Request, after timeout: Duration) {
        Task { [weak self, weak request] in
            try? await Task.sleep(for: timeout)
            self?.expire(for: identifier, request: request)
        }
    }

    private func complete(_ value: Value, for identifier: ObjectIdentifier, request: Request?) {
        guard let request, let state = states[identifier], state.active.removeValue(forKey: request.id) != nil else {
            return
        }
        if state.current === request {
            state.current = nil
            request.complete(with: value)
        }
        removeStateIfIdle(state, for: identifier)
    }

    private func expire(for identifier: ObjectIdentifier, request: Request?) {
        guard let request, let state = states[identifier], state.current === request else { return }
        state.current = nil
        request.complete(with: nil)
        browserAutomationLogger.warning("Cookie request expired before WebKit returned data")
    }

    private func removeStateIfIdle(_ state: State, for identifier: ObjectIdentifier) {
        guard state.active.isEmpty else { return }
        states[identifier] = nil
    }

    private func removeReleasedOwners() {
        states = states.filter { $0.value.owner != nil || $0.value.active.isEmpty == false }
    }

    @MainActor
    private final class State {
        weak var owner: AnyObject?
        var current: Request?
        var active: [UUID: Request] = [:]

        init(owner: AnyObject) {
            self.owner = owner
        }
    }

    @MainActor
    private final class Request {
        let id = UUID()
        private var result: Value?
        private var isComplete = false
        private var continuations: [UUID: CheckedContinuation<Value?, Never>] = [:]

        var waiterCount: Int {
            continuations.count
        }

        func value() async -> Value? {
            guard !isComplete else { return result }
            let identifier = UUID()
            return await withCheckedContinuation { continuation in
                continuations[identifier] = continuation
            }
        }

        func complete(with result: Value?) {
            guard !isComplete else { return }
            isComplete = true
            self.result = result
            let pending = continuations.values
            continuations.removeAll()
            for continuation in pending {
                continuation.resume(returning: result)
            }
        }
    }
}

@MainActor
private final class PDFResultBox {
    private var continuation: CheckedContinuation<Data?, Never>?
    private var resumed = false

    func attach(_ continuation: CheckedContinuation<Data?, Never>) {
        self.continuation = continuation
    }

    func resume(with data: Data?) {
        guard !resumed else { return }
        resumed = true
        continuation?.resume(returning: data)
        continuation = nil
    }
}

private extension BrowserCookieInfo {
    init(cookie: HTTPCookie) {
        name = cookie.name
        value = cookie.value
        domain = cookie.domain
        path = cookie.path
        secure = cookie.isSecure
        httpOnly = cookie.isHTTPOnly
        expires = cookie.expiresDate?.timeIntervalSince1970
    }

    func makeHTTPCookie() -> HTTPCookie? {
        var properties: [HTTPCookiePropertyKey: Any] = [
            .name: name,
            .value: value,
            .domain: domain,
            .path: path.isEmpty ? "/" : path,
        ]
        if secure {
            properties[.secure] = "TRUE"
        }
        if let expires {
            properties[.expires] = Date(timeIntervalSince1970: expires)
        }
        return HTTPCookie(properties: properties)
    }
}

private extension HTTPCookie {
    func domainMatches(host: String) -> Bool {
        let normalized = domain.hasPrefix(".") ? String(domain.dropFirst()) : domain
        return host == normalized || host.hasSuffix(".\(normalized)")
    }
}
