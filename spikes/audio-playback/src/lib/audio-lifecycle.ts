import type { AudioReceipt } from "./audio-contract";
import type { AudioM1Controller } from "./audio-controller";

export type VisibilityTarget = EventTarget & {
  readonly visibilityState: "hidden" | "visible" | "prerender";
};

export function installAudioLifecycleHooks(
  controller: Pick<AudioM1Controller, "enterBackground" | "enterForeground">,
  visibilityTarget: VisibilityTarget = document,
  pageTarget: EventTarget = window,
  onReceipt?: (receipt: AudioReceipt) => void,
): () => void {
  const publish = (receipt: AudioReceipt): void => onReceipt?.(receipt);
  const onVisibilityChange = (): void => {
    if (visibilityTarget.visibilityState === "hidden") {
      publish(controller.enterBackground());
    } else if (visibilityTarget.visibilityState === "visible") {
      publish(controller.enterForeground());
    }
  };
  const onPageHide = (): void => publish(controller.enterBackground());
  const onPageShow = (): void => publish(controller.enterForeground());

  visibilityTarget.addEventListener("visibilitychange", onVisibilityChange);
  pageTarget.addEventListener("pagehide", onPageHide);
  pageTarget.addEventListener("pageshow", onPageShow);

  return () => {
    visibilityTarget.removeEventListener("visibilitychange", onVisibilityChange);
    pageTarget.removeEventListener("pagehide", onPageHide);
    pageTarget.removeEventListener("pageshow", onPageShow);
  };
}
