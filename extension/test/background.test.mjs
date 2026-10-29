// Runs the extension background script against mocked browser APIs.
// Usage: deno run --allow-read extension/test/background.test.mjs
const source = await Deno.readTextFile(
  new URL("../chrome/background.js", import.meta.url),
);
const firefoxSource = await Deno.readTextFile(
  new URL("../firefox/background.js", import.meta.url),
);
const contentSource = await Deno.readTextFile(
  new URL("../firefox/content.js", import.meta.url),
);
const chromeContentSource = await Deno.readTextFile(
  new URL("../chrome/content.js", import.meta.url),
);
const popupSource = await Deno.readTextFile(
  new URL("../firefox/popup.js", import.meta.url),
);
const chromePopupSource = await Deno.readTextFile(
  new URL("../chrome/popup.js", import.meta.url),
);
const firefoxManifest = JSON.parse(await Deno.readTextFile(
  new URL("../firefox/manifest.json", import.meta.url),
));
const chromeManifest = JSON.parse(await Deno.readTextFile(
  new URL("../chrome/manifest.json", import.meta.url),
));

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function waitFor(predicate, what, timeoutMs = 2000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`timed out waiting for ${what}`);
}

function settle(ms = 10) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function load(kind, initialStorage = {}) {
  const state = {
    kind,
    requests: [],
    fetches: [],
    commands: [],
    webRequestSpecs: [],
    fetchMode: "ok",
    fetchPlan: [],
    storage: { ...initialStorage },
    cookies: [],
    suggestions: 0,
    launchFails: false,
    listeners: { headersReceived: [] },
  };
  const api = {
    runtime: {
      onInstalled: { addListener: (fn) => (state.listeners.installed = fn) },
      onMessage: { addListener: (fn) => (state.listeners.message = fn) },
    },
    contextMenus: {
      create: (options) => state.commands.push(["menu", options.id, options.contexts]),
      onClicked: { addListener: (fn) => (state.listeners.menuClicked = fn) },
    },
    cookies: {
      getAll: () => Promise.resolve(state.cookies),
    },
    tabs: {
      create: (options) => {
        if (state.launchFails) return Promise.reject(new Error("tabs unavailable"));
        state.commands.push(["tab", options.url, options.active]);
        return Promise.resolve({ id: 99 });
      },
      remove: (id) => {
        state.commands.push(["tabClose", id]);
        return Promise.resolve();
      },
      onRemoved: { addListener: (fn) => (state.listeners.tabRemoved = fn) },
    },
    downloads: {
      onCreated: { addListener: (fn) => (state.listeners.downloadCreated = fn) },
      cancel: (id) => {
        state.commands.push(["cancel", id]);
        return Promise.resolve();
      },
      erase: (query) => {
        state.commands.push(["erase", query.id]);
        return Promise.resolve();
      },
      download: (options) => {
        state.commands.push(["download", options.url]);
        return Promise.resolve(1);
      },
      removeFile: (id) => {
        state.commands.push(["removeFile", id]);
        return Promise.resolve();
      },
    },
    webRequest: {
      onSendHeaders: {
        addListener: (fn, filter, spec) => {
          state.listeners.sendHeaders = fn;
          state.webRequestSpecs.push(["sendHeaders", spec]);
        },
      },
      onHeadersReceived: {
        addListener: (fn, filter, spec) => {
          state.listeners.headersReceived.push(fn);
          state.webRequestSpecs.push([
            spec.includes("blocking") ? "blocking" : "passive",
            spec,
          ]);
        },
      },
    },
    storage: {
      local: {
        get: (defaults = {}) => Promise.resolve({ ...defaults, ...state.storage }),
        set: (value) => {
          Object.assign(state.storage, value);
          const changes = {};
          for (const key of Object.keys(value)) {
            changes[key] = { newValue: value[key] };
          }
          state.listeners.storageChanged?.(changes, "local");
          return Promise.resolve();
        },
      },
      onChanged: { addListener: (fn) => (state.listeners.storageChanged = fn) },
    },
    action: {
      setBadgeText: (options) => {
        state.commands.push(["badge", options.text]);
        return Promise.resolve();
      },
      setBadgeBackgroundColor: () => Promise.resolve(),
    },
  };
  if (kind === "chrome") {
    api.downloads.onDeterminingFilename = {
      addListener: (fn) => (state.listeners.determining = fn),
    };
  }
  state.suggest = () => {
    state.suggestions += 1;
    state.commands.push(["suggest"]);
  };
  state.api = api;
  state.load = () => new Function(source)();

  globalThis.fetch = async (url, init) => {
    state.fetches.push({ url, init });
    if (url.includes("/oembed")) {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          title: "Test video",
          thumbnail_url: "https://example.com/thumb.jpg",
          author_name: "Author",
          provider_name: "YouTube",
        }),
      };
    }
    const mode = state.fetchPlan.length > 0 ? state.fetchPlan.shift() : state.fetchMode;
    if (mode === "throw") throw new Error("connection refused");
    state.requests.push(JSON.parse(init.body));
    if (mode === "http500") {
      return { status: 500, json: async () => ({ ok: false }) };
    }
    if (mode === "badjson") {
      return { status: 200, json: async () => JSON.parse("<html>") };
    }
    if (mode === "decline") {
      return { status: 200, json: async () => ({ ok: false, error: "declined" }) };
    }
    return { status: 200, json: async () => ({ ok: true }) };
  };

  if (kind === "chrome") {
    globalThis.chrome = api;
    delete globalThis.browser;
  } else {
    globalThis.browser = api;
    delete globalThis.chrome;
  }
  state.load();
  return state;
}

function sendHeaders(state, url, headers = {}, method = "GET") {
  state.listeners.sendHeaders({
    url,
    method,
    type: "main_frame",
    requestHeaders: Object.entries(headers).map(([name, value]) => ({ name, value })),
  });
}

function dispatchHeaders(state, details) {
  let result = {};
  for (const listener of state.listeners.headersReceived) {
    const value = listener(details);
    if (value && value.cancel) result = value;
  }
  return result;
}

function sendRuntime(state, message, sender = {}) {
  return new Promise((resolve) => {
    const keepOpen = state.listeners.message(message, sender, (response) => resolve(response));
    if (keepOpen !== true) resolve(undefined);
  });
}

function assertFetchShape(state) {
  const fetch = state.fetches[0];
  assert(fetch, "a request must be sent to DwV");
  assert(fetch.url === "http://127.0.0.1:41763/", `IPC url: ${fetch.url}`);
  assert(fetch.init.method === "POST", "IPC must use POST");
  assert(
    fetch.init.headers["Content-Type"] === "text/plain;charset=UTF-8",
    "IPC content type",
  );
  assert(fetch.init.signal, "IPC request must be abortable");
}

assert(source === firefoxSource, "chrome and firefox background scripts must be identical");
assert(contentSource === chromeContentSource, "chrome and firefox content scripts must be identical");
assert(popupSource === chromePopupSource, "chrome and firefox popup scripts must be identical");
assert(firefoxManifest.version === "1.2.8", "firefox manifest version");
assert(chromeManifest.version === "1.2.8", "chrome manifest version");
assert(
  /sendResponse\(\{\s*videos:\s*collectVideos\(\)\s*\}\)/.test(contentSource),
  "content scan-videos responds with {videos}",
);
assert(
  contentSource.includes('sendResponse({ ok: true })'),
  "content watch-on-page responds with {ok}",
);
assert(
  popupSource.includes('type: "handoff"'),
  "popup hands requests off through the background",
);
assert(
  popupSource.includes('message: { type: "video", url }'),
  "popup sends page videos through the video pipeline",
);
assert(!popupSource.includes("window.open"), "popup does not launch the protocol itself");
assert(!popupSource.includes("paused"), "popup does not rewrite the legacy paused flag");
assert(
  popupSource.includes('saveSetting("enabled", enabled.checked)')
    && popupSource.includes('saveSetting("captureFiles", captureFiles.checked)')
    && popupSource.includes('saveSetting("captureVideo", captureVideo.checked)'),
  "popup stores the three global settings as separate keys",
);

const baseHeaders = {
  "User-Agent": "TestAgent/1",
  Referer: "https://example.com/page",
  Origin: "https://example.com",
  Accept: "application/zip",
};

{
  const state = load("chrome");
  state.listeners.installed();
  const menu = state.commands.find(([name]) => name === "menu");
  assert(menu && menu[1] === "dwv-download", "menu created");
  assert(menu[2].includes("link") && menu[2].includes("image"), "menu covers link and image");

  state.cookies = [
    { name: "session", value: "ok" },
    { name: "csrf", value: "token" },
  ];
  sendHeaders(state, "https://example.com/files/archive.zip", baseHeaders);
  const handled = state.listeners.determining({
    id: 7,
    url: "https://example.com/files/archive.zip",
    finalUrl: "https://example.com/files/archive.zip",
    filename: "C:\\Users\\me\\Downloads\\archive.zip",
    referrer: "https://example.com/page",
    fileSize: 4242,
    mime: "application/zip",
  }, state.suggest);
  assert(handled === true, "onDeterminingFilename must return true");
  await waitFor(() => state.suggestions === 1, "the suggestion");

  assertFetchShape(state);
  const message = state.requests.find((request) => request.type === "download");
  assert(message, "archive must be sent as a download");
  assert(message.url === "https://example.com/files/archive.zip", "download url");
  assert(message.final_url === "https://example.com/files/archive.zip", "final url");
  assert(message.filename === "archive.zip", `basename filename: ${message.filename}`);
  assert(message.referrer === "https://example.com/page", "download referrer");
  assert(message.user_agent === "TestAgent/1", "user agent from request");
  assert(message.headers.Origin === "https://example.com", "origin header");
  assert(message.headers.Accept === "application/zip", "accept header");
  assert(message.size === 4242, "size");
  assert(message.mime === "application/zip", "mime");
  assert(message.cookies.includes("session=ok"), `cookies: ${message.cookies}`);
  assert(message.cookies.includes("csrf=token"), `cookies: ${message.cookies}`);
  assert(message.headers.Cookie === message.cookies, "cookie header matches cookies field");

  const names = state.commands.map(([name, id]) => [name, id]);
  assert(names.some(([name, id]) => name === "cancel" && id === 7), "browser copy cancelled");
  assert(names.some(([name, id]) => name === "erase" && id === 7), "browser copy erased");
  const cancelAt = state.commands.findIndex(([name, id]) => name === "cancel" && id === 7);
  const eraseAt = state.commands.findIndex(([name, id]) => name === "erase" && id === 7);
  const suggestAt = state.commands.findIndex(([name]) => name === "suggest");
  assert(cancelAt < suggestAt && eraseAt < suggestAt, "cancel and erase happen before suggest");
  assert(!state.commands.some(([name, text]) => name === "badge" && text === "!"), "success clears badge");
  console.log("ok: chrome archive accepted");
}

for (const mode of ["throw", "decline", "badjson", "http500"]) {
  const state = load("chrome");
  state.launchFails = true;
  state.fetchMode = mode;
  sendHeaders(state, "https://example.com/files/fail.zip", baseHeaders);
  state.listeners.determining({
    id: 21,
    url: "https://example.com/files/fail.zip",
    filename: "fail.zip",
    referrer: "https://example.com/page",
  }, state.suggest);
  await waitFor(() => state.suggestions === 1, `suggest after ${mode}`);
  assert(!state.commands.some(([name]) => name === "cancel"), `${mode}: no cancel`);
  assert(!state.commands.some(([name]) => name === "erase"), `${mode}: no erase`);
  assert(state.fetches.length === 1, `${mode}: hand-off attempted`);
  assert(
    state.commands.some(([name, text]) => name === "badge" && text === "!"),
    `${mode}: error badge`,
  );
}
console.log("ok: chrome fail-open on every transport failure");

{
  const state = load("chrome");
  state.listeners.determining({
    id: 8,
    url: "https://example.com/page.html",
    filename: "C:\\Users\\me\\Downloads\\page.html",
  }, state.suggest);
  await waitFor(() => state.suggestions === 1, "html suggestion");
  assert(state.requests.length === 0, "html page must not be intercepted");
  assert(!state.commands.some(([name]) => name === "cancel"), "html page must not be cancelled");

  sendHeaders(state, "https://example.com/post.zip", baseHeaders, "POST");
  state.listeners.determining({
    id: 9,
    url: "https://example.com/post.zip",
    filename: "post.zip",
  }, state.suggest);
  await waitFor(() => state.suggestions === 2, "post suggestion");
  assert(state.requests.length === 0, "POST downloads are left to the browser");
  console.log("ok: non-matching downloads left alone");
}

{
  const state = load("chrome");
  sendHeaders(state, "https://example.com/files/linux.torrent");
  state.listeners.determining({
    id: 10,
    url: "https://example.com/files/linux.torrent",
    filename: "linux.torrent",
  }, state.suggest);
  await waitFor(() => state.suggestions === 1, "torrent suggestion");
  const message = state.requests.find((request) => request.type === "torrent");
  assert(message, "torrent file must be sent as a torrent");
  assert(message.source === "https://example.com/files/linux.torrent", "torrent source");
  assert(!state.requests.some((request) => request.type === "download"), "no download message");
  assert(state.commands.some(([name, id]) => name === "cancel" && id === 10), "torrent cancelled");
  assert(state.commands.some(([name, id]) => name === "erase" && id === 10), "torrent erased");
  console.log("ok: chrome .torrent handed off");
}

{
  const state = load("chrome");
  state.listeners.installed();
  const magnet = "magnet:?xt=urn:btih:0123456789abcdef";
  await state.listeners.menuClicked({
    menuItemId: "dwv-download",
    linkUrl: magnet,
    pageUrl: "https://example.com/page",
  });
  const message = state.requests.find((request) => request.type === "torrent");
  assert(message, "magnet must be sent as a torrent");
  assert(message.source === magnet, "magnet source");
  assert(!state.commands.some(([name]) => name === "cancel"), "context menu has nothing to cancel");
  console.log("ok: context menu magnet");
}

{
  const state = load("chrome");
  state.listeners.installed();
  await state.listeners.menuClicked({
    menuItemId: "dwv-video-page",
    pageUrl: "https://www.youtube.com/watch?v=abc",
  });
  const message = state.requests.find((request) => request.type === "video");
  assert(message, "page video must be sent as a video");
  assert(message.url === "https://www.youtube.com/watch?v=abc", "page video url");
  console.log("ok: context menu page video");
  const preview = await sendRuntime(state, {
    type: "page-preview",
    url: "https://www.youtube.com/watch?v=abc",
  });
  assert(preview.found, "YouTube page preview is found");
  assert(preview.title === "Test video", `preview title: ${preview.title}`);
  assert(preview.thumbnail.includes("thumb.jpg"), "preview thumbnail");
  console.log("ok: youtube page preview");
}

{
  const state = load("firefox");
  assert(
    state.webRequestSpecs.some(([, spec]) => spec.includes("responseHeaders")),
    "firefox registers a headers listener",
  );
  assert(
    state.webRequestSpecs.some(([, spec]) => spec.includes("blocking")),
    "firefox headers listener blocks",
  );
  sendHeaders(state, "https://example.com/report.pdf", {
    "User-Agent": "TestAgent/1",
    Referer: "https://example.com/page",
  });
  const result = dispatchHeaders(state, {
    url: "https://example.com/report.pdf",
    method: "GET",
    type: "main_frame",
    statusCode: 200,
    responseHeaders: [
      { name: "Content-Disposition", value: 'attachment; filename="report.pdf"' },
      { name: "Content-Type", value: "application/pdf" },
      { name: "Content-Length", value: "100" },
    ],
  });
  assert(result.cancel === true, "attachment cancelled before the download starts");
  await waitFor(
    () => state.requests.some((request) => request.type === "download"),
    "attachment download",
  );
  const message = state.requests.find((request) => request.type === "download");
  assert(message, "attachment sent as download");
  assert(message.url === "https://example.com/report.pdf", "attachment url");
  assert(message.final_url === "https://example.com/report.pdf", "attachment final url");
  assert(message.filename === "report.pdf", `disposition filename: ${message.filename}`);
  assert(message.referrer === "https://example.com/page", "referrer");
  assert(!("cookies" in message), "attachment message has no cookies");
  assert(message.user_agent === "TestAgent/1", "attachment user agent");

  state.fetchMode = "decline";
  const declined = dispatchHeaders(state, {
    url: "https://example.com/other.pdf",
    method: "GET",
    type: "main_frame",
    statusCode: 200,
    responseHeaders: [
      { name: "Content-Disposition", value: 'attachment; filename="other.pdf"' },
    ],
  });
  assert(declined.cancel === true, "declined attachment still cancels the blocked request");
  await waitFor(
    () => state.commands.some(([name, url]) => name === "download" && url === "https://example.com/other.pdf"),
    "fail-open re-download",
  );

  const before = state.requests.length;
  const replayed = dispatchHeaders(state, {
    url: "https://example.com/other.pdf",
    method: "GET",
    type: "main_frame",
    statusCode: 200,
    responseHeaders: [
      { name: "Content-Disposition", value: 'attachment; filename="other.pdf"' },
    ],
  });
  assert(replayed.cancel !== true, "fail-open re-download is not cancelled again");
  await new Promise((resolve) => setTimeout(resolve, 30));
  assert(state.requests.length === before, "handedOff set blocks duplicate handling");
  console.log("ok: firefox attachment path");
}

{
  const state = load("firefox");
  const result = dispatchHeaders(state, {
    url: "https://example.com/embed/report.pdf",
    method: "GET",
    type: "sub_frame",
    statusCode: 200,
    responseHeaders: [
      { name: "Content-Disposition", value: 'attachment; filename="report.pdf"' },
      { name: "Content-Type", value: "application/pdf" },
    ],
  });
  assert(JSON.stringify(result) === "{}", "sub_frame response is left untouched");
  await new Promise((resolve) => setTimeout(resolve, 30));
  assert(state.fetches.length === 0, "sub_frame sends nothing");
  assert(state.requests.length === 0, "sub_frame posts no message");
  assert(
    !state.commands.some(([name]) => ["cancel", "erase", "download", "removeFile"].includes(name)),
    "sub_frame cancels nothing",
  );
  console.log("ok: firefox sub_frame ignored");
}

{
  const state = load("firefox");
  sendHeaders(state, "https://example.com/movie.mp4", baseHeaders);
  state.listeners.downloadCreated({
    id: 11,
    url: "https://example.com/movie.mp4",
    filename: "C:\\Users\\me\\Downloads\\movie.mp4",
    referrer: "https://example.com/page",
  });
  await waitFor(
    () => state.commands.some(([name, id]) => name === "cancel" && id === 11),
    "fallback cancel",
  );
  await waitFor(
    () => state.requests.some((request) => request.type === "download"),
    "fallback download sent",
  );
  assert(state.commands.some(([name, id]) => name === "erase" && id === 11), "fallback erased");

  const before = state.requests.length;
  state.listeners.downloadCreated({ id: 12, url: "https://example.com/movie.mp4", filename: "movie.mp4" });
  await new Promise((resolve) => setTimeout(resolve, 30));
  assert(state.requests.length === before, "30s handedOff set blocks duplicate handling");
  console.log("ok: firefox onCreated fallback");
}

{
  const state = load("chrome", { enabled: false });
  await settle();
  sendHeaders(state, "https://example.com/files/off.zip", baseHeaders);
  state.listeners.determining({
    id: 13,
    url: "https://example.com/files/off.zip",
    filename: "off.zip",
  }, state.suggest);
  await waitFor(() => state.suggestions === 1, "disabled suggestion");
  assert(state.requests.length === 0, "disabled plugin sends nothing");
  assert(!state.commands.some(([name]) => name === "cancel"), "disabled plugin cancels nothing");
  const status = await sendRuntime(state, { type: "get-status" });
  assert(status.enabled === false, "get-status reports the disabled plugin");

  const denied = await sendRuntime(state, {
    type: "handoff",
    message: { type: "video", url: "https://example.com/movie.mp4" },
  });
  assert(!denied.ok, "disabled plugin rejects video hand-off");
  assert(state.fetches.length === 0, "rejected hand-off sends nothing");
  await state.listeners.menuClicked({
    menuItemId: "dwv-download",
    linkUrl: "magnet:?xt=urn:btih:0123456789abcdef",
  });
  assert(state.fetches.length === 0, "disabled plugin ignores the context menu");
  console.log("ok: enabled=false stops automatic capture");
}

{
  const state = load("chrome", { captureFiles: false });
  await settle();
  sendHeaders(state, "https://example.com/files/skip.zip", baseHeaders);
  state.listeners.determining({
    id: 14,
    url: "https://example.com/files/skip.zip",
    filename: "skip.zip",
  }, state.suggest);
  await waitFor(() => state.suggestions === 1, "captureFiles suggestion");
  assert(state.requests.length === 0, "captureFiles=false sends nothing");
  assert(!state.commands.some(([name]) => name === "cancel"), "captureFiles=false cancels nothing");

  state.listeners.storageChanged({ captureFiles: { newValue: true } }, "local");
  sendHeaders(state, "https://example.com/files/on.zip", baseHeaders);
  state.listeners.determining({
    id: 15,
    url: "https://example.com/files/on.zip",
    filename: "on.zip",
  }, state.suggest);
  await waitFor(
    () => state.requests.some((request) => request.url === "https://example.com/files/on.zip"),
    "captureFiles toggle on",
  );
  assert(state.commands.some(([name, id]) => name === "cancel" && id === 15), "toggle on captures");
  console.log("ok: captureFiles=false stops file capture");
}

{
  const state = load("firefox");
  state.fetchPlan = ["throw", "throw", "ok"];
  const reply = await sendRuntime(state, {
    type: "handoff",
    message: { type: "video", url: "https://example.com/stream/index.m3u8" },
  });
  assert(reply.ok, `video handoff retried after launch: ${JSON.stringify(reply)}`);
  const message = state.requests.find((request) => request.type === "video");
  assert(message, "video hand-off uses the video type");
  assert(message.url === "https://example.com/stream/index.m3u8", "video url");
  const tab = state.commands.find(([name]) => name === "tab");
  assert(tab, "dwv:// tab opened when DwV was offline");
  assert(tab[1] === "dwv://", `launch deep link must not add a duplicate: ${tab[1]}`);
  assert(tab[2] === true, "launch tab is active");
  await waitFor(
    () => state.commands.some(([name]) => name === "tabClose"),
    "launch tab closed",
    9000,
  );
  assert(state.fetches.length === 3, "POST retried until DwV answered");
  assert(!state.commands.some(([name, text]) => name === "badge" && text === "!"), "success clears badge");

  const ping = await sendRuntime(state, { type: "handoff", message: { type: "ping" } });
  assert(ping.ok, "ping hand-off works");
  assert(state.requests.some((request) => request.type === "ping"), "ping reaches DwV");
  console.log("ok: handoff launches DwV and retries");
}

{
  const state = load("firefox");
  await settle();
  dispatchHeaders(state, {
    url: "https://cdn.example.com/hls/index.m3u8",
    method: "GET",
    type: "xmlhttprequest",
    tabId: 5,
    statusCode: 200,
    responseHeaders: [
      { name: "Content-Type", value: "application/vnd.apple.mpegurl" },
      { name: "Content-Length", value: "1234" },
    ],
  });
  dispatchHeaders(state, {
    url: "https://cdn.example.com/clip.mp4",
    method: "GET",
    type: "media",
    tabId: 5,
    statusCode: 200,
    responseHeaders: [
      { name: "Content-Type", value: "video/mp4" },
      { name: "Content-Length", value: "5000" },
    ],
  });
  dispatchHeaders(state, {
    url: "https://cdn.example.com/segment.ts",
    method: "GET",
    type: "xmlhttprequest",
    tabId: 5,
    statusCode: 200,
    responseHeaders: [{ name: "Content-Type", value: "video/mp2t" }],
  });
  dispatchHeaders(state, {
    url: "https://rr1---sn.googlevideo.com/videoplayback?mime=video%2Fmp4&id=abc",
    method: "GET",
    type: "media",
    tabId: 5,
    statusCode: 206,
    responseHeaders: [{ name: "Content-Type", value: "application/octet-stream" }],
  });

  sendRuntime(state, {
    type: "videos",
    videos: [
      {
        url: "https://cdn.example.com/hls/index.m3u8",
        title: "Поток",
        kind: "stream",
        width: 1920,
        height: 1080,
        duration: 42,
        size: 0,
      },
      {
        url: "https://example.com/page-video.mp4",
        title: "Страница",
        kind: "file",
        width: 0,
        height: 0,
        duration: 0,
        size: 0,
      },
    ],
  }, { tab: { id: 5 } });

  const reply = await sendRuntime(state, { type: "videos-for-tab", tabId: 5 });
  assert(Array.isArray(reply.videos), "videos-for-tab returns {videos}");
  assert(reply.videos.length === 4, `merged videos: ${reply.videos.length}`);
  const hls = reply.videos.find((video) => video.url.endsWith("index.m3u8"));
  assert(hls && hls.title === "Поток", "content metadata wins for duplicate urls");
  assert(hls.size === 1234, "network size fills the content entry");
  assert(hls.width === 1920, "content dimensions are kept");
  const clip = reply.videos.find((video) => video.url.endsWith("clip.mp4"));
  assert(clip && clip.source === "network", "network media is listed");
  assert(!reply.videos.some((video) => video.url.endsWith("segment.ts")), "media segments are ignored");
  assert(reply.videos.some((video) => video.url.includes("googlevideo.com")), "YouTube mime query is detected");

  state.listeners.tabRemoved(5);
  const cleared = await sendRuntime(state, { type: "videos-for-tab", tabId: 5 });
  assert(cleared.videos.length === 0, "tab media is dropped when the tab closes");
  console.log("ok: network media merged with content videos");
}

{
  const state = load("firefox", { captureVideo: false });
  await settle();
  dispatchHeaders(state, {
    url: "https://cdn.example.com/hidden.mp4",
    method: "GET",
    type: "media",
    tabId: 6,
    statusCode: 200,
    responseHeaders: [{ name: "Content-Type", value: "video/mp4" }],
  });
  sendRuntime(state, {
    type: "videos",
    videos: [{ url: "https://example.com/page.mp4", title: "Скрытое" }],
  }, { tab: { id: 6 } });
  const reply = await sendRuntime(state, { type: "videos-for-tab", tabId: 6 });
  assert(reply.videos.length === 0, "captureVideo=false disables media discovery");
  console.log("ok: captureVideo=false disables media discovery");
}

console.log("extension tests passed");
Deno.exit(0);
