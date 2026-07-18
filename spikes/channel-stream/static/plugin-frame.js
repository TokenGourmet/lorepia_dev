"use strict";

(() => {
  const PROTOCOL_VERSION = 1;
  const BROKER_TIMEOUT_MS = 2_000;
  const PROBE_TIMEOUT_MS = 2_000;
  const DETAIL_LIMIT = 256;
  const NONCE_PATTERN = /^[0-9a-f]{32,128}$/;
  const REQUEST_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;
  const RUN_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;
  const RESPONSE_SUCCESS_KEYS = [
    "version",
    "type",
    "sessionNonce",
    "requestId",
    "ok",
    "result",
  ];
  const RESPONSE_ERROR_KEYS = [
    "version",
    "type",
    "sessionNonce",
    "requestId",
    "ok",
    "error",
  ];
  const ERROR_KEYS = ["code", "message"];
  const WATCHDOG_PING_KEYS = ["type", "sessionId", "seq"];
  const RUN_SUITE_KEYS = ["type", "sessionNonce", "runId"];
  const SET_WATCHDOG_MODE_KEYS = ["type", "sessionNonce", "mode"];
  const DANGEROUS_HTML =
    '<p onclick="attack()">safe<script>attack()</' +
    "script>" +
    '<img src="x" onerror="attack()"><a href="javascript:attack()">link</a></p>';

  const sessionNonce = new URL(window.location.href).searchParams.get("session");
  if (sessionNonce === null || !NONCE_PATTERN.test(sessionNonce)) return;

  let watchdogMode = "normal";
  let suiteRunning = false;
  let runSequence = 0;
  let requestSequence = 0;
  const pendingBrokerRequests = new Map();

  function isPlainRecord(value) {
    return (
      typeof value === "object" &&
      value !== null &&
      !Array.isArray(value) &&
      Object.getPrototypeOf(value) === Object.prototype
    );
  }

  function hasExactKeys(value, expected) {
    if (!isPlainRecord(value)) return false;
    const keys = Object.keys(value);
    return (
      keys.length === expected.length &&
      expected.every((key) => Object.prototype.hasOwnProperty.call(value, key))
    );
  }

  function boundedDetail(value) {
    let detail;
    if (typeof value === "string") {
      detail = value;
    } else if (value instanceof Error) {
      detail = `${value.name}: ${value.message}`;
    } else {
      try {
        detail = JSON.stringify(value);
      } catch {
        detail = String(value);
      }
    }
    if (typeof detail !== "string" || detail.length === 0) detail = "No detail";
    return detail.slice(0, DETAIL_LIMIT);
  }

  function postToHost(message) {
    parent.postMessage(message, "*");
  }

  function postTestResult(runId, testId, passed, detail) {
    postToHost({
      type: "lorepia:plugin:test-result",
      sessionNonce,
      runId,
      testId,
      passed,
      detail: boundedDetail(detail),
    });
  }

  function nextRequestId(label) {
    requestSequence += 1;
    return `p-${runSequence}-${requestSequence}-${label}`.slice(0, 64);
  }

  function withTimeout(promise, timeoutMs, label) {
    return new Promise((resolve, reject) => {
      const timer = window.setTimeout(() => {
        reject(new Error(`${label} timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      Promise.resolve(promise).then(
        (value) => {
          window.clearTimeout(timer);
          resolve(value);
        },
        (error) => {
          window.clearTimeout(timer);
          reject(error);
        },
      );
    });
  }

  function parseBrokerResponse(value) {
    if (!isPlainRecord(value) || value.type !== "response") return null;
    if (
      value.version !== PROTOCOL_VERSION ||
      value.sessionNonce !== sessionNonce ||
      typeof value.requestId !== "string" ||
      !REQUEST_ID_PATTERN.test(value.requestId)
    ) {
      return null;
    }

    if (value.ok === true) {
      return hasExactKeys(value, RESPONSE_SUCCESS_KEYS) ? value : null;
    }

    if (
      value.ok !== false ||
      !hasExactKeys(value, RESPONSE_ERROR_KEYS) ||
      !hasExactKeys(value.error, ERROR_KEYS) ||
      typeof value.error.code !== "string" ||
      typeof value.error.message !== "string" ||
      value.error.message.length > DETAIL_LIMIT
    ) {
      return null;
    }
    return value;
  }

  function brokerRequest(method, payload, options = {}) {
    const requestId = options.requestId ?? nextRequestId("broker");
    const request = {
      version: PROTOCOL_VERSION,
      type: "request",
      sessionNonce,
      requestId,
      method,
      payload,
      ...(options.extraEnvelope ?? {}),
    };

    return new Promise((resolve, reject) => {
      if (pendingBrokerRequests.has(requestId)) {
        reject(new Error("A broker request with this id is already pending"));
        return;
      }

      const timer = window.setTimeout(() => {
        pendingBrokerRequests.delete(requestId);
        reject(new Error(`broker request ${requestId} timed out`));
      }, BROKER_TIMEOUT_MS);
      pendingBrokerRequests.set(requestId, { resolve, timer });
      postToHost(request);
    });
  }

  function receiveBrokerResponse(value) {
    const response = parseBrokerResponse(value);
    if (response === null) return false;

    const pending = pendingBrokerRequests.get(response.requestId);
    if (pending === undefined) return true;
    pendingBrokerRequests.delete(response.requestId);
    window.clearTimeout(pending.timer);
    pending.resolve(response);
    return true;
  }

  function testParentDocumentBlocked() {
    try {
      void parent.document.documentElement;
      return { passed: false, detail: "parent.document was readable" };
    } catch (error) {
      return {
        passed: true,
        detail: `parent.document blocked (${boundedDetail(error)})`,
      };
    }
  }

  function testLocalStorageBlocked() {
    const key = "lorepia-isolation-probe";
    try {
      window.localStorage.setItem(key, "should-not-persist");
      window.localStorage.removeItem(key);
      return { passed: false, detail: "localStorage write succeeded" };
    } catch (error) {
      return {
        passed: true,
        detail: `localStorage blocked (${boundedDetail(error)})`,
      };
    }
  }

  function testHostTokenStorageInaccessible() {
    const key = "lorepia.m1.host-broker-token.v1";
    const journalKey = "lorepia.m1.host-broker-rotation.v1";
    const probeKey = "lorepia-isolation-session-storage-probe";
    try {
      const observed = window.sessionStorage.getItem(key);
      const observedJournal = window.sessionStorage.getItem(journalKey);
      if (observed !== null || observedJournal !== null) {
        return {
          passed: false,
          detail: "host broker token material was visible in sessionStorage",
        };
      }
      window.sessionStorage.setItem(probeKey, "opaque-frame");
      const isolated = window.sessionStorage.getItem(probeKey) === "opaque-frame";
      window.sessionStorage.removeItem(probeKey);
      return {
        passed: isolated,
        detail: isolated
          ? "frame sessionStorage is isolated and contains no host token"
          : "frame sessionStorage behaved unexpectedly",
      };
    } catch (error) {
      return {
        passed: true,
        detail: `host sessionStorage blocked (${boundedDetail(error)})`,
      };
    }
  }

  function testWindowOpenBlocked() {
    try {
      const opened = window.open("about:blank", "_blank");
      if (opened === null) {
        return { passed: true, detail: "window.open returned null" };
      }
      try {
        opened.close();
      } catch {
        // The unexpected popup is already a failed test; cleanup is best effort.
      }
      return { passed: false, detail: "window.open returned a WindowProxy" };
    } catch (error) {
      return {
        passed: true,
        detail: `window.open blocked (${boundedDetail(error)})`,
      };
    }
  }

  async function testExternalFetchBlocked() {
    let violation = null;
    let acceptViolation;
    const violationPromise = new Promise((resolve) => {
      acceptViolation = resolve;
    });
    const onViolation = (event) => {
      if (
        event.effectiveDirective === "connect-src" ||
        event.violatedDirective === "connect-src"
      ) {
        violation = {
          effectiveDirective: event.effectiveDirective,
          blockedURI: event.blockedURI,
        };
        acceptViolation(violation);
      }
    };
    document.addEventListener("securitypolicyviolation", onViolation);

    let fetchRejected = false;
    try {
      await withTimeout(
        fetch("https://example.invalid/lorepia-plugin-csp-probe", {
          cache: "no-store",
          credentials: "omit",
          mode: "cors",
        }),
        PROBE_TIMEOUT_MS,
        "external fetch",
      );
    } catch {
      fetchRejected = true;
    }

    try {
      await withTimeout(violationPromise, PROBE_TIMEOUT_MS, "CSP violation event");
    } catch {
      // The result below records the missing violation as a failed probe.
    }
    document.removeEventListener("securitypolicyviolation", onViolation);
    return {
      passed: fetchRejected && violation !== null,
      detail:
        fetchRejected && violation !== null
          ? `fetch rejected; ${violation.effectiveDirective} blocked ${violation.blockedURI}`
          : `fetchRejected=${fetchRejected}; cspViolation=${violation !== null}`,
    };
  }

  function tauriInvokeOrNull() {
    const internals = globalThis.__TAURI_INTERNALS__;
    return internals !== undefined && typeof internals.invoke === "function"
      ? internals.invoke.bind(internals)
      : null;
  }

  function nativeErrorCode(error) {
    let candidate = error;
    if (typeof candidate === "string") {
      try {
        candidate = JSON.parse(candidate);
      } catch {
        const match = candidate.match(/\b[A-Z][A-Z_]{2,63}\b/);
        return match === null ? null : match[0];
      }
    }
    return isPlainRecord(candidate) && typeof candidate.code === "string"
      ? candidate.code
      : null;
  }

  function isTimeoutError(error) {
    return error instanceof Error && /timed out after \d+ms$/.test(error.message);
  }

  function directNativeRequestJson(label, method = "probe.increment") {
    return JSON.stringify({
      request_id: nextRequestId(label),
      method,
      payload: {},
    });
  }

  async function testDirectBrokerMissingTokenDenied() {
    const directInvoke = tauriInvokeOrNull();
    if (directInvoke === null) {
      return { passed: true, detail: "Tauri invoke transport is absent in this frame" };
    }
    try {
      await withTimeout(
        directInvoke("host_broker_request", {
          hostToken: null,
          requestJson: directNativeRequestJson("missing-token"),
        }),
        PROBE_TIMEOUT_MS,
        "host broker without token",
      );
      return { passed: false, detail: "host_broker_request accepted a missing token" };
    } catch (error) {
      const code = nativeErrorCode(error);
      const timedOut = isTimeoutError(error);
      return {
        passed: code === "MISSING_HOST_TOKEN",
        detail:
          code === "MISSING_HOST_TOKEN"
            ? `missing token returned ${code}`
            : timedOut
              ? "INCONCLUSIVE: missing-token native callback timed out"
              : `missing token returned ${boundedDetail(error)}`,
      };
    }
  }

  async function testDirectBrokerWrongTokenDenied() {
    const directInvoke = tauriInvokeOrNull();
    if (directInvoke === null) {
      return { passed: true, detail: "Tauri invoke transport is absent in this frame" };
    }
    try {
      await withTimeout(
        directInvoke("host_broker_request", {
          hostToken: "0".repeat(64),
          requestJson: directNativeRequestJson("wrong-token"),
        }),
        PROBE_TIMEOUT_MS,
        "host broker with wrong token",
      );
      return { passed: false, detail: "host_broker_request accepted a wrong token" };
    } catch (error) {
      const code = nativeErrorCode(error);
      const timedOut = isTimeoutError(error);
      return {
        passed: code === "INVALID_HOST_TOKEN",
        detail:
          code === "INVALID_HOST_TOKEN"
            ? `wrong token returned ${code}`
            : timedOut
              ? "INCONCLUSIVE: wrong-token native callback timed out"
              : `wrong token returned ${boundedDetail(error)}`,
      };
    }
  }

  async function testDirectRegistrationTakeoverDenied() {
    const directInvoke = tauriInvokeOrNull();
    if (directInvoke === null) {
      return { passed: true, detail: "Tauri invoke transport is absent in this frame" };
    }
    const attackerToken = "0".repeat(64);
    let registrationDetail;
    try {
      await withTimeout(
        directInvoke("register_host_broker_session", {
          hostToken: attackerToken,
          policy: {
            moduleId: "fixture.plugin",
            manifestPermissions: ["probe.increment"],
            approvedPermissions: ["probe.increment"],
          },
        }),
        PROBE_TIMEOUT_MS,
        "host broker registration takeover",
      );
      return { passed: false, detail: "broker registration accepted the attacker token" };
    } catch (error) {
      const code = nativeErrorCode(error);
      const timedOut = isTimeoutError(error);
      if (code !== "INVALID_HOST_TOKEN") {
        return {
          passed: false,
          detail: timedOut
            ? "INCONCLUSIVE: registration takeover native callback timed out"
            : `registration takeover returned ${boundedDetail(error)}`,
        };
      }
      registrationDetail = `registration returned ${code}`;
    }

    try {
      await withTimeout(
        directInvoke("host_broker_request", {
          hostToken: attackerToken,
          requestJson: directNativeRequestJson("takeover-attacker-probe"),
        }),
        PROBE_TIMEOUT_MS,
        "attacker token after registration takeover",
      );
      return {
        passed: false,
        detail: "attacker token became valid after registration takeover",
      };
    } catch (error) {
      const code = nativeErrorCode(error);
      const timedOut = isTimeoutError(error);
      return {
        passed: code === "INVALID_HOST_TOKEN",
        detail:
          code === "INVALID_HOST_TOKEN"
            ? `${registrationDetail}; attacker probe returned ${code}`
            : timedOut
              ? `INCONCLUSIVE: ${registrationDetail}; attacker probe native callback timed out`
              : `attacker probe returned ${boundedDetail(error)}`,
      };
    }
  }

  async function testBrokerStateRead() {
    const response = await brokerRequest("state.read", {});
    return {
      passed: response.ok === true,
      detail: response.ok
        ? "state.read returned a broker result"
        : `state.read failed with ${response.error.code}`,
    };
  }

  async function testBrokerSanitize() {
    const response = await brokerRequest("render.sanitize", {
      html: DANGEROUS_HTML,
    });
    if (!response.ok) {
      return {
        passed: false,
        detail: `render.sanitize failed with ${response.error.code}`,
      };
    }

    const html =
      isPlainRecord(response.result) && typeof response.result.html === "string"
        ? response.result.html
        : null;
    const unsafe =
      html === null ||
      /<script|<img|onerror|onclick|javascript:/i.test(html) ||
      !html.includes("safe");
    return {
      passed: !unsafe,
      detail:
        html === null
          ? "render.sanitize did not return result.html"
          : `sanitized=${boundedDetail(html)}`,
    };
  }

  async function testBrokerProbeIncrement() {
    const response = await brokerRequest("probe.increment", {});
    const result = response.ok && isPlainRecord(response.result) ? response.result : null;
    const passed =
      result !== null &&
      result.sentinel === "LOREPIA_HOST_BROKER_PROBE_REACHED" &&
      Number.isSafeInteger(result.callCount);
    return {
      passed,
      detail: response.ok
        ? passed
          ? `authenticated probe count=${result.callCount}`
          : "probe.increment returned an invalid broker result"
        : `probe.increment failed with ${response.error.code}`,
    };
  }

  async function testBrokerNetworkDenied() {
    const response = await brokerRequest("network.fetch", {
      url: "https://example.invalid/lorepia-broker-probe",
    });
    return {
      passed: response.ok === false && response.error.code === "NETWORK_DENIED",
      detail: response.ok
        ? "network.fetch unexpectedly succeeded"
        : `network.fetch returned ${response.error.code}`,
    };
  }

  async function testBrokerSecretDenied() {
    const response = await brokerRequest("secret.read", {});
    return {
      passed:
        response.ok === false && response.error.code === "PERMISSION_DENIED",
      detail: response.ok
        ? "secret.read unexpectedly succeeded"
        : `secret.read returned ${response.error.code}`,
    };
  }

  async function testBrokerReplayRejected() {
    const requestId = nextRequestId("replay");
    const first = await brokerRequest("state.read", {}, { requestId });
    if (!first.ok) {
      return {
        passed: false,
        detail: `replay setup failed with ${first.error.code}`,
      };
    }

    const second = await brokerRequest("state.read", {}, { requestId });
    return {
      passed: second.ok === false && second.error.code === "REPLAYED_REQUEST",
      detail: second.ok
        ? "replayed request unexpectedly succeeded"
        : `replayed request returned ${second.error.code}`,
    };
  }

  async function testUnknownEnvelopeRejected() {
    const response = await brokerRequest("state.read", {}, {
      extraEnvelope: { unexpected: true },
    });
    return {
      passed:
        response.ok === false && response.error.code === "MALFORMED_REQUEST",
      detail: response.ok
        ? "request with unknown envelope field unexpectedly succeeded"
        : `unknown envelope field returned ${response.error.code}`,
    };
  }

  const TESTS = [
    ["parent-document-blocked", testParentDocumentBlocked],
    ["local-storage-blocked", testLocalStorageBlocked],
    ["host-token-storage-inaccessible", testHostTokenStorageInaccessible],
    ["window-open-blocked", testWindowOpenBlocked],
    ["external-fetch-csp-blocked", testExternalFetchBlocked],
    ["direct-broker-missing-token-denied", testDirectBrokerMissingTokenDenied],
    ["direct-broker-wrong-token-denied", testDirectBrokerWrongTokenDenied],
    ["direct-registration-takeover-denied", testDirectRegistrationTakeoverDenied],
    ["broker-state-read", testBrokerStateRead],
    ["broker-render-sanitize", testBrokerSanitize],
    ["broker-probe-increment", testBrokerProbeIncrement],
    ["broker-network-denied", testBrokerNetworkDenied],
    ["broker-secret-permission-denied", testBrokerSecretDenied],
    ["broker-replay-rejected", testBrokerReplayRejected],
    ["broker-unknown-field-rejected", testUnknownEnvelopeRejected],
  ];

  async function runSuite(runId) {
    if (suiteRunning) return;
    suiteRunning = true;
    runSequence += 1;

    try {
      for (const [testId, test] of TESTS) {
        try {
          const result = await test();
          postTestResult(runId, testId, result.passed === true, result.detail);
        } catch (error) {
          postTestResult(runId, testId, false, boundedDetail(error));
        }
      }
    } finally {
      suiteRunning = false;
    }
  }

  function receiveWatchdogPing(value) {
    if (
      !hasExactKeys(value, WATCHDOG_PING_KEYS) ||
      value.type !== "lorepia:watchdog:ping" ||
      typeof value.sessionId !== "string" ||
      value.sessionId.length === 0 ||
      value.sessionId.length > 128 ||
      !Number.isSafeInteger(value.seq) ||
      value.seq < 1
    ) {
      return false;
    }

    if (watchdogMode === "normal") {
      postToHost({
        type: "lorepia:watchdog:pong",
        sessionId: value.sessionId,
        seq: value.seq,
      });
    }
    return true;
  }

  window.addEventListener("message", (event) => {
    if (event.source !== parent) return;
    const value = event.data;

    if (receiveBrokerResponse(value) || receiveWatchdogPing(value)) return;

    if (
      hasExactKeys(value, SET_WATCHDOG_MODE_KEYS) &&
      value.type === "lorepia:plugin:set-watchdog-mode" &&
      value.sessionNonce === sessionNonce &&
      (value.mode === "normal" || value.mode === "silent")
    ) {
      watchdogMode = value.mode;
      return;
    }

    if (
      hasExactKeys(value, RUN_SUITE_KEYS) &&
      value.type === "lorepia:plugin:run-suite" &&
      value.sessionNonce === sessionNonce &&
      typeof value.runId === "string" &&
      RUN_ID_PATTERN.test(value.runId)
    ) {
      void runSuite(value.runId);
    }
  });

  postToHost({ type: "lorepia:plugin:ready", sessionNonce });
})();
