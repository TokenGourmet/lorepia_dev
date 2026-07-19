import { describe, expect, it, vi } from "vitest";

import {
  AUDIO_ACTIONS,
  AUDIO_BACKEND,
  AUDIO_FIXTURE,
  AUDIO_POLICY_VERSION,
  type AudioActionCounts,
  type AudioReceipt,
} from "./audio-contract";
import { installAudioLifecycleHooks } from "./audio-lifecycle";

function actionCounts(appPhase: "FOREGROUND" | "BACKGROUND"): AudioActionCounts {
  const counts = {} as AudioActionCounts;
  for (const action of AUDIO_ACTIONS) counts[action] = 0;
  counts.INITIALIZE = 1;
  counts.BACKGROUND = 1;
  counts.FOREGROUND = appPhase === "FOREGROUND" ? 1 : 0;
  return counts;
}

function receipt(appPhase: "FOREGROUND" | "BACKGROUND"): AudioReceipt {
  const action: "BACKGROUND" | "FOREGROUND" =
    appPhase === "BACKGROUND" ? "BACKGROUND" : "FOREGROUND";
  const transition = {
    seq: appPhase === "BACKGROUND" ? 1 : 2,
    action,
    before: "RELEASED" as const,
    after: "RELEASED" as const,
    positionMs: 0,
  };
  return {
    protocolVersion: 1,
    policyVersion: AUDIO_POLICY_VERSION,
    backend: AUDIO_BACKEND,
    actionCounts: actionCounts(appPhase),
    fixture: AUDIO_FIXTURE,
    verification: {
      bytesAndSha256Matched: false,
      wavHeaderMatched: false,
      metadataDurationMatched: false,
    },
    observations: {
      playProgressDeltaMs: 0,
      pauseWindowMs: 0,
      pauseDriftMs: 0,
      resumeProgressDeltaMs: 0,
      seekEventObserved: false,
      stopResetObserved: false,
    },
    events: {
      loadedMetadataCount: 0,
      timeUpdateCount: 0,
      seekedCount: 0,
      endedCount: 0,
      mediaErrorCount: 0,
    },
    state: "RELEASED",
    appPhase,
    positionMs: 0,
    transition,
    lifecycle: {
      backgroundCount: 1,
      foregroundCount: appPhase === "FOREGROUND" ? 1 : 0,
      automaticPauseCount: 0,
      automaticReleaseCount: 0,
      automaticResumeCount: 0,
    },
    resources: {
      elementAllocated: false,
      sourceAttached: false,
      objectUrlActive: false,
      listenerCount: 0,
      osResourceRelease: "UNVERIFIED_REQUIRES_DEVICE_EVIDENCE",
    },
    trace: [transition],
  };
}

class FakeVisibilityTarget extends EventTarget {
  visibilityState: "hidden" | "visible" | "prerender" = "visible";
}

describe("candidate WebView lifecycle hooks", () => {
  it("maps hidden/pagehide to background and visible/pageshow to foreground", () => {
    const background = receipt("BACKGROUND");
    const foreground = receipt("FOREGROUND");
    const controller = {
      enterBackground: vi.fn(() => background),
      enterForeground: vi.fn(() => foreground),
    };
    const visibility = new FakeVisibilityTarget();
    const page = new EventTarget();
    const onReceipt = vi.fn();
    const remove = installAudioLifecycleHooks(controller, visibility, page, onReceipt);

    visibility.visibilityState = "hidden";
    visibility.dispatchEvent(new Event("visibilitychange"));
    page.dispatchEvent(new Event("pagehide"));
    visibility.visibilityState = "visible";
    visibility.dispatchEvent(new Event("visibilitychange"));
    page.dispatchEvent(new Event("pageshow"));

    expect(controller.enterBackground).toHaveBeenCalledTimes(2);
    expect(controller.enterForeground).toHaveBeenCalledTimes(2);
    expect(onReceipt.mock.calls.map(([value]) => value.appPhase)).toEqual([
      "BACKGROUND",
      "BACKGROUND",
      "FOREGROUND",
      "FOREGROUND",
    ]);

    remove();
    page.dispatchEvent(new Event("pagehide"));
    page.dispatchEvent(new Event("pageshow"));
    expect(controller.enterBackground).toHaveBeenCalledTimes(2);
    expect(controller.enterForeground).toHaveBeenCalledTimes(2);
  });

  it("ignores prerender visibility without inventing a lifecycle transition", () => {
    const controller = {
      enterBackground: vi.fn(() => receipt("BACKGROUND")),
      enterForeground: vi.fn(() => receipt("FOREGROUND")),
    };
    const visibility = new FakeVisibilityTarget();
    const page = new EventTarget();
    const remove = installAudioLifecycleHooks(controller, visibility, page);
    visibility.visibilityState = "prerender";
    visibility.dispatchEvent(new Event("visibilitychange"));
    expect(controller.enterBackground).not.toHaveBeenCalled();
    expect(controller.enterForeground).not.toHaveBeenCalled();
    remove();
  });
});
