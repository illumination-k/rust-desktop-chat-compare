export interface KeyLike {
  key: string;
  shiftKey: boolean;
  isComposing: boolean;
}

/** Enter sends, Shift+Enter inserts a newline, and Enter that confirms an IME conversion is ignored. */
export function isSendKey(event: KeyLike): boolean {
  return event.key === "Enter" && !event.shiftKey && !event.isComposing;
}
