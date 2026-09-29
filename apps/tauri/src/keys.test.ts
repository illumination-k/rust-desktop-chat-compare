import { expect, test } from "vitest";
import { isSendKey } from "./keys";

test("plain Enter sends", () => {
  expect(isSendKey({ key: "Enter", shiftKey: false, isComposing: false })).toBe(true);
});

test("Shift+Enter inserts a newline", () => {
  expect(isSendKey({ key: "Enter", shiftKey: true, isComposing: false })).toBe(false);
});

test("Enter during IME composition does not send", () => {
  expect(isSendKey({ key: "Enter", shiftKey: false, isComposing: true })).toBe(false);
});

test("other keys do not send", () => {
  expect(isSendKey({ key: "a", shiftKey: false, isComposing: false })).toBe(false);
});
