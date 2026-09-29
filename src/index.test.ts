import { expect, test } from "vitest";
import { hello } from "./index";

test("hello", () => {
  expect(hello()).toBe("hello");
});
