export interface AnimationFrameClock {
  request(callback: (timestamp: number) => void): number;
  cancel(frameId: number): void;
}

export interface FrameChunkBuffer {
  append(chunk: string): void;
  flush(): void;
  drain(): string;
  close(): string;
  cancel(): void;
}

const browserAnimationFrameClock: AnimationFrameClock = {
  request(callback) {
    return requestAnimationFrame(callback);
  },
  cancel(frameId) {
    cancelAnimationFrame(frameId);
  },
};

/**
 * Preserves the exact order of streamed string chunks while limiting reactive
 * UI work to one commit per animation frame.
 */
export function createFrameChunkBuffer(
  commit: (text: string) => void,
  clock: AnimationFrameClock = browserAnimationFrameClock,
): FrameChunkBuffer {
  let chunks: string[] = [];
  let scheduledFrame: number | null = null;
  let closed = false;

  function cancelScheduledFrame(): void {
    if (scheduledFrame === null) return;
    clock.cancel(scheduledFrame);
    scheduledFrame = null;
  }

  function takePendingText(): string {
    if (chunks.length === 0) return "";
    const pending = chunks.join("");
    chunks = [];
    return pending;
  }

  function commitPendingText(): void {
    const pending = takePendingText();
    if (pending.length > 0) {
      commit(pending);
    }
  }

  return {
    append(chunk) {
      if (closed || chunk.length === 0) return;
      chunks.push(chunk);
      if (scheduledFrame !== null) return;

      scheduledFrame = clock.request(() => {
        scheduledFrame = null;
        if (closed) return;
        commitPendingText();
      });
    },

    flush() {
      if (closed) return;
      cancelScheduledFrame();
      commitPendingText();
    },

    drain() {
      if (closed) return "";
      cancelScheduledFrame();
      return takePendingText();
    },

    close() {
      if (closed) return "";
      closed = true;
      cancelScheduledFrame();
      return takePendingText();
    },

    cancel() {
      if (closed) return;
      closed = true;
      cancelScheduledFrame();
      chunks = [];
    },
  };
}
