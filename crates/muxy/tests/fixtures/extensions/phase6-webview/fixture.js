(() => {
  const byID = (id) => document.getElementById(id);
  const counters = { focus: 0, data: 0, theme: 0 };

  const renderFocus = (focused) => {
    const badge = byID("focusBadge");
    badge.textContent = focused ? "Focused" : "Not focused";
    badge.classList.toggle("focused", focused);
  };

  const renderData = (data) => {
    byID("data").textContent = JSON.stringify(data, null, 2);
  };

  const renderTheme = (theme) => {
    byID("theme").textContent = `${theme.colorScheme} · ${theme.accent} · ${theme.topbarHeight}`;
  };

  const renderCount = (kind, label) => {
    byID(`${kind}Events`).textContent = `${counters[kind]} ${label} observed`;
  };

  byID("extensionID").textContent = window.muxy.extensionID;
  byID("instanceID").textContent = window.muxy.tabInstanceID;
  renderData(window.muxy.data);
  renderTheme(window.muxy.theme);
  renderFocus(window.muxy.focused);

  window.muxy.onFocus((focused) => {
    counters.focus += 1;
    renderCount("focus", counters.focus === 1 ? "focus change" : "focus changes");
    renderFocus(focused);
  });

  window.muxy.onDataChange((data) => {
    counters.data += 1;
    renderCount("data", counters.data === 1 ? "data change" : "data changes");
    renderData(data);
  });

  window.muxy.onThemeChange((theme) => {
    counters.theme += 1;
    renderCount("theme", counters.theme === 1 ? "theme change" : "theme changes");
    renderTheme(theme);
  });

  window.muxy.lifecycle.onBeforeClose(({ surface, instanceID }) => {
    const prevent = byID("preventClose").checked;
    byID("lifecycle").textContent = prevent
      ? `Prevented close for ${surface}:${instanceID}`
      : `Allowed close for ${surface}:${instanceID}`;
    if (prevent) {
      byID("preventClose").checked = false;
    }
    return prevent;
  });

  const value = {
    status: "round-trip complete",
    extensionID: window.muxy.extensionID,
    instanceID: window.muxy.tabInstanceID,
  };
  window.muxy.storage.set("phase6-webview", value)
    .then(() => window.muxy.storage.get("phase6-webview"))
    .then((stored) => {
      byID("storage").textContent = stored && stored.status
        ? `${stored.status} · ${stored.extensionID}`
        : "Unexpected storage reply";
    })
    .catch((error) => {
      byID("storage").textContent = `Bridge error: ${error.message}`;
    });
})();
