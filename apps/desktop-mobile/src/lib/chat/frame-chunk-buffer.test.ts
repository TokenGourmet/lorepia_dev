import { describe, expect, it } from "vitest";

import {
  createFrameChunkBuffer,
  type AnimationFrameClock,
} from "./frame-chunk-buffer";

class FakeAnimationFrameClock implements AnimationFrameClock {
  private nextFrameId = 1;
  private callbacks = new Map<number, (timestamp: number) => void>();
  requestCount = 0;
  cancelCount = 0;

  request(callback: (timestamp: number) => void): number {
    const frameId = this.nextFrameId++;
    this.requestCount += 1;
    this.callbacks.set(frameId, callback);
    return frameId;
  }

  cancel(frameId: number): void {
    this.cancelCount += 1;
    this.callbacks.delete(frameId);
  }

  get pendingFrames(): number {
    return this.callbacks.size;
  }

  runNext(timestamp = 0): void {
    const next = this.callbacks.entries().next();
    if (next.done) {
      throw new Error("NO_PENDING_ANIMATION_FRAME");
    }
    const [frameId, callback] = next.value;
    this.callbacks.delete(frameId);
    callback(timestamp);
  }
}

describe("frame chunk buffer", () => {
  it("commits many ordered deltas with one UI update in one frame", () => {
    const clock = new FakeAnimationFrameClock();
    let fullMessageUpdates = 0;
    let messages = [
      { id: "user", chunks: ["질문"] },
      { id: "assistant", chunks: [] as string[] },
    ];
    const buffer = createFrameChunkBuffer((text) => {
      fullMessageUpdates += 1;
      messages = messages.map((message) =>
        message.id === "assistant"
          ? { ...message, chunks: [...message.chunks, text] }
          : message,
      );
    }, clock);

    buffer.append("한");
    buffer.append("글 ");
    buffer.append("👩");
    buffer.append("\u200d");
    buffer.append("💻 e");
    buffer.append("\u0301");

    expect(clock.requestCount).toBe(1);
    expect(clock.pendingFrames).toBe(1);
    expect(fullMessageUpdates).toBe(0);

    clock.runNext();

    expect(fullMessageUpdates).toBe(1);
    expect(messages[1]?.chunks.join("")).toBe("한글 👩‍💻 é");
    expect(clock.pendingFrames).toBe(0);
  });

  it("allows at most one UI commit per animation frame", () => {
    const clock = new FakeAnimationFrameClock();
    const commits: string[] = [];
    const buffer = createFrameChunkBuffer((text) => commits.push(text), clock);

    buffer.append("a");
    buffer.append("b");
    clock.runNext(16);
    buffer.append("c");
    buffer.append("d");

    expect(commits).toEqual(["ab"]);
    expect(clock.requestCount).toBe(2);

    clock.runNext(32);
    expect(commits).toEqual(["ab", "cd"]);
  });

  it("drains every pending delta before terminal without a stale frame", () => {
    const clock = new FakeAnimationFrameClock();
    const commits: string[] = [];
    const buffer = createFrameChunkBuffer((text) => commits.push(text), clock);

    buffer.append("끝나기 ");
    buffer.append("직전");
    const terminalText = buffer.close();

    expect(terminalText).toBe("끝나기 직전");
    expect(commits).toEqual([]);
    expect(clock.pendingFrames).toBe(0);
    expect(clock.cancelCount).toBe(1);

    buffer.append("terminal 뒤 delta");
    expect(clock.pendingFrames).toBe(0);
    expect(buffer.close()).toBe("");
  });

  it("flushes pending text on user cancellation and remains usable until terminal", () => {
    const clock = new FakeAnimationFrameClock();
    const commits: string[] = [];
    const buffer = createFrameChunkBuffer((text) => commits.push(text), clock);

    buffer.append("보이던 응답");
    buffer.flush();

    expect(commits).toEqual(["보이던 응답"]);
    expect(clock.pendingFrames).toBe(0);

    buffer.append("취소 경합 중 도착");
    expect(buffer.close()).toBe("취소 경합 중 도착");
    expect(clock.pendingFrames).toBe(0);
  });

  it("cancels a pending frame without committing after view destruction or reload", () => {
    const clock = new FakeAnimationFrameClock();
    const commits: string[] = [];
    const buffer = createFrameChunkBuffer((text) => commits.push(text), clock);

    buffer.append("폐기할 화면 상태");
    buffer.cancel();

    expect(commits).toEqual([]);
    expect(clock.pendingFrames).toBe(0);
    expect(clock.cancelCount).toBe(1);
  });
});
