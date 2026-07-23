import type { ThreadMode } from "./types";

/* The room's display mode is edited on /chat/info and read by /chat, so it
   lives outside both routes. Seeded once per app session from the account
   default; becomes per-chat persisted state when rooms multiply. */
class ChatRoomPrefs {
  mode = $state<ThreadMode>("chat");
  #seeded = false;

  seedDefault(mode: ThreadMode): void {
    if (!this.#seeded) {
      this.mode = mode;
      this.#seeded = true;
    }
  }
}

export const chatRoomPrefs = new ChatRoomPrefs();
