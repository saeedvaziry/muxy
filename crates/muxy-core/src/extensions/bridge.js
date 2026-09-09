(() => {
    const background = __MUXY_BACKGROUND__;
    const extensionID = __MUXY_EXTENSION_ID__;
    const tabInstanceID = __MUXY_INSTANCE_ID__;
    let currentData = __MUXY_DATA__;
    let currentTheme = Object.freeze(__MUXY_THEME__);
    let currentFocus = __MUXY_FOCUSED__;
    const dataListeners = new Set();
    const themeListeners = new Set();
    const focusListeners = new Set();
    const writeThemeToDocument = (theme) => {
        if (background || typeof document === 'undefined' || !document.documentElement) return;
        for (const [key, value] of Object.entries(theme)) {
            const cssName = key.replace(/[A-Z]/g, (match) => '-' + match.toLowerCase());
            document.documentElement.style.setProperty('--muxy-' + cssName, value);
        }
        document.documentElement.style.colorScheme = theme.colorScheme || 'light';
    };
    globalThis.__muxyApplyData = (data) => {
        currentData = data == null ? null : data;
        for (const listener of dataListeners) {
            try { listener(currentData); } catch (_) {}
        }
    };
    globalThis.__muxyApplyTheme = (theme) => {
        if (!theme || typeof theme !== 'object') return;
        currentTheme = Object.freeze({ ...theme });
        writeThemeToDocument(currentTheme);
        for (const listener of themeListeners) {
            try { listener(currentTheme); } catch (_) {}
        }
    };
    globalThis.__muxyApplyFocus = (focused) => {
        const next = !!focused;
        if (next === currentFocus) return;
        currentFocus = next;
        for (const listener of focusListeners) {
            try { listener(currentFocus); } catch (_) {}
        }
    };
    if (!background && typeof document !== 'undefined') {
        if (document.documentElement) writeThemeToDocument(currentTheme);
        else document.addEventListener('DOMContentLoaded', () => writeThemeToDocument(currentTheme), { once: true });
    }
    const unwrapReply = (reply) => {
        if (reply && reply.ok) return reply.value;
        throw new Error((reply && reply.error) || 'extension api error');
    };
    const dispatch = (verb, args) => {
        const reply = __muxyDispatch(verb, args || {});
        return reply && typeof reply.then === 'function' ? reply.then(unwrapReply) : unwrapReply(reply);
    };
    const mapResult = (value, fn) => value && typeof value.then === 'function' ? value.then(fn) : fn(value);
    const parseJSON = (value) => { try { return JSON.parse(value); } catch (_) { return value; } };
    const normalizeModalItems = (raw) => (Array.isArray(raw) ? raw : (raw && raw.items) || [])
        .map((item) => item && item.id != null && item.title != null ? {
            id: String(item.id),
            title: String(item.title),
            subtitle: item.subtitle == null ? null : String(item.subtitle),
        } : null)
        .filter(Boolean);
    const modalLabels = (options) => {
        const labels = {};
        if (options.placeholder != null) labels.placeholder = String(options.placeholder);
        if (options.emptyLabel != null) labels.emptyLabel = String(options.emptyLabel);
        if (options.noMatchLabel != null) labels.noMatchLabel = String(options.noMatchLabel);
        if ('searchToolbar' in options) labels.searchToolbar = !!options.searchToolbar;
        if (typeof options.onQuery === 'function' || typeof options.onQueryChange === 'function') labels.dynamic = true;
        return labels;
    };
    const buildExecPayload = (argvOrOptions, maybeOptions) => {
        let payload;
        if (Array.isArray(argvOrOptions)) {
            const options = maybeOptions || {};
            payload = { argv: argvOrOptions.map(String) };
            if (options.cwd != null) payload.cwd = String(options.cwd);
            if (options.env) payload.env = options.env;
            if (options.stdin != null) payload.stdin = String(options.stdin);
            if (options.timeoutMs != null) payload.timeoutMs = Number(options.timeoutMs);
        } else {
            const options = argvOrOptions || {};
            payload = {};
            if (options.shell != null) payload.shell = String(options.shell);
            if (options.argv) payload.argv = options.argv.map(String);
            if (options.cwd != null) payload.cwd = String(options.cwd);
            if (options.env) payload.env = options.env;
            if (options.stdin != null) payload.stdin = String(options.stdin);
            if (options.timeoutMs != null) payload.timeoutMs = Number(options.timeoutMs);
        }
        return payload;
    };
    const modalResultHandlers = {};
    const modalWebviewResultHandlers = {};
    const modalQueryHandlers = {};
    let activeModalQueryID = null;
    const webviewModalPrefix = 'webview:';
    globalThis.__muxiDeliverModalResult = (requestID, item) => {
        if (String(requestID).indexOf(webviewModalPrefix) === 0) {
            const handler = modalWebviewResultHandlers[requestID];
            delete modalWebviewResultHandlers[requestID];
            if (typeof handler === 'function') {
                try { handler(item == null ? null : item); } catch (error) { console.error(error); }
            }
            return;
        }
        const handler = modalResultHandlers[requestID];
        delete modalResultHandlers[requestID];
        delete modalQueryHandlers[requestID];
        if (typeof handler === 'function') {
            try { handler(item == null ? null : item); } catch (error) { console.error(error); }
        }
    };
    globalThis.__muxyDeliverModalQuery = (requestID, queryID, query, options) => {
        const handler = modalQueryHandlers[requestID];
        const emit = (batch) => dispatch('modal.feed', { items: normalizeModalItems(batch), queryID });
        const finish = () => dispatch('modal.finish', { queryID });
        if (typeof handler !== 'function') { finish(); return; }
        let produced;
        const previous = activeModalQueryID;
        activeModalQueryID = queryID;
        try {
            produced = handler(query, emit, options || {});
        } catch (error) {
            console.error(error);
            finish();
            return;
        } finally {
            activeModalQueryID = previous;
        }
        const done = (value) => {
            if (value != null) emit(value);
            finish();
        };
        if (produced && typeof produced.then === 'function') {
            produced.then(done, (error) => { console.error(error); finish(); });
        } else {
            done(produced);
        }
    };
    const muxy = {
        extensionID,
        notifications: { notify: (options) => dispatch('notifications.notify', options || {}) },
        exec(argvOrOptions, maybeOptions) { return dispatch('exec', buildExecPayload(argvOrOptions, maybeOptions)); },
        dialog: {
            confirm(options) {
                const source = options || {};
                const payload = {};
                for (const key of ['title', 'message', 'default', 'cancel', 'style']) {
                    if (source[key] != null) payload[key] = String(source[key]);
                }
                if (Array.isArray(source.buttons)) payload.buttons = source.buttons.map(String);
                return dispatch('dialog.confirm', payload);
            },
            alert(options) {
                const source = options || {};
                const payload = {};
                for (const key of ['title', 'message', 'style']) {
                    if (source[key] != null) payload[key] = String(source[key]);
                }
                return dispatch('dialog.alert', payload);
            },
            prompt(options) {
                const source = options || {};
                const payload = {};
                for (const key of ['title', 'message', 'default', 'placeholder', 'confirm', 'cancel']) {
                    if (source[key] != null) payload[key] = String(source[key]);
                }
                return dispatch('dialog.prompt', payload);
            },
            pickFolder(options) {
                const source = options || {};
                const payload = {};
                for (const key of ['title', 'message', 'default']) {
                    if (source[key] != null) payload[key] = String(source[key]);
                }
                return dispatch('dialog.pickFolder', payload);
            },
        },
        storage: {
            get: (key) => dispatch('storage.get', { key: String(key) }),
            set: (key, value) => dispatch('storage.set', { key: String(key), value: value === undefined ? null : value }),
            delete: (key) => dispatch('storage.delete', { key: String(key) }),
            keys: () => dispatch('storage.keys', {}),
        },
        shortcuts: {
            register(options) {
                const source = options || {};
                return dispatch('shortcuts.register', {
                    id: String(source.id == null ? '' : source.id),
                    combo: String(source.combo == null ? '' : source.combo),
                });
            },
            unregister: (id) => dispatch('shortcuts.unregister', { id: String(id == null ? '' : id) }),
            list: () => dispatch('shortcuts.list', {}),
        },
        modal: {
            open(options) {
                const source = options || {};
                const opened = dispatch('modal.open', modalLabels(source));
                const requestID = opened && opened.requestID;
                if (requestID != null) {
                    if (typeof source.onSelect === 'function') modalResultHandlers[requestID] = source.onSelect;
                    if (typeof source.onQuery === 'function') modalQueryHandlers[requestID] = source.onQuery;
                    else if (typeof source.onQueryChange === 'function') {
                        modalQueryHandlers[requestID] = (query, emit, queryOptions) => source.onQueryChange(query, queryOptions || {});
                    }
                }
                const emit = (batch) => dispatch('modal.feed', { items: normalizeModalItems(batch) });
                if (typeof source.items === 'function') {
                    const produced = source.items(emit);
                    if (produced != null) emit(produced);
                } else {
                    emit(source.items);
                }
                dispatch('modal.finish', {});
                return requestID;
            },
            feed(items) {
                const payload = { items: normalizeModalItems(items) };
                if (activeModalQueryID != null) payload.queryID = activeModalQueryID;
                return dispatch('modal.feed', payload);
            },
            finish() {
                const payload = {};
                if (activeModalQueryID != null) payload.queryID = activeModalQueryID;
                return dispatch('modal.finish', payload);
            },
            openWebview(options) {
                const source = options || {};
                const payload = { entry: String(source.entry == null ? '' : source.entry) };
                if (source.width != null) payload.width = Number(source.width);
                if (source.height != null) payload.height = Number(source.height);
                if (source.dismissOnOutsideClick != null) payload.dismissOnOutsideClick = !!source.dismissOnOutsideClick;
                if (source.data !== undefined) payload.data = source.data == null ? null : source.data;
                const opened = dispatch('modal.openWebview', payload);
                const requestID = opened && opened.requestID;
                return new Promise((resolve) => {
                    if (requestID == null) { resolve(null); return; }
                    modalWebviewResultHandlers[requestID] = resolve;
                });
            },
            closeWebview: () => dispatch('modal.closeWebview', {}),
        },
        topbar: {
            set(options) {
                const source = options || {};
                const payload = { id: String(source.id == null ? '' : source.id) };
                if (source.icon != null) payload.icon = source.icon;
                if ('visible' in source) payload.visible = !!source.visible;
                return dispatch('topbar.set', payload);
            },
            show: (id) => dispatch('topbar.set', { id: String(id == null ? '' : id), visible: true }),
            hide: (id) => dispatch('topbar.set', { id: String(id == null ? '' : id), visible: false }),
        },
        statusbar: {
            set(options) {
                const source = options || {};
                const payload = { id: String(source.id == null ? '' : source.id) };
                if (source.icon != null) payload.icon = source.icon;
                if ('text' in source) payload.text = source.text == null ? null : String(source.text);
                if ('visible' in source) payload.visible = !!source.visible;
                return dispatch('statusbar.set', payload);
            },
            show: (id) => dispatch('statusbar.set', { id: String(id == null ? '' : id), visible: true }),
            hide: (id) => dispatch('statusbar.set', { id: String(id == null ? '' : id), visible: false }),
        },
    };
    if (background) {
        muxy.tabs = { open: (request) => dispatch('tabs.open', request || {}) };
        const handlers = {};
        globalThis.__muxyEventHandlers = handlers;
        globalThis.__muxyDispatchEvent = (name, payload) => {
            for (const handler of (handlers[String(name)] || []).slice()) {
                try { handler(payload); } catch (error) { console.error(error); }
            }
        };
        muxy.events = {
            subscribe(name, handler) {
                if (typeof handler !== 'function') return () => {};
                const key = String(name);
                if (!handlers[key]) {
                    handlers[key] = [];
                    if (!key.startsWith('extension.')) __muxySubscribe(key);
                }
                handlers[key].push(handler);
                return () => muxy.events.unsubscribe(key, handler);
            },
            unsubscribe(name, handler) {
                const list = handlers[String(name)];
                if (!list) return;
                const index = list.indexOf(handler);
                if (index >= 0) list.splice(index, 1);
            },
            emit(name, payload) {
                const key = String(name);
                if (!key.startsWith('extension.') || key.length <= 'extension.'.length) {
                    throw new Error('extension events must start with extension.');
                }
                return dispatch('events.emit', { event: key, payload: payload === undefined ? null : payload });
            },
        };
        const remoteHandlers = {};
        muxy.remote = {
            handle: (action, handler) => { remoteHandlers[String(action)] = handler; },
            unhandle: (action) => { delete remoteHandlers[String(action)]; },
        };
        globalThis.__muxyDispatchInvoke = (callID, action, argument) => {
            const handler = remoteHandlers[String(action)];
            if (typeof handler !== 'function') {
                __muxyInvokeReject(callID, "no handler registered for '" + action + "'");
                return;
            }
            let result;
            try { result = handler(argument); }
            catch (error) {
                __muxyInvokeReject(callID, String((error && error.message) || error));
                return;
            }
            Promise.resolve(result).then(
                (value) => {
                    let json;
                    try { json = JSON.stringify(value === undefined ? null : value); }
                    catch (_) { __muxyInvokeReject(callID, 'result is not serializable'); return; }
                    __muxyInvokeResolve(callID, json == null ? 'null' : json);
                },
                (error) => __muxyInvokeReject(callID, String((error && error.message) || error)),
            );
        };
    } else {
        Object.defineProperties(muxy, {
            tabInstanceID: { value: tabInstanceID, enumerable: true },
            data: { get: () => currentData, enumerable: true },
            theme: { get: () => currentTheme, enumerable: true },
            focused: { get: () => currentFocus, enumerable: true },
        });
        muxy.onDataChange = (callback) => {
            if (typeof callback !== 'function') return () => {};
            dataListeners.add(callback);
            return () => dataListeners.delete(callback);
        };
        muxy.onThemeChange = (callback) => {
            if (typeof callback !== 'function') return () => {};
            themeListeners.add(callback);
            return () => themeListeners.delete(callback);
        };
        muxy.onFocus = (callback) => {
            if (typeof callback !== 'function') return () => {};
            focusListeners.add(callback);
            return () => focusListeners.delete(callback);
        };
        let beforeCloseHandler = null;
        globalThis.__muxyResolveBeforeClose = (callID, prevent) => {
            Promise.resolve(dispatch('lifecycle.resolveBeforeClose', { callID: String(callID), prevent: !!prevent })).catch(() => {});
        };
        globalThis.__muxyBeforeClose = (callID, reason, instanceID) => {
            if (typeof beforeCloseHandler !== 'function') {
                globalThis.__muxyResolveBeforeClose(callID, false);
                return;
            }
            Promise.resolve(dispatch('lifecycle.ackBeforeClose', { callID: String(callID) })).catch(() => {});
            let outcome;
            try { outcome = beforeCloseHandler({ surface: String(reason), instanceID: String(instanceID) }); }
            catch (_) { globalThis.__muxyResolveBeforeClose(callID, false); return; }
            Promise.resolve(outcome).then(
                (value) => globalThis.__muxyResolveBeforeClose(callID, value === true || !!(value && value.prevent === true)),
                () => globalThis.__muxyResolveBeforeClose(callID, false),
            );
        };
        muxy.lifecycle = {
            onBeforeClose(handler) {
                beforeCloseHandler = typeof handler === 'function' ? handler : null;
                return () => { if (beforeCloseHandler === handler) beforeCloseHandler = null; };
            },
            close: () => dispatch('lifecycle.closeSelf', {}),
        };
        muxy.toast = (options) => dispatch('toast', options || {});
        muxy.tabs = {
            list: () => dispatch('tabs.list', {}),
            switchTo: (identifier) => dispatch('tabs.switch', { identifier: String(identifier) }),
            new: () => dispatch('tabs.new', {}),
            next: () => dispatch('tabs.next', {}),
            previous: () => dispatch('tabs.previous', {}),
            open: (request) => dispatch('tabs.open', request || {}),
            setTitle: (title) => dispatch('tabs.setTitle', { tabInstanceID, title: String(title == null ? '' : title) }),
            setIcon: (icon) => dispatch('tabs.setIcon', { tabInstanceID, icon: icon == null ? null : icon }),
        };
        muxy.browser = {
            open: (url, options) => dispatch('browser.open', { url: url == null ? null : String(url), split: Boolean((options || {}).split) }),
            navigate: (tabId, url) => dispatch('browser.navigate', { tabId: String(tabId), url: String(url) }),
            list: () => dispatch('browser.list', {}),
            read: (tabId) => dispatch('browser.read', { tabId: String(tabId) }),
            close: (tabId) => dispatch('browser.close', { tabId: String(tabId) }),
            eval: (tabId, script) => mapResult(dispatch('browser.eval', { tabId: String(tabId), script: String(script) }), parseJSON),
            click: (tabId, selector) => dispatch('browser.click', { tabId: String(tabId), selector: String(selector) }),
            type: (tabId, selector, text, options) => dispatch('browser.type', { tabId: String(tabId), selector: String(selector), text: String(text), submit: Boolean((options || {}).submit) }),
            waitFor: (tabId, selector, options) => dispatch('browser.waitFor', { tabId: String(tabId), selector: String(selector), timeoutMs: Number((options || {}).timeoutMs == null ? 5000 : options.timeoutMs) }),
            wait: (tabId, options) => {
                const source = options || {};
                return dispatch('browser.wait', {
                    tabId: String(tabId),
                    selector: source.selector == null ? null : String(source.selector),
                    text: source.text == null ? null : String(source.text),
                    urlContains: source.urlContains == null ? null : String(source.urlContains),
                    function: source.function == null ? null : String(source.function),
                    timeoutMs: Number(source.timeoutMs == null ? 5000 : source.timeoutMs),
                });
            },
            fill: (tabId, selector, text) => dispatch('browser.fill', { tabId: String(tabId), selector: String(selector), text: String(text) }),
            press: (tabId, key, selector) => dispatch('browser.press', { tabId: String(tabId), key: String(key), selector: selector == null ? null : String(selector) }),
            select: (tabId, selector, value) => dispatch('browser.select', { tabId: String(tabId), selector: String(selector), value: String(value) }),
            hover: (tabId, selector) => dispatch('browser.hover', { tabId: String(tabId), selector: String(selector) }),
            scrollIntoView: (tabId, selector) => dispatch('browser.scrollIntoView', { tabId: String(tabId), selector: String(selector) }),
            setChecked: (tabId, selector, checked) => dispatch('browser.setChecked', { tabId: String(tabId), selector: String(selector), checked: Boolean(checked) }),
            is: (tabId, property, selector) => dispatch('browser.is', { tabId: String(tabId), property: String(property), selector: String(selector) }),
            getValue: (tabId, selector) => dispatch('browser.getValue', { tabId: String(tabId), selector: String(selector) }),
            getCount: (tabId, selector) => dispatch('browser.getCount', { tabId: String(tabId), selector: String(selector) }),
            find: (tabId, kind, value) => mapResult(dispatch('browser.find', { tabId: String(tabId), kind: String(kind), value: String(value) }), parseJSON),
            snapshot: (tabId, selector) => mapResult(dispatch('browser.snapshot', { tabId: String(tabId), selector: selector == null ? null : String(selector) }), parseJSON),
            getText: (tabId, selector) => dispatch('browser.getText', { tabId: String(tabId), selector: String(selector) }),
            getHTML: (tabId, selector) => dispatch('browser.getHTML', { tabId: String(tabId), selector: selector == null ? null : String(selector) }),
            getAttribute: (tabId, selector, name) => dispatch('browser.getAttribute', { tabId: String(tabId), selector: String(selector), attribute: String(name) }),
            reload: (tabId) => dispatch('browser.reload', { tabId: String(tabId) }),
            back: (tabId) => dispatch('browser.back', { tabId: String(tabId) }),
            forward: (tabId) => dispatch('browser.forward', { tabId: String(tabId) }),
            waitForNavigation: (tabId, options) => dispatch('browser.waitForNavigation', { tabId: String(tabId), timeoutMs: Number((options || {}).timeoutMs == null ? 10000 : options.timeoutMs) }),
            screenshot: (tabId) => mapResult(dispatch('browser.screenshot', { tabId: String(tabId) }), (result) => (result || {}).png),
            storage: {
                get: (tabId, key, kind) => dispatch('browser.storage.get', { tabId: String(tabId), key: String(key), kind: kind || 'local' }),
                set: (tabId, key, value, kind) => dispatch('browser.storage.set', { tabId: String(tabId), key: String(key), value: String(value), kind: kind || 'local' }),
                clear: (tabId, kind) => dispatch('browser.storage.clear', { tabId: String(tabId), kind: kind || 'local' }),
            },
            cookies: {
                get: (tabId, url) => dispatch('browser.cookies.get', { tabId: String(tabId), url: url == null ? null : String(url) }),
                set: (tabId, cookie) => dispatch('browser.cookies.set', Object.assign({ tabId: String(tabId) }, cookie || {})),
                delete: (tabId, name, domain) => dispatch('browser.cookies.delete', { tabId: String(tabId), name: String(name), domain: domain == null ? null : String(domain) }),
                clear: (tabId) => dispatch('browser.cookies.clear', { tabId: String(tabId) }),
            },
        };
        muxy.panes = {
            list: () => dispatch('panes.list', {}),
            send: (paneID, text) => dispatch('panes.send', { paneID, text: String(text) }),
            sendKeys: (paneID, key) => dispatch('panes.sendKeys', { paneID, key: String(key) }),
            readScreen: (paneID, lines) => dispatch('panes.readScreen', { paneID, lines: lines == null ? 50 : Number(lines) }),
            close: (paneID) => dispatch('panes.close', { paneID }),
            rename: (paneID, title) => dispatch('panes.rename', { paneID, title: String(title) }),
        };
        muxy.projects = {
            list: () => dispatch('projects.list', {}),
            switchTo: (identifier) => dispatch('projects.switch', { identifier: String(identifier) }),
            delete: (identifier) => dispatch('projects.delete', { identifier: String(identifier) }),
            add: (path) => dispatch('projects.add', { path: String(path) }),
            create(path, options) {
                const source = options || {};
                const payload = { path: String(path), createIfMissing: Boolean(source.createIfMissing) };
                if (source.name != null) payload.name = String(source.name);
                if (source.workspace != null) payload.workspace = String(source.workspace);
                return dispatch('projects.create', payload);
            },
            attach: (identifier, workspace) => dispatch('projects.attach', { identifier: String(identifier), workspace: String(workspace) }),
            detach: (identifier) => dispatch('projects.detach', { identifier: String(identifier) }),
            rename: (identifier, name) => dispatch('projects.rename', { identifier: String(identifier), name: String(name) }),
            setColor: (identifier, color) => dispatch('projects.setColor', { identifier: String(identifier), color: color == null ? null : String(color) }),
            setIcon: (identifier, icon) => dispatch('projects.setIcon', { identifier: String(identifier), icon: icon == null ? null : String(icon) }),
            setLogo: (identifier, logo) => dispatch('projects.setLogo', { identifier: String(identifier), logo: logo == null ? null : String(logo) }),
            reorder: (identifiers) => dispatch('projects.reorder', { identifiers: (identifiers || []).map(String) }),
        };
        muxy.workspaces = {
            list: () => dispatch('workspaces.list', {}),
            create: (name) => dispatch('workspaces.create', { name: String(name) }),
            switchTo: (identifier) => dispatch('workspaces.switch', { identifier: String(identifier) }),
            rename: (identifier, name) => dispatch('workspaces.rename', { identifier: String(identifier), name: String(name) }),
            delete: (identifier) => dispatch('workspaces.delete', { identifier: String(identifier) }),
        };
        muxy.panels = {
            open: (panel, data) => dispatch('panel.open', { panel: String(panel), data: data == null ? null : data }),
            toggle: (panel, data) => dispatch('panel.toggle', { panel: String(panel), data: data == null ? null : data }),
            close: (panel) => dispatch('panel.close', { panel: String(panel) }),
        };
        muxy.popover = {
            close: () => dispatch('popover.close', {}),
            resize: (width, height) => dispatch('popover.resize', { width: Number(width), height: Number(height) }),
        };
        muxy.http = {
            fetch(url, options) {
                const source = options || {};
                const payload = { url: String(url) };
                if (source.method != null) payload.method = String(source.method);
                if (source.headers) payload.headers = source.headers;
                if (source.body != null) payload.body = String(source.body);
                if (source.timeoutMs != null) payload.timeoutMs = Number(source.timeoutMs);
                return dispatch('http.fetch', payload);
            },
        };
        muxy.worktrees = {
            list: (project) => dispatch('worktrees.list', { project: project == null ? null : String(project) }),
            switchTo: (identifier, project) => dispatch('worktrees.switch', { identifier: String(identifier), project: project == null ? null : String(project) }),
            refresh: (project) => dispatch('worktrees.refresh', { project: project == null ? null : String(project) }),
        };
        const fileProject = (options) => options && options.project != null ? String(options.project) : null;
        muxy.files = {
            list: (path, options) => dispatch('files.list', { project: fileProject(options), path: String(path == null ? '' : path) }),
            read: (path, options) => dispatch('files.read', { project: fileProject(options), path: String(path == null ? '' : path) }),
            stat: (path, options) => dispatch('files.stat', { project: fileProject(options), path: String(path == null ? '' : path) }),
            write: (path, contents, options) => dispatch('files.write', { project: fileProject(options), path: String(path == null ? '' : path), contents: String(contents == null ? '' : contents) }),
            mkdir: (path, options) => dispatch('files.mkdir', { project: fileProject(options), path: String(path == null ? '' : path) }),
            rename: (path, newName, options) => dispatch('files.rename', { project: fileProject(options), path: String(path == null ? '' : path), newName: String(newName == null ? '' : newName) }),
            move: (paths, into, options) => dispatch('files.move', { project: fileProject(options), paths: (paths || []).map(String), into: String(into == null ? '' : into) }),
            delete: (paths, options) => dispatch('files.delete', { project: fileProject(options), paths: (paths || []).map(String) }),
        };
        const handlers = {};
        globalThis.__muxyEventHandlers = handlers;
        globalThis.__muxyDispatchEvent = (name, payload) => {
            for (const handler of (handlers[String(name)] || []).slice()) {
                try { handler(payload); } catch (error) { console.error(error); }
            }
        };
        globalThis.__muxyEventDispatch = globalThis.__muxyDispatchEvent;
        muxy.events = {
            subscribe(name, handler) {
                if (typeof name !== 'string' || typeof handler !== 'function') return () => {};
                const key = String(name);
                if (!handlers[key]) {
                    handlers[key] = [];
                    Promise.resolve(dispatch('events.subscribe', { event: key })).catch((error) => {
                        delete handlers[key];
                        try { console.error('muxy.events.subscribe failed:', error.message || error); } catch (_) {}
                    });
                }
                handlers[key].push(handler);
                return () => muxy.events.unsubscribe(key, handler);
            },
            unsubscribe(name, handler) {
                const key = String(name);
                const list = handlers[key];
                if (!list) return;
                const index = list.indexOf(handler);
                if (index >= 0) list.splice(index, 1);
                if (list.length === 0) {
                    delete handlers[key];
                    Promise.resolve(dispatch('events.unsubscribe', { event: key })).catch(() => {});
                }
            },
            emit(name, payload) {
                const key = String(name);
                if (!key.startsWith('extension.') || key.length <= 'extension.'.length) {
                    return Promise.reject(new Error('extension events must start with extension.'));
                }
                return dispatch('events.emit', { event: key, payload: payload === undefined ? null : payload });
            },
        };
        muxy.modal = {
            async open(options) {
                const source = options || {};
                const opened = await dispatch('modal.open', modalLabels(source));
                const requestID = opened && opened.requestID;
                if (requestID != null) {
                    if (typeof source.onQuery === 'function') modalQueryHandlers[requestID] = source.onQuery;
                    else if (typeof source.onQueryChange === 'function') {
                        modalQueryHandlers[requestID] = (query, emit, queryOptions) => source.onQueryChange(query, queryOptions || {});
                    }
                }
                const emit = (batch) => dispatch('modal.feed', { items: normalizeModalItems(batch) });
                try {
                    if (typeof source.items === 'function') {
                        const produced = await source.items(emit);
                        if (produced != null) await emit(produced);
                    } else {
                        await emit(source.items);
                    }
                    await dispatch('modal.finish', {});
                    const choice = await dispatch('modal.await', { requestID });
                    if (typeof source.onSelect === 'function') source.onSelect(choice);
                    return choice;
                } finally {
                    if (requestID != null) delete modalQueryHandlers[requestID];
                }
            },
            feed(items) {
                const payload = { items: normalizeModalItems(items) };
                if (activeModalQueryID != null) payload.queryID = activeModalQueryID;
                return dispatch('modal.feed', payload);
            },
            finish() {
                const payload = {};
                if (activeModalQueryID != null) payload.queryID = activeModalQueryID;
                return dispatch('modal.finish', payload);
            },
            async openWebview(options) {
                const source = options || {};
                const payload = { entry: String(source.entry == null ? '' : source.entry) };
                if (source.width != null) payload.width = Number(source.width);
                if (source.height != null) payload.height = Number(source.height);
                if (source.dismissOnOutsideClick != null) payload.dismissOnOutsideClick = !!source.dismissOnOutsideClick;
                if (source.data !== undefined) payload.data = source.data == null ? null : source.data;
                const opened = await dispatch('modal.openWebview', payload);
                const requestID = opened && opened.requestID;
                return requestID == null ? null : dispatch('modal.awaitWebview', { requestID });
            },
            submitWebview: (result) => dispatch('modal.submitWebview', { requestID: tabInstanceID, result: result === undefined ? null : result }),
            closeWebview: () => dispatch('modal.closeWebview', {}),
        };
        globalThis.__muxyDeliverModalQuery = async (requestID, queryID, query, options) => {
            const handler = modalQueryHandlers[String(requestID)];
            const emit = (batch) => dispatch('modal.feed', { items: normalizeModalItems(batch), queryID });
            try {
                if (typeof handler === 'function') {
                    const previous = activeModalQueryID;
                    activeModalQueryID = queryID;
                    let produced;
                    try { produced = await handler(query, emit, options || {}); }
                    finally { activeModalQueryID = previous; }
                    if (produced != null) await emit(produced);
                }
            } catch (error) {
                console.error(error);
            }
            await dispatch('modal.finish', { queryID });
        };
    }
    const gitProject = (options) => options && options.project != null ? String(options.project) : null;
    muxy.git = {
        status: (options) => dispatch('git.status', { project: gitProject(options), local: Boolean((options || {}).local), fresh: Boolean((options || {}).fresh) }),
        diff: (options) => dispatch('git.diff', { project: gitProject(options), filePath: String((options || {}).filePath || ''), raw: Boolean((options || {}).raw), staged: (options || {}).staged == null ? null : Boolean(options.staged), lineLimit: (options || {}).lineLimit == null ? null : Number(options.lineLimit), fresh: Boolean((options || {}).fresh) }),
        repoInfo: (options) => dispatch('git.repoInfo', { project: gitProject(options) }),
        log: (options) => dispatch('git.log', { project: gitProject(options), maxCount: (options || {}).maxCount == null ? null : Number(options.maxCount), skip: (options || {}).skip == null ? null : Number(options.skip), fresh: Boolean((options || {}).fresh) }),
        branches: (options) => dispatch('git.branches', { project: gitProject(options) }),
        remoteBranches: (options) => dispatch('git.remoteBranches', { project: gitProject(options) }),
        currentBranch: (options) => dispatch('git.currentBranch', { project: gitProject(options) }),
        aheadBehind: (options) => dispatch('git.aheadBehind', { project: gitProject(options), fresh: Boolean((options || {}).fresh) }),
        init: (options) => dispatch('git.init', { project: gitProject(options) }),
        worktrees: (options) => dispatch('git.worktrees', { project: gitProject(options) }),
        stage: (options) => dispatch('git.stage', { project: gitProject(options), paths: ((options || {}).paths || []).map(String) }),
        unstage: (options) => dispatch('git.unstage', { project: gitProject(options), paths: ((options || {}).paths || []).map(String) }),
        discard: (options) => dispatch('git.discard', { project: gitProject(options), paths: ((options || {}).paths || []).map(String), untrackedPaths: ((options || {}).untrackedPaths || []).map(String) }),
        commit: (options) => dispatch('git.commit', { project: gitProject(options), message: String((options || {}).message || ''), stageAll: Boolean((options || {}).stageAll) }),
        push: (options) => dispatch('git.push', { project: gitProject(options), setUpstream: Boolean((options || {}).setUpstream) }),
        pull: (options) => dispatch('git.pull', { project: gitProject(options) }),
        checkout: (options) => dispatch('git.checkout', { project: gitProject(options), hash: String((options || {}).hash || '') }),
        cherryPick: (options) => dispatch('git.cherryPick', { project: gitProject(options), hash: String((options || {}).hash || '') }),
        revert: (options) => dispatch('git.revert', { project: gitProject(options), hash: String((options || {}).hash || '') }),
        branch: {
            create: (options) => dispatch('git.branch.create', { project: gitProject(options), name: String((options || {}).name || '') }),
            switchTo: (options) => dispatch('git.branch.switch', { project: gitProject(options), branch: String((options || {}).branch || '') }),
            delete: (options) => dispatch('git.branch.delete', { project: gitProject(options), name: String((options || {}).name || ''), force: Boolean((options || {}).force) }),
            deleteRemote: (options) => dispatch('git.branch.deleteRemote', { project: gitProject(options), branch: String((options || {}).branch || '') }),
        },
        tag: {
            create: (options) => dispatch('git.tag.create', { project: gitProject(options), name: String((options || {}).name || ''), hash: String((options || {}).hash || '') }),
        },
        pr: {
            info: (options) => dispatch('git.pr.info', { project: gitProject(options), fresh: Boolean((options || {}).fresh) }),
            number: (options) => dispatch('git.pr.number', { project: gitProject(options), fresh: Boolean((options || {}).fresh) }),
            diff: (options) => dispatch('git.pr.diff', { project: gitProject(options), number: Number((options || {}).number), lineLimit: (options || {}).lineLimit == null ? null : Number(options.lineLimit), fresh: Boolean((options || {}).fresh) }),
            checkout: (options) => dispatch('git.pr.checkout', { project: gitProject(options), number: Number((options || {}).number) }),
            checkoutWorktree: (options) => dispatch('git.pr.checkoutWorktree', { project: gitProject(options), path: String((options || {}).path || ''), number: Number((options || {}).number) }),
            list: (options) => dispatch('git.pr.list', { project: gitProject(options), filter: (options || {}).filter == null ? null : String(options.filter), limit: (options || {}).limit == null ? null : Number(options.limit), checks: (options || {}).checks == null ? null : Boolean(options.checks) }),
            create: (options) => dispatch('git.pr.create', { project: gitProject(options), title: String((options || {}).title || ''), body: String((options || {}).body || ''), baseBranch: (options || {}).baseBranch == null ? null : String(options.baseBranch), draft: Boolean((options || {}).draft) }),
            merge: (options) => dispatch('git.pr.merge', { project: gitProject(options), number: Number((options || {}).number), method: (options || {}).method == null ? null : String(options.method), deleteBranch: (options || {}).deleteBranch == null ? true : Boolean(options.deleteBranch) }),
            close: (options) => dispatch('git.pr.close', { project: gitProject(options), number: Number((options || {}).number) }),
        },
        worktree: {
            add: (options) => dispatch('git.worktree.add', { project: gitProject(options), path: String((options || {}).path || ''), branch: String((options || {}).branch || ''), createBranch: Boolean((options || {}).createBranch), baseBranch: (options || {}).baseBranch == null ? null : String(options.baseBranch) }),
            remove: (options) => dispatch('git.worktree.remove', { project: gitProject(options), path: String((options || {}).path || ''), force: Boolean((options || {}).force), timeoutMs: (options || {}).timeoutMs == null ? null : Number(options.timeoutMs) }),
            switchTo: (options) => dispatch('git.worktree.switch', { project: gitProject(options), identifier: String((options || {}).identifier || '') }),
        },
    };
    muxy.gh = { user: () => dispatch('gh.user', {}) };
    muxy.agents = { list: () => dispatch('agents.list', {}) };
    for (const value of Object.values(muxy)) {
        if (value && typeof value === 'object') Object.freeze(value);
    }
    Object.freeze(muxy.git.pr);
    Object.freeze(muxy.git.branch);
    Object.freeze(muxy.git.worktree);
    Object.freeze(muxy);
    globalThis.muxy = muxy;
    const formatForConsole = (value) => {
        if (value === null) return 'null';
        if (value === undefined) return 'undefined';
        if (typeof value === 'string') return value;
        if (value instanceof Error) return value.stack || value.message;
        try { return JSON.stringify(value); } catch (_) { return String(value); }
    };
    const consoleSend = (level, args) => __muxyConsole(level, Array.prototype.map.call(args, formatForConsole).join(' '));
    if (background) {
        globalThis.console = {
            log() { consoleSend('log', arguments); },
            warn() { consoleSend('warn', arguments); },
            error() { consoleSend('err', arguments); },
        };
    } else {
        const originalConsole = globalThis.console || {};
        const wrapConsole = (original, level) => function () {
            try { consoleSend(level, arguments); } catch (_) {}
            if (typeof original === 'function') {
                try { original.apply(originalConsole, arguments); } catch (_) {}
            }
        };
        originalConsole.log = wrapConsole(originalConsole.log, 'log');
        originalConsole.warn = wrapConsole(originalConsole.warn, 'warn');
        originalConsole.error = wrapConsole(originalConsole.error, 'err');
        globalThis.console = originalConsole;
        if (typeof globalThis.addEventListener === 'function') {
            globalThis.addEventListener('error', (event) => {
                try { __muxyConsole('err', event.error ? formatForConsole(event.error) : String(event.message || 'unknown error')); } catch (_) {}
            });
            globalThis.addEventListener('unhandledrejection', (event) => {
                try { __muxyConsole('err', event.reason === undefined ? 'unhandledrejection' : formatForConsole(event.reason)); } catch (_) {}
            });
        }
    }
})();
